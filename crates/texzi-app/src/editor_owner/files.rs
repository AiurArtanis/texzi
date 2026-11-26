use super::*;
use doc::files::{Encoding, FileError, Format, LoadedBytes, encode, save};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) struct FileState {
    format: Format,
    disk: Option<DiskContents>,
    pub(super) keep_dirty_revision: Option<doc::Revision>,
    #[cfg(test)]
    before_publish: Option<BeforePublish>,
}

struct DiskContents {
    bytes: Arc<[u8]>,
    token: doc::StateToken,
}

#[cfg(test)]
type BeforePublish = Box<dyn FnOnce(&doc::Snapshot) + Send>;

const UTF8: Format = Format {
    encoding: Encoding::Utf8,
    bom: false,
};

impl Default for FileState {
    fn default() -> Self {
        Self {
            format: UTF8,
            disk: None,
            keep_dirty_revision: None,
            #[cfg(test)]
            before_publish: None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum SaveError {
    Encoding,
    Other(String),
}

impl NativeView {
    pub(crate) fn has_disk_baseline(&self) -> bool {
        self.owner.files.borrow().disk.is_some()
    }

    pub(crate) fn disk_len(&self) -> u64 {
        self.owner
            .files
            .borrow()
            .disk
            .as_ref()
            .map_or(0, |file| file.bytes.len() as u64)
    }

    pub fn new_file(format: Format, default_eol: doc::Eol) -> Rc<Self> {
        let mut store = doc::Store::new(doc::HistoryBudget::default());
        let snapshot = store
            .open_decoded_with_eol("", format, default_eol)
            .expect("empty file with a supported format");
        let files = FileState {
            format,
            ..FileState::default()
        };
        Self::from_store(store, snapshot, files).expect("empty source view")
    }

    pub fn open_file(path: &Path) -> Result<Rc<Self>, String> {
        let bytes: Arc<[u8]> = std::fs::read(path)
            .map_err(|error| error.to_string())?
            .into();
        let loaded = match LoadedBytes::decode(bytes.clone(), None) {
            Err(FileError::EncodingChoiceRequired) => {
                LoadedBytes::decode(bytes.clone(), Some(Encoding::GbkWebEncodingRs0844)).or_else(
                    |_| {
                        LoadedBytes::decode(bytes.clone(), Some(Encoding::Gb18030WebEncodingRs0844))
                    },
                )
            }
            result => result,
        }
        .map_err(|error| format!("{error:?}"))?;
        let format = loaded.format();
        let mut store = doc::Store::new(doc::HistoryBudget::default());
        let snapshot = store.open_decoded(loaded.text(), format).map_err(err)?;
        let files = FileState {
            format,
            disk: Some(DiskContents {
                bytes,
                token: snapshot.token,
            }),
            ..FileState::default()
        };
        Self::from_store(store, snapshot, files)
    }

    pub(crate) fn file_status(&self) -> Option<(&'static str, &'static str)> {
        self.track();
        let encoding = match self.owner.files.borrow().format {
            Format {
                encoding: Encoding::Utf8,
                bom: true,
            } => "UTF-8 BOM",
            Format {
                encoding: Encoding::Utf8,
                ..
            } => "UTF-8",
            Format {
                encoding: Encoding::Utf16Le,
                ..
            } => "UTF-16 LE",
            Format {
                encoding: Encoding::Utf16Be,
                ..
            } => "UTF-16 BE",
            Format {
                encoding: Encoding::Gbk936 | Encoding::GbkWebEncodingRs0844,
                ..
            } => "GBK",
            Format {
                encoding: Encoding::Gb18030_2022 | Encoding::Gb18030WebEncodingRs0844,
                ..
            } => "GB18030",
            Format {
                encoding: Encoding::Windows1252,
                ..
            } => "Windows-1252",
        };
        let store = self.owner.store.borrow();
        let snapshot = store.snapshot(store.view(self.view).ok()?.doc).ok()?;
        let newline = match snapshot.body.dominant_eol() {
            doc::Eol::Lf => "LF",
            doc::Eol::CrLf => "CRLF",
            doc::Eol::Cr => "CR",
        };
        Some((encoding, newline))
    }

    #[cfg(test)]
    pub(crate) fn before_next_publication(&self, hook: BeforePublish) {
        self.owner.files.borrow_mut().before_publish = Some(hook);
    }

    pub(crate) async fn save_file(
        &self,
        path: PathBuf,
        replace: bool,
        use_utf8: bool,
    ) -> Result<(), SaveError> {
        let snapshot = {
            let mut store = self.owner.store.borrow_mut();
            let id = store.view(self.view).map_err(save_state_error)?.doc;
            store
                .fence(id, doc::FenceReason::Save)
                .map_err(save_state_error)?;
            store.snapshot(id).map_err(save_state_error)?
        };
        let (format, original, unchanged) = {
            let files = self.owner.files.borrow();
            let format = if use_utf8 { UTF8 } else { files.format };
            let original = files.disk.as_ref();
            let unchanged = original
                .filter(|disk| disk.token == snapshot.token && format == files.format)
                .map(|disk| disk.bytes.clone());
            let expected = if replace {
                Some(
                    original
                        .ok_or_else(|| SaveError::Other("缺少已打开文件的保存基准。".into()))?
                        .bytes
                        .clone(),
                )
            } else {
                None
            };
            (format, expected, unchanged)
        };
        let body = snapshot.body.clone();
        #[cfg(test)]
        let (test_snapshot, before_publish) = (
            snapshot.clone(),
            self.owner.files.borrow_mut().before_publish.take(),
        );
        let bytes = blocking(move || -> Result<Arc<[u8]>, SaveError> {
            let bytes = match unchanged {
                Some(bytes) => bytes,
                None => {
                    let utf8 = body.to_utf8();
                    let bytes = if format == UTF8 {
                        utf8
                    } else {
                        let text = std::str::from_utf8(&utf8)
                            .map_err(|error| SaveError::Other(error.to_string()))?;
                        encode(text, format).map_err(|error| match error {
                            FileError::Unrepresentable => SaveError::Encoding,
                            other => SaveError::Other(format!("{other:?}")),
                        })?
                    };
                    Arc::from(bytes)
                }
            };
            #[cfg(test)]
            if let Some(hook) = before_publish {
                hook(&test_snapshot);
            }
            save::write(&path, &bytes, original.as_deref())
                .map_err(|error| SaveError::Other(error.to_string()))?;
            Ok(bytes)
        })
        .await
        .map_err(|error| SaveError::Other(error.to_string()))??;

        let current = {
            let mut store = self.owner.store.borrow_mut();
            let current = store.snapshot(snapshot.doc).map_err(save_state_error)?;
            store
                .record_disk_save(&snapshot)
                .map_err(save_state_error)?;
            current
        };
        {
            let mut files = self.owner.files.borrow_mut();
            files.format = format;
            // Keep the dirty marker if an edit and undo arrived during this write.
            files.keep_dirty_revision =
                (current.revision != snapshot.revision).then_some(current.revision);
            files.disk = Some(DiskContents {
                bytes,
                token: snapshot.token,
            });
        }
        self.notify();
        Ok(())
    }
}

fn save_state_error(error: doc::Error) -> SaveError {
    SaveError::Other(err(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn with_file(name: &str, run: impl FnOnce(&Path, &tokio::runtime::Runtime)) {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("texzi-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let worker = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        run(&dir, &worker);
        assert!(fs::read_dir(&dir).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".texzi-")
        }));
        fs::remove_dir_all(dir).unwrap();
    }

    fn append(doc: &NativeView, text: &str) {
        let before = doc.snapshot().unwrap();
        let end = before.rope.len_bytes();
        let caret = end + text.len();
        doc.edit(
            before.revision,
            before.view_epoch,
            vec![(end..end, text.into())],
            ExternalSelections {
                primary: 0,
                ranges: vec![(caret, caret)],
            },
            ExternalEditKind::Typing,
        )
        .unwrap();
    }

    #[test]
    fn new_file_uses_selected_encoding_and_eol_without_changing_existing_formats() {
        with_file("new-file-format", |dir, worker| {
            let format = Format {
                encoding: Encoding::Utf16Le,
                bom: true,
            };
            let target = dir.join("new.txt");
            let doc = NativeView::new_file(format, doc::Eol::Lf);
            append(&doc, "A\nB");
            worker
                .block_on(doc.save_file(target.clone(), false, false))
                .unwrap();
            assert_eq!(fs::read(target).unwrap(), b"\xff\xfeA\x00\n\x00B\x00");

            let existing = dir.join("existing.txt");
            let original = b"a\r\nb\n";
            fs::write(&existing, original).unwrap();
            let doc = NativeView::open_file(&existing).unwrap();
            worker
                .block_on(doc.save_file(existing.clone(), true, false))
                .unwrap();
            assert_eq!(fs::read(existing).unwrap(), original);
        });
    }

    #[test]
    fn saving_without_edits_preserves_encoding_bom_and_newlines() {
        with_file("roundtrip", |dir, worker| {
            let cases: &[&[u8]] = &[
                b"line\nnext\r\n",
                b"\xef\xbb\xbfline\r\n",
                b"\xff\xfeA\x00\r\x00\n\x00",
                b"\xfe\xff\x00A\x00\n",
                b"\xd6\xd0\xce\xc4\r\n",
                b"\x94\x39\xfc\x36\n",
            ];
            for (index, bytes) in cases.iter().enumerate() {
                let path = dir.join(format!("{index}.txt"));
                fs::write(&path, bytes).unwrap();
                let doc = NativeView::open_file(&path).unwrap();
                worker
                    .block_on(doc.save_file(path.clone(), true, false))
                    .unwrap();
                assert_eq!(&fs::read(path).unwrap(), bytes);
                assert!(!doc.is_dirty().unwrap());
            }
        });
    }

    #[test]
    fn read_only_and_unrepresentable_saves_keep_original_and_dirty() {
        with_file("retry-save", |dir, worker| {
            let original = dir.join("original.txt");
            let copy = dir.join("copy.txt");
            let bytes = b"\xd6\xd0\r\n";
            fs::write(&original, bytes).unwrap();
            let doc = NativeView::open_file(&original).unwrap();
            append(&doc, "😀");
            assert!(matches!(
                worker.block_on(doc.save_file(original.clone(), true, false)),
                Err(SaveError::Encoding)
            ));
            let mut permissions = fs::metadata(&original).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&original, permissions.clone()).unwrap();
            assert!(matches!(
                worker.block_on(doc.save_file(original.clone(), true, true)),
                Err(SaveError::Other(_))
            ));
            permissions.set_readonly(false);
            fs::set_permissions(&original, permissions).unwrap();
            assert_eq!(fs::read(&original).unwrap(), bytes);
            assert!(doc.is_dirty().unwrap());
            worker
                .block_on(doc.save_file(copy.clone(), false, true))
                .unwrap();
            assert_eq!(fs::read_to_string(&copy).unwrap(), "中\r\n😀");
            let copy_doc = NativeView::open_file(&copy).unwrap();
            assert_eq!(copy_doc.text().unwrap(), "中\n😀");
            assert!(!doc.is_dirty().unwrap());

            append(&doc, "!");
            worker
                .block_on(doc.save_file(copy.clone(), true, false))
                .unwrap();
            assert_eq!(fs::read_to_string(&copy).unwrap(), "中\r\n😀!");
            assert!(!doc.is_dirty().unwrap());

            append(&doc, "?");
            fs::write(&copy, b"outside").unwrap();
            assert!(matches!(
                worker.block_on(doc.save_file(copy.clone(), true, false)),
                Err(SaveError::Other(_))
            ));
            assert_eq!(fs::read(copy).unwrap(), b"outside");
            assert!(doc.is_dirty().unwrap());
        });
    }
}
