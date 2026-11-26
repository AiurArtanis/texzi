use super::{Document, Shell};
use crate::{editor_owner::NativeView, settings};
use nsh_base::{
    editor::{EditorApi, ExternalDocument, ExternalSelections},
    rich_editor::view::RichEditorApi,
};
use nsh_core::{desktop::WindowPlacement, geometry::point, prelude::*};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use texzi_document::{
    Body, Eol,
    files::{Encoding, Format},
};

const INTERVAL: Duration = Duration::from_secs(2);
const NEW_WINDOW: &str = "--texzi-new-window";
const RESUME_WINDOW: &str = "--texzi-restore-window=";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct LocalPath(Vec<u16>);

impl LocalPath {
    fn from_path(path: &Path) -> Self {
        Self(path.as_os_str().encode_wide().collect())
    }

    fn to_path(&self) -> PathBuf {
        OsString::from_wide(&self.0).into()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct EditorView {
    rich: bool,
    wrap: Option<bool>,
    tabs: Option<u32>,
    selection: Option<(Vec<(usize, usize)>, usize)>,
    scroll: Option<(f32, f32)>,
}

impl EditorView {
    fn capture(doc: &Document) -> Self {
        let rich = doc.displayed_rich();
        Self {
            rich,
            wrap: doc.wrap.get(),
            tabs: doc.tab_size.get(),
            selection: doc.gate.recovery_selection().map(|s| (s.ranges, s.primary)),
            scroll: scroll_position(doc).or(doc.retained_scroll.get()),
        }
    }

    fn restore_source(&self, source: &NativeView) {
        if let Some((ranges, primary)) = &self.selection
            && let Ok(snapshot) = source.snapshot()
        {
            let _ = source.select(
                snapshot.revision,
                snapshot.view_epoch,
                ExternalSelections {
                    ranges: ranges.clone(),
                    primary: *primary,
                },
            );
        }
    }

    fn restore_view(&self, doc: &Document) {
        doc.wrap.set(self.wrap);
        doc.tab_size
            .set(self.tabs.filter(|width| (1..=8).contains(width)));
        doc.retained_scroll.set(self.scroll);
    }
}

fn scroll_position(doc: &Document) -> Option<(f32, f32)> {
    if doc.displayed_rich() {
        doc.rich_reference
            .api::<RichEditorApi>()
            .map(|editor| (0.0, editor.scroll_top()))
    } else {
        doc.reference.api::<EditorApi>().map(|editor| {
            let offset = editor.scroll_offset();
            (offset.x.0, offset.y.0)
        })
    }
}

pub(super) fn keep_scroll(doc: &Document) {
    if let Some(position) = scroll_position(doc) {
        doc.retained_scroll.set(Some(position));
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Draft {
    text: String,
    #[serde(flatten)]
    format: FilePreference,
    disk_sha256: Option<[u8; 32]>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct FilePreference {
    encoding: String,
    bom: bool,
    eol: String,
}

struct DraftCapture {
    body: Arc<Body>,
    format: Format,
    baseline: Option<Arc<[u8]>>,
}

impl DraftCapture {
    fn materialize(self) -> Draft {
        Draft {
            text: String::from_utf8(self.body.to_utf8())
                .expect("document recovery bytes are UTF-8"),
            format: FilePreference::capture(self.format, self.body.default_eol()),
            disk_sha256: self.baseline.as_deref().map(digest),
        }
    }
}

impl FilePreference {
    fn capture(format: Format, eol: Eol) -> Self {
        let encoding = match format.encoding {
            Encoding::Utf8 => "utf-8",
            Encoding::Utf16Le => "utf-16le",
            Encoding::Utf16Be => "utf-16be",
            Encoding::Gbk936 => "gbk-936",
            Encoding::GbkWebEncodingRs0844 => "gbk",
            Encoding::Windows1252 => "windows-1252",
            Encoding::Gb18030_2022 => "gb18030-2022",
            Encoding::Gb18030WebEncodingRs0844 => "gb18030",
        };
        let eol = match eol {
            Eol::Lf => "LF",
            Eol::CrLf => "CRLF",
            Eol::Cr => "CR",
        };
        Self {
            encoding: encoding.into(),
            bom: format.bom,
            eol: eol.into(),
        }
    }

    fn restore(&self) -> (Format, Eol) {
        let (mut format, mut eol) = settings::new_document_format(&self.encoding, &self.eol);
        format.bom = self.bom;
        if self.encoding == "gbk-936" {
            format.encoding = Encoding::Gbk936;
        } else if self.encoding == "gb18030-2022" {
            format.encoding = Encoding::Gb18030_2022;
        }
        if self.eol == "CR" {
            eol = Eol::Cr;
        }
        (format, eol)
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Tab {
    id: u64,
    path: Option<LocalPath>,
    view: EditorView,
    draft: Option<Draft>,
    blank: Option<FilePreference>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Split {
    id: u64,
    down: bool,
    active: bool,
    view: EditorView,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct WindowRecord {
    version: u32,
    folder: Option<LocalPath>,
    sidebar: bool,
    sidebar_width: f32,
    placement: Option<WindowPlacement>,
    tabs: Vec<Tab>,
    active: u64,
    split: Option<Split>,
}

struct Disk {
    path: PathBuf,
    _lease: File,
    committed: Mutex<u64>,
}

impl Disk {
    fn claim(directory: &Path, id: &str) -> io::Result<Option<Arc<Self>>> {
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(format!("{id}.lock")))?;
        if lease.try_lock().is_err() {
            return Ok(None);
        }
        Ok(Some(Arc::new(Self {
            path: directory.join(format!("{id}.json")),
            _lease: lease,
            committed: Mutex::new(0),
        })))
    }

    fn read(&self) -> io::Result<WindowRecord> {
        let record: WindowRecord = serde_json::from_reader(File::open(&self.path)?)?;
        if record.version != 1
            || record
                .tabs
                .iter()
                .any(|tab| tab.id == 0 || tab.id == u64::MAX)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "不支持此恢复记录版本",
            ));
        }
        Ok(record)
    }

    fn update(
        &self,
        sequence: u64,
        make_record: impl FnOnce() -> io::Result<Option<WindowRecord>>,
    ) -> io::Result<()> {
        let mut committed = self
            .committed
            .lock()
            .map_err(|_| io::Error::other("恢复写入锁不可用"))?;
        if sequence <= *committed {
            return Ok(());
        }
        let Some(record) = make_record()? else {
            *committed = sequence;
            return Ok(());
        };
        let temporary = self.path.with_extension("tmp");
        let result = (|| {
            let mut file = File::create(&temporary)?;
            serde_json::to_writer(&mut file, &record)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        *committed = sequence;
        Ok(())
    }
}

type Signature = (WindowRecord, Vec<(u64, u64)>);

pub(super) struct Session {
    disk: Arc<Disk>,
    sequence: Cell<u64>,
    last: RefCell<Option<Signature>>,
    writing: Cell<bool>,
}

fn enabled(shell: &Shell) -> bool {
    shell.settings.effective(settings::RESTORE, None).0 == true
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 68 && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

pub(super) fn spawn_blank_window() -> io::Result<()> {
    Command::new(std::env::current_exe()?)
        .arg(NEW_WINDOW)
        .spawn()
        .map(|_| ())
}

pub(super) fn start(shell: &Rc<Shell>) -> io::Result<()> {
    if cfg!(test) {
        return Ok(());
    }
    let Some(base) = std::env::var_os("LOCALAPPDATA") else {
        return Ok(());
    };
    let directory = PathBuf::from(base).join("texzi").join("recovery");
    fs::create_dir_all(&directory)?;
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let targeted = args
        .iter()
        .find_map(|arg| arg.to_str()?.strip_prefix(RESUME_WINDOW))
        .filter(|id| valid_id(id));
    let mut queues = Vec::new();
    if enabled(shell) && !args.iter().any(|arg| arg == NEW_WINDOW) {
        if let Some(id) = targeted {
            queues.push(id.to_owned());
        } else {
            let mut files = fs::read_dir(&directory)?
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let path = entry.path();
                    let id = path.file_stem()?.to_str()?.to_owned();
                    (path.extension()?.eq_ignore_ascii_case("json") && valid_id(&id)).then(|| {
                        (
                            entry
                                .metadata()
                                .and_then(|metadata| metadata.modified())
                                .unwrap_or(UNIX_EPOCH),
                            id,
                        )
                    })
                })
                .collect::<Vec<_>>();
            files.sort_unstable_by(|a, b| b.cmp(a));
            queues.extend(files.into_iter().map(|(_, id)| id));
        }
    }

    let mut chosen = None;
    let mut other_windows = Vec::new();
    for id in queues {
        let Some(disk) = Disk::claim(&directory, &id)? else {
            continue;
        };
        match disk.read() {
            Ok(record) if !record.tabs.is_empty() => {
                if chosen.is_none() {
                    chosen = Some((disk, record));
                } else {
                    other_windows.push(id);
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                log::warn!("无法读取恢复记录 {id}：{error}");
            }
            Err(_) => {}
        }
    }
    let disk = if let Some((disk, record)) = chosen {
        restore(shell, record);
        disk
    } else {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let id = format!("{}-{now}", std::process::id());
        Disk::claim(&directory, &id)?.ok_or_else(|| io::Error::other("无法锁定本窗口的恢复记录"))?
    };
    *shell.session.borrow_mut() = Some(Rc::new(Session {
        disk,
        sequence: Cell::new(0),
        last: RefCell::new(None),
        writing: Cell::new(false),
    }));
    for id in other_windows {
        if let Err(error) = Command::new(std::env::current_exe()?)
            .arg(format!("{RESUME_WINDOW}{id}"))
            .spawn()
        {
            log::warn!("无法恢复窗口 {id}：{error}");
        }
    }
    periodic(shell);
    Ok(())
}

fn periodic(shell: &Rc<Shell>) {
    let owner = shell.owner;
    let weak = Rc::downgrade(shell);
    owner.run(|| {
        set_timeout(INTERVAL, move || {
            if let Some(shell) = weak.upgrade() {
                checkpoint(&shell, false);
                periodic(&shell);
            }
        })
    });
}

fn collect(shell: &Shell, drafts: bool) -> (Signature, Vec<(u64, DraftCapture)>) {
    let mut revisions = Vec::new();
    let mut bodies = Vec::new();
    let tabs = shell.docs.with(|docs| {
        docs.iter()
            .filter_map(|doc| {
                let path = doc.path.get().as_deref().map(LocalPath::from_path);
                if !doc.editable_body_ready() {
                    return Some(Tab {
                        id: doc.id,
                        path,
                        view: EditorView {
                            rich: false,
                            wrap: None,
                            tabs: None,
                            selection: None,
                            scroll: None,
                        },
                        draft: None,
                        blank: None,
                    });
                }
                let revision = doc.source.dirty_revision();
                let view = EditorView::capture(doc);
                let mut blank = None;
                if drafts
                    && let Some(revision) = revision
                    && let Some(body) = doc.source.current_body()
                {
                    let (format, baseline) = doc.source.recovery_format();
                    revisions.push((doc.id, revision));
                    bodies.push((
                        doc.id,
                        DraftCapture {
                            body,
                            format,
                            baseline,
                        },
                    ));
                } else if revision.is_some() {
                    return None;
                } else if path.is_none() {
                    if view.wrap.is_none() && view.tabs.is_none() {
                        return None;
                    }
                    let (format, _) = doc.source.recovery_format();
                    let eol = doc.source.current_body()?.default_eol();
                    blank = Some(FilePreference::capture(format, eol));
                }
                Some(Tab {
                    id: doc.id,
                    path,
                    view,
                    draft: None,
                    blank,
                })
            })
            .collect()
    });
    let record = WindowRecord {
        version: 1,
        folder: shell.root.get().as_deref().map(LocalPath::from_path),
        sidebar: shell.sidebar.get(),
        sidebar_width: shell.sidebar_width.get(),
        placement: shell.window.state().map(|state| state.placement),
        tabs,
        active: shell.active.get(),
        split: shell.split.get().map(|doc| Split {
            id: doc.id,
            down: shell.split_down.get(),
            active: shell.second_active.get(),
            view: EditorView::capture(&doc),
        }),
    };
    ((record, revisions), bodies)
}

pub(super) fn checkpoint(shell: &Rc<Shell>, force: bool) {
    let Some(session) = shell.session.borrow().clone() else {
        return;
    };
    if !enabled(shell) || !force && session.writing.get() {
        return;
    }
    let (signature, bodies) = collect(shell, true);
    if !force && session.last.borrow().as_ref() == Some(&signature) {
        return;
    }
    let record = signature.0.clone();
    session.sequence.set(session.sequence.get() + 1);
    let sequence = session.sequence.get();
    let disk = session.disk.clone();
    session.writing.set(true);
    let owner = shell.owner;
    let shell = Rc::downgrade(shell);
    owner.run(|| {
        spawn(async move {
            let result = blocking(move || {
                let mut record = record;
                for (id, body) in bodies {
                    if let Some(tab) = record.tabs.iter_mut().find(|tab| tab.id == id) {
                        tab.draft = Some(body.materialize());
                    }
                }
                disk.update(sequence, || {
                    Ok((!record.tabs.is_empty() || disk.path.is_file()).then_some(record))
                })
            })
            .await;
            if session.sequence.get() != sequence {
                return;
            }
            session.writing.set(false);
            match result {
                Ok(Ok(())) => *session.last.borrow_mut() = Some(signature),
                Ok(Err(error)) => {
                    if let Some(shell) = shell.upgrade() {
                        shell.notice.set(format!("未能写入恢复快照 · {error}"));
                    }
                }
                Err(error) => log::warn!("恢复写入任务退出：{error}"),
            }
        })
    });
}

pub(super) fn saved(shell: &Rc<Shell>, doc: &Document) {
    if enabled(shell) {
        checkpoint(shell, true);
        return;
    }
    if doc.source.is_dirty().unwrap_or(true) {
        return;
    }
    let Some(session) = shell.session.borrow().clone() else {
        return;
    };
    session.sequence.set(session.sequence.get() + 1);
    let path = doc.path.get().as_deref().map(LocalPath::from_path);
    let result = session.disk.update(session.sequence.get(), || {
        let mut record = match session.disk.read() {
            Ok(record) => record,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if let Some(tab) = record.tabs.iter_mut().find(|tab| tab.id == doc.id) {
            tab.draft = None;
            tab.path = path;
        }
        Ok(Some(record))
    });
    session.writing.set(false);
    *session.last.borrow_mut() = None;
    if let Err(error) = result {
        log::warn!("保存后未能更新恢复记录：{error}");
    }
}

pub(super) fn remove_discarded(shell: &Shell, id: u64) -> io::Result<()> {
    let Some(session) = shell.session.borrow().clone() else {
        return Ok(());
    };
    session.sequence.set(session.sequence.get() + 1);
    session.writing.set(false);
    *session.last.borrow_mut() = None;
    session.disk.update(session.sequence.get(), || {
        let mut record = match session.disk.read() {
            Ok(record) => record,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        record.tabs.retain(|tab| tab.id != id);
        Ok(Some(record))
    })
}

pub(super) fn finish(shell: &Shell, discard_dirty: bool) -> io::Result<()> {
    let Some(session) = shell.session.borrow().clone() else {
        return Ok(());
    };
    session.sequence.set(session.sequence.get() + 1);
    session.writing.set(false);
    *session.last.borrow_mut() = None;
    if enabled(shell) {
        let record = collect(shell, false).0.0;
        return session.disk.update(session.sequence.get(), || {
            Ok((!record.tabs.is_empty() || session.disk.path.is_file()).then_some(record))
        });
    }
    if discard_dirty {
        let discarded: Vec<_> = shell.docs.with(|docs| {
            docs.iter()
                .filter(|doc| doc.source.is_dirty().unwrap_or(true))
                .map(|doc| doc.id)
                .collect()
        });
        session.disk.update(session.sequence.get(), || {
            let mut record = match session.disk.read() {
                Ok(record) => record,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            };
            record.tabs.retain(|tab| !discarded.contains(&tab.id));
            Ok(Some(record))
        })?;
    }
    Ok(())
}

fn restore(shell: &Rc<Shell>, record: WindowRecord) {
    shell.docs.set(Vec::new());
    if let Some(path) = record
        .folder
        .as_ref()
        .map(LocalPath::to_path)
        .filter(|path| path.is_dir())
    {
        shell.show_folder(path);
    }
    shell.sidebar.set(record.sidebar);
    shell
        .sidebar_width
        .set(record.sidebar_width.clamp(120.0, 520.0));
    if let Some(placement) = record.placement {
        shell.window.set_placement(placement);
    }
    let mut unavailable = 0;
    let max_id = record.tabs.iter().map(|tab| tab.id).max().unwrap_or(0);
    for tab in record.tabs {
        if !restore_tab(shell, tab) {
            unavailable += 1;
        }
    }
    shell.next.set(max_id.saturating_add(1));
    if shell.docs.with(|docs| docs.is_empty()) {
        shell.add_blank();
    }
    if let Some(split) = record.split
        && let Some(primary) = shell
            .docs
            .with(|docs| docs.iter().find(|doc| doc.id == split.id).cloned())
    {
        let rich = primary.displayed_rich();
        shell.activate_primary(primary.id);
        shell.split_editor(split.down);
        if rich {
            primary.gate.to_rich();
        }
        if let Some(second) = shell.split.get() {
            split.view.restore_source(&second.source);
            split.view.restore_view(&second);
            if split.view.rich && second.source.disk_len() <= settings::MARKDOWN_SOURCE_ONLY_BYTES {
                second.gate.to_rich();
            }
        }
        shell.second_active.set(split.active);
    }
    if shell
        .docs
        .with(|docs| docs.iter().any(|doc| doc.id == record.active))
    {
        shell.active.set(record.active);
    }
    if unavailable > 0 {
        shell.notice.set(format!(
            "会话已恢复 · {unavailable} 个文件无法重新关联，未覆盖现有文件"
        ));
    }
}

fn restore_tab(shell: &Rc<Shell>, tab: Tab) -> bool {
    let mut path = tab.path.as_ref().map(LocalPath::to_path);
    if let Some(path) = &mut path {
        if let Ok(canonical) = path.canonicalize() {
            *path = canonical;
        }
    }
    let named = path.is_some();
    shell.next.set(tab.id);
    if let Some(draft) = tab.draft {
        let baseline = path
            .as_deref()
            .and_then(|path| shell.load_recovery_target(path).ok())
            .and_then(|source| source.recovery_format().1)
            .filter(|bytes| {
                draft
                    .disk_sha256
                    .is_some_and(|expected| digest(bytes) == expected)
            });
        if baseline.is_none() {
            path = None;
        }
        let (format, eol) = draft.format.restore();
        let Ok(source) = NativeView::recover_file(&draft.text, format, eol, baseline) else {
            return false;
        };
        tab.view.restore_source(&source);
        let detached = named && path.is_none();
        shell.add_with_mode(source, path, Some(tab.view.rich && !detached));
        if let Some(doc) = shell.primary() {
            tab.view.restore_view(&doc);
            doc.auto_save.ignore_restored(doc.source.dirty_revision());
        }
        !detached
    } else if let Some(path) = path {
        let Ok(source) = shell.load_recovery_target(&path) else {
            return false;
        };
        tab.view.restore_source(&source);
        shell.add_with_mode(source, Some(path), Some(tab.view.rich));
        if let Some(doc) = shell.primary() {
            tab.view.restore_view(&doc);
        }
        true
    } else if let Some(blank) = tab.blank {
        let (format, eol) = blank.restore();
        let source = shell.owner.run(|| NativeView::new_file(format, eol));
        tab.view.restore_source(&source);
        shell.add_with_mode(source, None, Some(false));
        if let Some(doc) = shell.primary() {
            tab.view.restore_view(&doc);
        }
        true
    } else {
        true
    }
}

pub(super) fn apply_scroll(doc: &Document, rich: bool) {
    let Some((x, y)) = doc.retained_scroll.get() else {
        return;
    };
    if rich {
        if let Some(editor) = doc.rich_reference.api::<RichEditorApi>() {
            editor.set_scroll_top(y);
            doc.retained_scroll.set(None);
        }
    } else if let Some(editor) = doc.reference.api::<EditorApi>() {
        editor.set_scroll_offset(point(px(x), px(y)));
        doc.retained_scroll.set(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_core::{ApproxTextEngine, Core, Size, element::Root};

    thread_local! {
        static RECOVERY_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    fn recovery_tab(path: &Path, bytes: &[u8]) -> Tab {
        Tab {
            id: 4,
            path: Some(LocalPath::from_path(path)),
            view: EditorView {
                rich: false,
                wrap: Some(true),
                tabs: Some(8),
                selection: Some((vec![(2, 6)], 0)),
                scroll: Some((0.0, 20.0)),
            },
            draft: Some(Draft {
                text: "private draft\r\nsecond\n".into(),
                format: FilePreference {
                    encoding: "utf-8".into(),
                    bom: false,
                    eol: "LF".into(),
                },
                disk_sha256: Some(digest(bytes)),
            }),
            blank: None,
        }
    }

    fn record(tab: Tab) -> WindowRecord {
        WindowRecord {
            version: 1,
            folder: None,
            sidebar: true,
            sidebar_width: 240.0,
            placement: None,
            active: tab.id,
            tabs: vec![tab],
            split: None,
        }
    }

    #[component]
    fn RecoveryFixture() -> impl IntoElement {
        let directory = RECOVERY_PATH.with(|slot| slot.borrow_mut().take()).unwrap();
        let path = directory.join("named.txt");
        let disk = Disk::claim(&directory, "410").unwrap().unwrap();
        let initial = recovery_tab(&path, b"published");
        disk.update(1, || Ok(Some(record(initial.clone()))))
            .unwrap();
        let restored = Shell::new();
        restore(&restored, disk.read().unwrap());
        let tab = restored.current().unwrap();
        assert_eq!(tab.id, 4);
        assert_eq!(
            tab.path.get().as_deref(),
            Some(path.canonicalize().unwrap().as_path())
        );
        assert_eq!(
            tab.source.snapshot().unwrap().selections.ranges,
            vec![(2, 6)]
        );
        assert_eq!(restored.tab_size(&tab), 8);
        assert!(tab.source.is_dirty().unwrap());
        assert_eq!(
            tab.source.current_body().unwrap().to_utf8(),
            b"private draft\r\nsecond\n"
        );
        assert_eq!(fs::read(&path).unwrap(), b"published");

        let blank_format = Format {
            encoding: Encoding::Utf16Be,
            bom: true,
        };
        restored.add(NativeView::new_file(blank_format, Eol::Lf), None);
        let blank = restored.current().unwrap();
        blank.tab_size.set(Some(8));
        let ((metadata, _), _) = collect(&restored, false);
        let blank_record = metadata
            .tabs
            .into_iter()
            .find(|entry| entry.id == blank.id)
            .unwrap();
        assert!(blank_record.draft.is_none() && blank_record.blank.is_some());
        let mut with_blank = disk.read().unwrap();
        with_blank.active = blank.id;
        with_blank.tabs.push(blank_record);
        restore(&restored, with_blank);
        let blank = restored.current().unwrap();
        assert!(blank.path.get().is_none() && !blank.source.is_dirty().unwrap());
        assert_eq!(restored.tab_size(&blank), 8);
        assert_eq!(blank.source.recovery_format().0, blank_format);
        assert_eq!(blank.source.current_body().unwrap().default_eol(), Eol::Lf);

        fs::write(&path, b"changed elsewhere").unwrap();
        restore(&restored, disk.read().unwrap());
        let tab = restored.current().unwrap();
        assert!(tab.path.get().is_none());
        assert!(tab.source.is_dirty().unwrap());
        assert_eq!(tab.source.text().unwrap(), "private draft\nsecond\n");
        assert_eq!(fs::read(&path).unwrap(), b"changed elsewhere");

        tab.source.set_composing(true);
        restored.add_blank();
        let independent = restored.current().unwrap();
        let snapshot = independent.source.snapshot().unwrap();
        independent
            .source
            .edit(
                snapshot.revision,
                snapshot.view_epoch,
                vec![(0..0, "other tab".into())],
                ExternalSelections {
                    ranges: vec![(9, 9)],
                    primary: 0,
                },
                nsh_base::editor::ExternalEditKind::Typing,
            )
            .unwrap();
        let (_, captured) = collect(&restored, true);
        assert_eq!(captured.len(), 2);
        assert!(
            captured.iter().any(
                |(id, capture)| *id == independent.id && capture.body.to_utf8() == b"other tab"
            )
        );
        assert!(
            captured.iter().any(|(id, capture)| *id == tab.id
                && capture.body.to_utf8() == b"private draft\r\nsecond\n")
        );
        tab.source.set_composing(false);

        *restored.session.borrow_mut() = Some(Rc::new(Session {
            disk: disk.clone(),
            sequence: Cell::new(1),
            last: RefCell::new(None),
            writing: Cell::new(false),
        }));
        assert!(
            restored
                .settings
                .apply(settings::RESTORE, None, Some(false.into()))
        );
        finish(&restored, false).unwrap();
        assert_eq!(disk.read().unwrap().tabs.len(), 1);
        let recovery = restored.session.borrow().clone().unwrap();
        recovery.writing.set(true);
        let obstructed = disk.path.with_extension("tmp");
        fs::create_dir(&obstructed).unwrap();
        assert!(remove_discarded(&restored, 4).is_err());
        assert!(!recovery.writing.get());
        assert!(tab.source.is_dirty().unwrap());
        assert_eq!(disk.read().unwrap().tabs.len(), 1);
        fs::remove_dir(&obstructed).unwrap();
        remove_discarded(&restored, 4).unwrap();
        assert!(disk.read().unwrap().tabs.is_empty());
        disk.update(1, || Ok(Some(record(initial)))).unwrap();
        assert!(
            disk.read().unwrap().tabs.is_empty(),
            "a stale checkpoint must not restore a discarded tab"
        );

        *restored.session.borrow_mut() = None;
        drop(recovery);
        drop(disk);
        fs::remove_file(path).unwrap();
        fs::remove_file(directory.join("410.json")).unwrap();
        fs::remove_file(directory.join("410.lock")).unwrap();
        fs::remove_dir(directory).unwrap();
        text("")
    }

    #[test]
    fn recovery_restores_a_dirty_draft_and_never_overwrites_an_external_edit() {
        let directory =
            std::env::temp_dir().join(format!("texzi-recovery-test-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("named.txt"), b"published").unwrap();
        RECOVERY_PATH.with(|slot| *slot.borrow_mut() = Some(directory));
        let mut core = Core::new(
            ApproxTextEngine,
            Size {
                width: px(800.0),
                height: px(600.0),
            },
        );
        core.mount_root(&Root::new::<RecoveryFixture>(|| ()));
        core.run_component_tasks();
        let _ = core.commit();
    }
}
