//! T07 codecs, captured payload and Windows save components. UI integration pending.
use crate::{DocId, SaveCapture, Snapshot, Store, TargetKey};
use std::{path::Path, sync::Arc};
pub mod staging;
#[cfg(windows)]
pub mod windows_identity;
#[cfg(windows)]
pub mod windows_save;
#[cfg(windows)]
pub mod windows_security;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Gbk936,
    GbkWebEncodingRs0844,
    Windows1252,
    /// Strict standard: disabled until the mapping differences are adapted/tested.
    Gb18030_2022,
    /// Explicit alternative; never silently substituted for the strict standard.
    Gb18030WebEncodingRs0844,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub encoding: Encoding,
    pub bom: bool,
}
#[derive(Debug)]
pub enum FileError {
    Io(std::io::Error),
    InvalidBytes,
    BomMismatch,
    EncodingChoiceRequired,
    UnsupportedStandard,
    Unrepresentable,
    UnsupportedBom,
    NeedsDecodedImportApi,
    WrongDocument,
    ProfileChangeNeedsCoordination,
    ExternalChangeDuringRead,
    UnsupportedFileKind,
    NeedsLegacyByteMap,
    UnverifiedDiskBinding,
    StaleDiskBinding,
    Document(crate::Error),
}
impl From<crate::Error> for FileError {
    fn from(e: crate::Error) -> Self {
        Self::Document(e)
    }
}

