//! Pre-permit staging only. Never replaces/deletes a target or reports save success.
use super::PreparedSave;
use crate::{SaveStage, Store};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Clone)]
pub enum ContentGuard {
    Absent,
    Exact(Arc<[u8]>),
}
#[derive(Debug)]
pub enum StageIssue {
    Io(std::io::Error),
    Document(crate::Error),
    Conflict,
    InvalidTarget,
    TempNameExhausted,
    #[cfg(windows)]
    Identity(super::windows_identity::IdentityError),
    #[cfg(windows)]
    SecurityMismatch,
}
#[derive(Debug)]
pub struct StageFailure {
    pub issue: StageIssue,
    pub recovery_temp: Option<PathBuf>,
    recovery: Option<TemporaryRecovery>,
}
struct CreatedTemp {
    file: Mutex<File>,
    capture: crate::SaveCapture,
    expected: Arc<[u8]>,
    temp: PathBuf,
    target: PathBuf,
}
/// Original created object retained by an explicitly returned error. Expected
/// payload is intent; actual bytes can still be empty or only partly written.
/// This is not a publication permit or a process/worker Drop registry.
#[derive(Clone)]
pub struct TemporaryRecovery {
    created: Arc<CreatedTemp>,
}
impl std::fmt::Debug for TemporaryRecovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemporaryRecovery")
            .field("operation", &self.capture().operation)
            .field("temp_hint", &self.temp_hint())
            .field("expected_len", &self.expected_bytes().len())
            .finish()
    }
}
impl StageFailure {
    pub fn recovery(&self) -> Option<&TemporaryRecovery> {
        self.recovery.as_ref()
    }
}
impl TemporaryRecovery {
    pub fn capture(&self) -> &crate::SaveCapture {
        &self.created.capture
    }
    pub fn expected_bytes(&self) -> &[u8] {
        &self.created.expected
    }
    /// Hints only; do not reopen these paths as recovery capabilities.
    pub fn temp_hint(&self) -> &Path {
        &self.created.temp
    }
    pub fn target_hint(&self) -> &Path {
        &self.created.target
    }
    fn file_for_inspection(&self) -> MutexGuard<'_, File> {
        // Read-only inspection does not clear poison or restore worker rights.
        self.created.file.lock().unwrap_or_else(|e| e.into_inner())
    }
    #[cfg(all(test, windows))]
    pub(super) fn original_handle_for_test(&self) -> usize {
        use std::os::windows::io::AsRawHandle;
        self.file_for_inspection().as_raw_handle() as usize
    }
    pub fn read_created_bytes(&self) -> std::io::Result<Vec<u8>> {
        let mut file = self.file_for_inspection();
        file.rewind()?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }
    #[cfg(windows)]
    pub fn current_path(&self) -> Result<PathBuf, super::windows_identity::IdentityError> {
        super::windows_identity::volume_path(&self.file_for_inspection())
    }
    #[cfg(windows)]
    pub fn current_version(
        &self,
    ) -> Result<super::windows_identity::FileVersion, super::windows_identity::IdentityError> {
        super::windows_identity::read_version_handle(&mut self.file_for_inspection())
    }
}
pub struct StagedSave {
    created: Arc<CreatedTemp>,
    #[cfg(windows)]
    created_identity: super::windows_identity::FileIdentity,
    #[cfg(windows)]
    created_security: super::windows_security::SecurityDescriptor,
    #[cfg(windows)]
    payload_security: super::windows_security::SecurityDescriptor,
    temp: PathBuf,
    target: PathBuf,
    capture: crate::SaveCapture,
    expected: Arc<[u8]>,
}
pub struct StagingWork {
    capture: crate::SaveCapture,
    bytes: Arc<[u8]>,
    #[cfg(all(test, windows))]
    security_hook: Option<Box<dyn Fn(&File) -> std::io::Result<()> + Send>>,
    #[cfg(all(test, windows))]
    creation_hook: Option<Box<dyn Fn(&File, &Path) -> std::io::Result<()> + Send>>,
    #[cfg(all(test, windows))]
    copy_handle_hook: Option<Box<dyn Fn(&File) -> std::io::Result<File> + Send>>,
}
impl StagedSave {
    #[cfg(windows)]
    pub fn created_identity(&self) -> super::windows_identity::FileIdentity {
        self.created_identity
    }
    #[cfg(windows)]
    pub fn created_security(&self) -> &super::windows_security::SecurityDescriptor {
        &self.created_security
    }
    #[cfg(windows)]
    pub fn payload_security(&self) -> &super::windows_security::SecurityDescriptor {
        &self.payload_security
    }
    pub(super) fn created_file(&self) -> std::io::Result<MutexGuard<'_, File>> {
        self.created
            .file
            .lock()
            .map_err(|_| std::io::Error::other("temporary handle lock poisoned"))
    }
    /// Recovery inspection only; poisoning never grants a publication capability.
    #[cfg(windows)]
    pub(super) fn created_file_recovery(&self) -> MutexGuard<'_, File> {
        self.created.file.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Owner capability read: never reopens the temporary pathname.
    pub fn read_created_bytes(&self) -> std::io::Result<Vec<u8>> {
        let mut file = self.created_file()?;
        file.rewind()?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }
    pub fn capture(&self) -> &crate::SaveCapture {
        &self.capture
    }
    pub fn expected_bytes(&self) -> &[u8] {
        &self.expected
    }
    /// Retain the same original object for an outer returned-error path.
    pub(super) fn recovery(&self) -> TemporaryRecovery {
        TemporaryRecovery {
            created: self.created.clone(),
        }
    }
    pub fn recovery_temp(&self) -> &Path {
        &self.temp
    }
    pub fn target(&self) -> &Path {
        &self.target
    }
}
impl ContentGuard {
    /// Content comparison only; not a Windows file-ID/reparse resolver or CAS.
    pub fn verify(&self, target: &Path) -> Result<(), StageIssue> {
        let metadata = match fs::symlink_metadata(target) {
            Ok(m) => Some(m),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(StageIssue::Io(e)),
        };
        match (self, metadata) {
            (Self::Absent, None) => Ok(()),
            (Self::Exact(expected), Some(m)) if m.is_file() && !m.file_type().is_symlink() => {
                let actual = fs::read(target).map_err(StageIssue::Io)?;
                if actual.as_slice() == expected.as_ref() {
                    Ok(())
                } else {
                    Err(StageIssue::Conflict)
                }
            }
            _ => Err(StageIssue::Conflict),
        }
    }
}
impl PreparedSave {
    /// The UI owner holds Store only for the quick stage transition.
    pub fn begin_staging(&self, store: &mut Store) -> Result<StagingWork, StageFailure> {
        store
            .advance_save(&self.capture, SaveStage::WritingTemp)
            .map_err(|e| StageFailure {
                issue: StageIssue::Document(e),
                recovery_temp: None,
                recovery: None,
            })?;
        Ok(StagingWork {
            capture: self.capture.clone(),
            bytes: self.bytes.clone(),
            #[cfg(all(test, windows))]
            security_hook: None,
            #[cfg(all(test, windows))]
            creation_hook: None,
            #[cfg(all(test, windows))]
            copy_handle_hook: None,
        })
    }
}
impl StagingWork {
    #[cfg(all(test, windows))]
    pub(super) fn set_copy_handle_hook(
        &mut self,
        hook: impl Fn(&File) -> std::io::Result<File> + Send + 'static,
    ) {
        self.copy_handle_hook = Some(Box::new(hook));
    }
    #[cfg(all(test, windows))]
    pub(super) fn set_creation_hook(
        &mut self,
        hook: impl Fn(&File, &Path) -> std::io::Result<()> + Send + 'static,
    ) {
        self.creation_hook = Some(Box::new(hook));
    }
    #[cfg(all(test, windows))]
    pub(super) fn set_security_hook(
        &mut self,
        hook: impl Fn(&File) -> std::io::Result<()> + Send + 'static,
    ) {
        self.security_hook = Some(Box::new(hook));
    }
    /// Reserves an exclusive sibling, writes the captured payload, flushes and
    /// rereads it, then rechecks the target. Errors keep reservation and temp.
    /// This lower-level API does not publish a name or accept a saved baseline;
    /// those require the separate bound new-target proof and owner permit.
    pub fn write_temp(
        self,
        target: &Path,
        guard: &ContentGuard,
    ) -> Result<StagedSave, StageFailure> {
        #[cfg(windows)]
        {
            self.write_temp_inner(target, guard, None)
        }
        #[cfg(not(windows))]
        {
            self.write_temp_inner(target, guard)
        }
    }
    #[cfg(windows)]
    pub(super) fn write_temp_protected(
        self,
        target: &Path,
        guard: &ContentGuard,
        security: Option<&super::windows_security::SecurityDescriptor>,
    ) -> Result<StagedSave, StageFailure> {
        self.write_temp_inner(target, guard, security)
    }
    fn write_temp_inner(
        self,
        target: &Path,
        guard: &ContentGuard,
        #[cfg(windows)] required_security: Option<&super::windows_security::SecurityDescriptor>,
    ) -> Result<StagedSave, StageFailure> {
        let fail = |issue| StageFailure {
            issue,
            recovery_temp: None,
            recovery: None,
        };
        if !target.is_absolute() || target.file_name().is_none() {
            return Err(fail(StageIssue::InvalidTarget));
        }
        let parent = target
            .parent()
            .ok_or_else(|| fail(StageIssue::InvalidTarget))?;
        let target_name = target.file_name().unwrap();
        // Reject Windows ambiguous/unsupported leaf forms before any creation.
        // Unicode names remain supported; non-Unicode OS strings are explicit
        // rejections until the identity coordinator owns their interpretation.
        let name = target_name
            .to_str()
            .ok_or_else(|| fail(StageIssue::InvalidTarget))?;
        if name.ends_with('.')
            || name.ends_with(' ')
            || name.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c))
        {
            return Err(fail(StageIssue::InvalidTarget));
        }
        guard.verify(target).map_err(fail)?;
        let mut chosen = None;
        for attempt in 0..64 {
            // Numeric 8.3 names have no case variants and need no distinct DOS
            // short alias. This avoids comparing Windows names using Unicode
            // case folding, or guessing a long name's generated short alias.
            // This is a deterministic collision namespace, not a random secret.
            let stem = (u64::from(std::process::id()) + self.capture.operation % 100_000_000)
                % 100_000_000;
            let candidate = format!("{stem:08}.{attempt:03}");
            if target_name == std::ffi::OsStr::new(&candidate) {
                continue; // Must happen before create_new: even an absent target.
            }
            let path = parent.join(candidate);
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                use windows_sys::Win32::Storage::FileSystem::{
                    DELETE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
                    FILE_SHARE_NONE, WRITE_DAC, WRITE_OWNER,
                };
                let access = FILE_GENERIC_READ
                    | FILE_GENERIC_WRITE
                    | DELETE
                    | WRITE_DAC
                    | if required_security.is_some() {
                        WRITE_OWNER
                    } else {
                        0
                    };
                options
                    .access_mode(access)
                    .share_mode(FILE_SHARE_NONE)
                    .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
            }
            match options.open(&path) {
                Ok(file) => {
                    chosen = Some((path, file));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(fail(StageIssue::Io(e))),
            }
        }
        let (temp, file) = chosen.ok_or_else(|| fail(StageIssue::TempNameExhausted))?;
        // Own the original create_new File before any fallible hook, identity,
        // security or payload operation. Both explicit errors and success keep
        // this same object; no close/reopen or replacement handle is involved.
        let created = Arc::new(CreatedTemp {
            file: Mutex::new(file),
            capture: self.capture.clone(),
            expected: self.bytes.clone(),
            temp: temp.clone(),
            target: target.to_owned(),
        });
        let failed = |issue| StageFailure {
            issue,
            recovery_temp: Some(temp.clone()),
            recovery: Some(TemporaryRecovery {
                created: created.clone(),
            }),
        };
        let mut file = created.file.lock().map_err(|_| {
            failed(StageIssue::Io(std::io::Error::other(
                "temporary handle lock poisoned",
            )))
        })?;
        #[cfg(all(test, windows))]
        if let Some(hook) = &self.creation_hook {
            hook(&file, &temp).map_err(|e| failed(StageIssue::Io(e)))?;
        }
        // Record identity/security from the still-open create_new handle before
        // writing any payload, not from a later reopen of its pathname.
        #[cfg(windows)]
        let created_identity = super::windows_identity::identity(&file)
            .map_err(|e| failed(StageIssue::Identity(e)))?;
        #[cfg(windows)]
        let created_security = super::windows_security::SecurityDescriptor::read(&file)
            .map_err(|e| failed(StageIssue::Io(e)))?;
        #[cfg(windows)]
        if let Some(required) = required_security {
            #[cfg(test)]
            let duplicate = self
                .copy_handle_hook
                .as_ref()
                .map(|hook| hook(&file))
                .transpose()
                .map_err(|e| failed(StageIssue::Io(e)))?;
            #[cfg(test)]
            let copy_file = duplicate.as_ref().unwrap_or(&file);
            #[cfg(not(test))]
            let copy_file = &file;
            required
                .apply_to_new_file(copy_file)
                .map_err(|e| failed(StageIssue::Io(e)))?;
        } else {
            super::windows_security::SecurityDescriptor::protect_new_file(&file)
                .map_err(|e| failed(StageIssue::Io(e)))?;
        }
        // Unit-test-only fault at the real post-copy/pre-payload boundary. No
        // hook, environment switch, or alternative path exists in product builds.
        #[cfg(all(test, windows))]
        if let Some(hook) = &self.security_hook {
            hook(&file).map_err(|e| failed(StageIssue::Io(e)))?;
        }
        #[cfg(windows)]
        let payload_security = super::windows_security::SecurityDescriptor::read(&file)
            .map_err(|e| failed(StageIssue::Io(e)))?;
        #[cfg(windows)]
        if required_security.is_some_and(|required| required != &payload_security) {
            return Err(failed(StageIssue::SecurityMismatch));
        }
        let work = (|| -> Result<(), StageIssue> {
            file.write_all(&self.bytes).map_err(StageIssue::Io)?;
            file.sync_all().map_err(StageIssue::Io)?;
            file.rewind().map_err(StageIssue::Io)?;
            let mut actual = Vec::new();
            file.read_to_end(&mut actual).map_err(StageIssue::Io)?;
            if actual.as_slice() != self.bytes.as_ref() {
                return Err(StageIssue::Conflict);
            }
            guard.verify(target)?;
            #[cfg(windows)]
            {
                if super::windows_identity::identity(&file).map_err(StageIssue::Identity)?
                    != created_identity
                    || super::windows_security::SecurityDescriptor::read(&file)
                        .map_err(StageIssue::Io)?
                        != payload_security
                {
                    return Err(StageIssue::Conflict);
                }
            }
            // Stay WritingTemp until a filesystem identity coordinator verifies
            // the target/parent/epoch and explicitly authorizes ReadyToReplace.
            Ok(())
        })();
        work.map_err(failed)?;
        // Release the Mutex guard, never the underlying original File.
        drop(file);
        Ok(StagedSave {
            created,
            #[cfg(windows)]
            created_identity,
            #[cfg(windows)]
            created_security,
            #[cfg(windows)]
            payload_security,
            temp,
            target: target.to_owned(),
            capture: self.capture,
            expected: self.bytes,
        })
    }
}
