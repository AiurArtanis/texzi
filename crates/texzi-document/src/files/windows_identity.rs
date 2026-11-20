//! Handle-based Windows identity observation, not an atomic replacement API.
//! Keep the directory handle alive; no delete sharing pins this parent.
#[forbid(unsafe_code)]
mod existing_read;
mod open;
use super::windows_security::SecurityDescriptor;
use crate::TargetKey;
pub(crate) use open::OpenAnchor;
pub use open::{AttachFailure, AttachIssue, OpenFailure, OpenIssue, OpenOutcome, OpenedExisting};
use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io::{self, Read, Seek},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};
use windows_sys::Win32::Storage::FileSystem::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    pub volume: u64,
    pub id: [u8; 16],
}
impl FileIdentity {
    pub(super) fn target_key(self) -> TargetKey {
        let hex: String = self.id.iter().map(|b| format!("{b:02x}")).collect();
        TargetKey(format!("windows-file:{:016x}:{hex}", self.volume))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileVersion {
    pub security: SecurityDescriptor,
    pub identity: FileIdentity,
    pub modified: SystemTime,
    pub bytes: Arc<[u8]>,
}
#[derive(Debug)]
pub enum IdentityError {
    Io(io::Error),
    InvalidPath,
    ReparsePoint,
    WrongKind,
    Changed,
    UnsupportedVolumePath,
}
impl From<io::Error> for IdentityError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
pub struct ObservedTarget {
    parent: File,
    parent_security: SecurityDescriptor,
    parent_identity: FileIdentity,
    parent_path: PathBuf,
    name: OsString,
    version: Option<FileVersion>,
}

/// A fresh, read-only resolution of the historical complete source path.
/// Retains its parent handle only for the operation's relationship checks; this
/// observation never replaces a disk binding or its original version baseline.
pub(super) struct HistoricalSource {
    observed: ObservedTarget,
}

#[derive(Clone, Copy)]
pub(super) struct HistoricalRelation {
    pub same_file: bool,
    pub same_parent: bool,
    pub same_leaf: bool,
}

impl HistoricalSource {
    pub(super) fn observe(path: &Path) -> Result<Self, IdentityError> {
        let observed = ObservedTarget::inspect(path)?;
        observed.recheck()?;
        Ok(Self { observed })
    }

    pub(super) fn relation_to(
        &self,
        destination: &ObservedTarget,
    ) -> Result<HistoricalRelation, IdentityError> {
        self.recheck()?;
        destination.recheck()?;
        let same_file = match (&self.observed.version, &destination.version) {
            (Some(source), Some(destination)) => source.identity == destination.identity,
            _ => false,
        };
        let same_parent = self.observed.parent_identity == destination.parent_identity;
        // Only exact spelling or ASCII case folding is supported here. This is
        // a name relation, never independent proof of file identity.
        let same_leaf = self.observed.name == destination.name
            || match (self.observed.name.to_str(), destination.name.to_str()) {
                (Some(source), Some(destination)) => source.eq_ignore_ascii_case(destination),
                _ => false,
            };
        Ok(HistoricalRelation {
            same_file,
            same_parent,
            same_leaf,
        })
    }

    pub(super) fn recheck(&self) -> Result<(), IdentityError> {
        self.observed.recheck()
    }
}

fn handle_info<T: Default>(
    file: &File,
    class: FILE_INFO_BY_HANDLE_CLASS,
) -> Result<T, IdentityError> {
    let mut value = T::default();
    // SAFETY: the selected Windows structures below have their documented C
    // layout; the live File owns the handle and value is writable for its size.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            class,
            (&mut value as *mut T).cast(),
            std::mem::size_of::<T>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(value)
}
pub(super) fn identity(file: &File) -> Result<FileIdentity, IdentityError> {
    let i: FILE_ID_INFO = handle_info(file, FileIdInfo)?;
    Ok(FileIdentity {
        volume: i.VolumeSerialNumber,
        id: i.FileId.Identifier,
    })
}
pub(super) fn check_kind(file: &File, directory: bool) -> Result<(), IdentityError> {
    let attrs: FILE_ATTRIBUTE_TAG_INFO = handle_info(file, FileAttributeTagInfo)?;
    if attrs.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(IdentityError::ReparsePoint);
    }
    let m = file.metadata()?;
    if if directory { !m.is_dir() } else { !m.is_file() } {
        return Err(IdentityError::WrongKind);
    }
    Ok(())
}
pub(super) fn volume_path(file: &File) -> Result<PathBuf, IdentityError> {
    let mut buffer = vec![0u16; 512];
    loop {
        // SAFETY: a live handle and a writable buffer of the passed capacity.
        let n = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                VOLUME_NAME_GUID,
            )
        };
        if n == 0 {
            return Err(io::Error::last_os_error().into());
        }
        if n as usize >= buffer.len() {
            buffer.resize(n as usize + 1, 0);
            continue;
        }
        let path = OsString::from_wide(&buffer[..n as usize]);
        // Require a local volume-GUID path. Network paths/unsupported filesystems
        // fail explicitly rather than falling back to an unchecked path string.
        if !path.to_string_lossy().starts_with(r"\\?\Volume{") {
            return Err(IdentityError::UnsupportedVolumePath);
        }
        return Ok(PathBuf::from(path));
    }
}
fn open_file(path: &Path) -> Result<File, IdentityError> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    check_kind(&file, false)?;
    Ok(file)
}
fn read_version(mut file: File) -> Result<FileVersion, IdentityError> {
    read_version_handle(&mut file)
}
pub(super) fn read_version_handle(file: &mut File) -> Result<FileVersion, IdentityError> {
    let id = identity(&file)?;
    let security = SecurityDescriptor::read(file)?;
    let before = file.metadata()?;
    let modified = before.modified()?;
    file.rewind()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    if id != identity(&file)?
        || before.len() != after.len()
        || after.len() != bytes.len() as u64
        || modified != after.modified()?
        || security != SecurityDescriptor::read(file)?
    {
        return Err(IdentityError::Changed);
    }
    Ok(FileVersion {
        security,
        identity: id,
        modified,
        bytes: bytes.into(),
    })
}
fn optional_version(path: &Path) -> Result<Option<FileVersion>, IdentityError> {
    match open_file(path) {
        Ok(file) => read_version(file).map(Some),
        Err(IdentityError::Io(e)) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
impl ObservedTarget {
    /// Retains a strictly shared read handle for this observation's original
    /// complete version. This does not refresh its baseline or bind a document.
    pub(super) fn hold_existing_read(&self) -> Result<File, IdentityError> {
        existing_read::hold_existing_read(self)
    }
    pub(super) fn hold_replacement_victim(&self) -> Result<File, IdentityError> {
        existing_read::hold_replacement_victim(self)
    }
    pub(super) fn target_name(&self) -> &OsStr {
        &self.name
    }
    pub(super) fn parent_for_new_file(&self) -> Result<File, IdentityError> {
        self.recheck_parent()?;
        let parent = OpenOptions::new()
            .access_mode(FILE_ADD_FILE | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | READ_CONTROL)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(&self.parent_path)?;
        check_kind(&parent, true)?;
        if identity(&parent)? != self.parent_identity
            || volume_path(&parent)? != self.parent_path
            || SecurityDescriptor::read(&parent)? != self.parent_security
        {
            return Err(IdentityError::Changed);
        }
        Ok(parent)
    }
    /// Resolves a local volume and a file-ID from handles. Reads original bytes;
    /// never creates or modifies the target. Only the final parent/leaf may not
    /// be reparses; ancestor aliases resolve through the opened parent handle.
    pub fn inspect(path: &Path) -> Result<Self, IdentityError> {
        if !path.is_absolute() {
            return Err(IdentityError::InvalidPath);
        }
        let name = path
            .file_name()
            .ok_or(IdentityError::InvalidPath)?
            .to_os_string();
        let units: Vec<_> = name.encode_wide().collect();
        if units.is_empty()
            || units.iter().any(|u| [0, 58, 47, 92].contains(u))
            || name == OsStr::new(".")
            || name == OsStr::new("..")
            || matches!(units.last(), Some(32 | 46))
        {
            return Err(IdentityError::InvalidPath);
        }
        let parent = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES | READ_CONTROL)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path.parent().ok_or(IdentityError::InvalidPath)?)?;
        check_kind(&parent, true)?;
        let parent_identity = identity(&parent)?;
        let parent_security = SecurityDescriptor::read(&parent)?;
        let parent_path = volume_path(&parent)?;
        let version = optional_version(&parent_path.join(&name))?;
        Ok(Self {
            parent,
            parent_security,
            parent_identity,
            parent_path,
            name,
            version,
        })
    }
    pub fn version(&self) -> Option<&FileVersion> {
        self.version.as_ref()
    }
    pub fn parent_identity(&self) -> FileIdentity {
        self.parent_identity
    }
    pub fn parent_security(&self) -> &SecurityDescriptor {
        &self.parent_security
    }
    pub fn resolved_path(&self) -> PathBuf {
        self.parent_path.join(&self.name)
    }
    /// Resolve an existing leaf through its actual handle, not input spelling.
    /// Compare full observed version so a raced replacement is not canonicalized
    /// into a credential for another object.
    pub(super) fn canonical_observed_path(&self) -> Result<Option<PathBuf>, IdentityError> {
        let Some(expected) = &self.version else {
            return Ok(None);
        };
        let mut file = open_file(&self.resolved_path())?;
        if &read_version_handle(&mut file)? != expected {
            return Err(IdentityError::Changed);
        }
        let path = volume_path(&file)?;
        self.recheck_parent()?;
        Ok(Some(path))
    }
    /// Existing hardlinks share one key. For absent files deliberately reserve
    /// the whole directory, avoiding unsafe case-folding/short-name assumptions.
    pub fn reservation_key(&self) -> TargetKey {
        let (kind, id) = match &self.version {
            Some(v) => return v.identity.target_key(),
            None => ("new-in-directory", self.parent_identity),
        };
        let hex: String = id.id.iter().map(|b| format!("{b:02x}")).collect();
        TargetKey(format!("windows-{kind}:{:016x}:{hex}", id.volume))
    }
    /// Always include the parent domain, including when this target exists.
    /// This is deliberately conservative: all writes in one directory serialize.
    pub fn save_scope(&self) -> crate::SaveScope {
        let id = self.parent_identity;
        let hex: String = id.id.iter().map(|b| format!("{b:02x}")).collect();
        crate::SaveScope {
            parent: TargetKey(format!("windows-parent:{:016x}:{hex}", id.volume)),
            object: self.version.as_ref().map(|_| self.reservation_key()),
        }
    }
    /// Full byte comparison is intentionally not described as compare-and-swap.
    /// Publication must still retain any version raced in after this check.
    pub fn recheck(&self) -> Result<(), IdentityError> {
        self.recheck_parent()?;
        if optional_version(&self.resolved_path())? != self.version {
            return Err(IdentityError::Changed);
        }
        Ok(())
    }
    pub(super) fn recheck_parent(&self) -> Result<(), IdentityError> {
        if identity(&self.parent)? != self.parent_identity
            || SecurityDescriptor::read(&self.parent)? != self.parent_security
            || volume_path(&self.parent)? != self.parent_path
        {
            return Err(IdentityError::Changed);
        }
        Ok(())
    }
}