pub struct LoadedBytes {
    original: Arc<[u8]>,
    text: String,
    format: Format,
}
impl LoadedBytes {
    /// Reads bytes only. Filesystem identity/reparse resolution is not claimed.
    pub fn read_only(path: &Path, explicit: Option<Encoding>) -> Result<Self, FileError> {
        use std::io::Read;
        let mut file = std::fs::File::open(path).map_err(FileError::Io)?;
        let before = file.metadata().map_err(FileError::Io)?;
        if !before.is_file() {
            return Err(FileError::UnsupportedFileKind);
        }
        let modified = before.modified().map_err(FileError::Io)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(FileError::Io)?;
        let after = file.metadata().map_err(FileError::Io)?;
        if before.len() != after.len()
            || after.len() != bytes.len() as u64
            || after.modified().map_err(FileError::Io)? != modified
        {
            return Err(FileError::ExternalChangeDuringRead);
        }
        Self::decode(bytes.into(), explicit)
    }
    pub fn decode(original: Arc<[u8]>, explicit: Option<Encoding>) -> Result<Self, FileError> {
        let signature = if !matches!(explicit, Some(Encoding::Utf16Le | Encoding::Utf16Be))
            && (original.starts_with(&[0, 0, 0xfe, 0xff])
                || original.starts_with(&[0xff, 0xfe, 0, 0]))
        {
            return Err(FileError::UnsupportedBom);
        } else if original.starts_with(&[0xef, 0xbb, 0xbf]) {
            Some((Encoding::Utf8, 3))
        } else if original.starts_with(&[0xff, 0xfe]) {
            Some((Encoding::Utf16Le, 2))
        } else if original.starts_with(&[0xfe, 0xff]) {
            Some((Encoding::Utf16Be, 2))
        } else {
            None
        };
        if let (Some(wanted), Some((detected, _))) = (explicit, signature) {
            if wanted != detected {
                return Err(FileError::BomMismatch);
            }
        }
        let encoding = explicit
            .or(signature.map(|v| v.0))
            .unwrap_or(Encoding::Utf8);
        let data = &original[signature.map_or(0, |v| v.1)..];
        let text = decode(data, encoding).map_err(|e| {
            if explicit.is_none() && signature.is_none() && matches!(e, FileError::InvalidBytes) {
                FileError::EncodingChoiceRequired
            } else {
                e
            }
        })?;
        Ok(Self {
            original,
            text,
            format: Format {
                encoding,
                bom: signature.is_some(),
            },
        })
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    /// An explicit BOM policy disambiguates a signature from literal FEFF text.
    pub fn decode_exact(original: Arc<[u8]>, format: Format) -> Result<Self, FileError> {
        let prefix: &[u8] = if format.bom {
            match format.encoding {
                Encoding::Utf8 => &[0xef, 0xbb, 0xbf],
                Encoding::Utf16Le => &[0xff, 0xfe],
                Encoding::Utf16Be => &[0xfe, 0xff],
                _ => return Err(FileError::UnsupportedBom),
            }
        } else {
            &[]
        };
        if !original.starts_with(prefix) {
            return Err(FileError::BomMismatch);
        }
        let text = decode(&original[prefix.len()..], format.encoding)?;
        Ok(Self {
            original,
            text,
            format,
        })
    }
    pub fn format(&self) -> Format {
        self.format
    }
    pub fn original(&self) -> &[u8] {
        &self.original
    }
    /// No Store mutation happens until decoding and the import precondition pass.
    pub fn attach(self, store: &mut Store) -> Result<(Snapshot, FileBinding), FileError> {
        // File identity is not yet resolved: even a real read is not a verified
        // filesystem binding. Import decoded content without claiming disk identity.
        let snapshot = store.open_decoded(&self.text, self.format)?;
        let binding = FileBinding {
            imported: snapshot.clone(),
            original: self.original,
            format: self.format,
            #[cfg(windows)]
            disk: None,
        };
        Ok((snapshot, binding))
    }
}

pub struct FileBinding {
    imported: Snapshot,
    original: Arc<[u8]>,
    format: Format,
    #[cfg(windows)]
    disk: Option<DiskBinding>,
}
#[cfg(windows)]
struct DiskBinding {
    anchor: windows_identity::OpenAnchor,
    version: windows_identity::FileVersion,
    path: std::path::PathBuf,
    epoch: u64,
}
pub struct PreparedSave {
    capture: SaveCapture,
    bytes: Arc<[u8]>,
    format: Format,
}
impl PreparedSave {
    pub fn capture(&self) -> &SaveCapture {
        &self.capture
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn format(&self) -> Format {
        self.format
    }
}
impl FileBinding {
    #[cfg(windows)]
    fn checked_disk_binding(&self, store: &Store) -> Result<&DiskBinding, FileError> {
        let disk = self.disk.as_ref().ok_or(FileError::UnverifiedDiskBinding)?;
        store.snapshot(self.doc())?; // Validate before indexing owner metadata.
        let metadata = store.observer_metadata(self.doc());
        if !metadata.baseline.backed_by_disk
            || metadata.baseline.token != self.imported.token
            || metadata.attention.binding_epoch != disk.epoch
            || metadata.attention.target.as_ref() != Some(&disk.version.identity.target_key())
        {
            return Err(FileError::StaleDiskBinding);
        }
        Ok(disk)
    }
    pub fn doc(&self) -> DocId {
        self.imported.doc
    }
    pub fn format(&self) -> Format {
        self.format
    }
    pub fn encode_snapshot(&self, s: &Snapshot, format: Format) -> Result<Arc<[u8]>, FileError> {
        if s.doc != self.imported.doc {
            return Err(FileError::WrongDocument);
        }
        if s.body.profile.utf8_bom || s.body.profile.file_format != Some(format) {
            return Err(FileError::ProfileChangeNeedsCoordination);
        }
        // Token plus exact immutable body identity, not revision or text equality.
        if format == self.format
            && s.token == self.imported.token
            && Arc::ptr_eq(&s.body, &self.imported.body)
        {
            return Ok(self.original.clone());
        }
        if format == self.format
            && !matches!(
                format.encoding,
                Encoding::Utf8 | Encoding::Utf16Le | Encoding::Utf16Be
            )
        {
            let imported = self.imported.body.to_utf8();
            let text = std::str::from_utf8(&imported).map_err(|_| FileError::InvalidBytes)?;
            if encode(text, self.format)?.as_slice() != self.original.as_ref() {
                // Without an exact legacy byte map, reencoding could rewrite
                // untouched aliases. Preserve the original and reject the save.
                return Err(FileError::NeedsLegacyByteMap);
            }
        }
        let utf8 = s.body.to_utf8();
        let text = std::str::from_utf8(&utf8).map_err(|_| FileError::InvalidBytes)?;
        encode(text, format).map(Arc::from)
    }
    /// Encode before reserving the save. &mut Store keeps capture/encoding serial.
    /// The returned bytes always belong to the captured token, never a later edit.
    pub fn prepare(
        &self,
        store: &mut Store,
        target: TargetKey,
        guard: String,
        format: Format,
    ) -> Result<PreparedSave, FileError> {
        self.prepare_with_scope(store, target, guard, format, None)
    }
    pub(crate) fn prepare_with_scope(
        &self,
        store: &mut Store,
        target: TargetKey,
        guard: String,
        format: Format,
        scope: Option<crate::SaveScope>,
    ) -> Result<PreparedSave, FileError> {
        let snapshot = store.snapshot(self.doc())?;
        let bytes = self.encode_snapshot(&snapshot, format)?;
        let capture = match scope {
            Some(scope) => store.begin_scoped_save(self.doc(), target, guard, scope)?,
            None => store.begin_save(self.doc(), target, guard)?,
        };
        Ok(PreparedSave {
            capture,
            bytes,
            format,
        })
    }
}

fn legacy(e: Encoding) -> Result<&'static encoding_rs::Encoding, FileError> {
    match e {
        Encoding::Gbk936 => Err(FileError::UnsupportedStandard),
        Encoding::GbkWebEncodingRs0844 => Ok(encoding_rs::GBK),
        Encoding::Windows1252 => Ok(encoding_rs::WINDOWS_1252),
        Encoding::Gb18030WebEncodingRs0844 => Ok(encoding_rs::GB18030),
        Encoding::Gb18030_2022 => Err(FileError::UnsupportedStandard),
        _ => Err(FileError::InvalidBytes),
    }
}
fn gbk_shape(data: &[u8]) -> bool {
    let mut at = 0;
    while at < data.len() {
        let b = data[at];
        at += 1;
        if b <= 0x80 {
            continue;
        }
        if !(0x81..=0xfe).contains(&b) || at == data.len() {
            return false;
        }
        let trail = data[at];
        at += 1;
        if !(0x40..=0xfe).contains(&trail) || trail == 0x7f {
            return false;
        }
    }
    true
}
fn decode(data: &[u8], e: Encoding) -> Result<String, FileError> {
    match e {
        Encoding::Utf8 => std::str::from_utf8(data)
            .map(str::to_owned)
            .map_err(|_| FileError::InvalidBytes),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            if data.len() % 2 != 0 {
                return Err(FileError::InvalidBytes);
            }
            let units = data.chunks_exact(2).map(|b| {
                if e == Encoding::Utf16Le {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            });
            char::decode_utf16(units)
                .collect::<Result<String, _>>()
                .map_err(|_| FileError::InvalidBytes)
        }
        _ => {
            if e == Encoding::GbkWebEncodingRs0844 && !gbk_shape(data) {
                return Err(FileError::InvalidBytes);
            }
            // Strict 1252 policy: undefined code points require an explicit choice.
            if e == Encoding::Windows1252
                && data
                    .iter()
                    .any(|b| [0x81, 0x8d, 0x8f, 0x90, 0x9d].contains(b))
            {
                return Err(FileError::InvalidBytes);
            }
            legacy(e)?
                .decode_without_bom_handling_and_without_replacement(data)
                .map(|s| s.into_owned())
                .ok_or(FileError::InvalidBytes)
        }
    }
}
pub fn encode(text: &str, f: Format) -> Result<Vec<u8>, FileError> {
    let mut out = Vec::new();
    if f.bom {
        out.extend_from_slice(match f.encoding {
            Encoding::Utf8 => &[0xef, 0xbb, 0xbf],
            Encoding::Utf16Le => &[0xff, 0xfe],
            Encoding::Utf16Be => &[0xfe, 0xff],
            _ => return Err(FileError::UnsupportedBom),
        });
    }
    match f.encoding {
        Encoding::Utf8 => out.extend_from_slice(text.as_bytes()),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            for unit in text.encode_utf16() {
                let bytes = if f.encoding == Encoding::Utf16Le {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                };
                out.extend_from_slice(&bytes);
            }
        }
        _ => {
            let (bytes, _, had_errors) = legacy(f.encoding)?.encode(text);
            if had_errors {
                return Err(FileError::Unrepresentable);
            }
            // Includes GBK byte-shape and undefined-1252 checks; never emit lossy output.
            if decode(&bytes, f.encoding)? != text {
                return Err(FileError::Unrepresentable);
            }
            out.extend_from_slice(&bytes);
        }
    }
    Ok(out)
}
