//! Pre-permit staging only. Never replaces/deletes a target or reports save success.
use super::PreparedSave;
use crate::{SaveStage, Store};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    sync::Arc,
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
}
pub struct StagedSave {
    created: File,
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
    #[cfg(windows)]
    pub(super) fn created_file_mut(&mut self) -> &mut File {
        &mut self.created
    }
    pub fn capture(&self) -> &crate::SaveCapture {
        &self.capture
    }
    pub fn expected_bytes(&self) -> &[u8] {
        &self.expected
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
            })?;
        Ok(StagingWork {
            capture: self.capture.clone(),
            bytes: self.bytes.clone(),
        })
    }
}
impl StagingWork {
    /// Reserves an exclusive sibling, writes the captured payload, flushes and
    /// rereads it, then rechecks the target. Errors keep reservation and temp.
    /// There is deliberately no publish/commit/success function before identity
    /// coordination and the post-permit failure API have been reviewed.
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
                    FILE_SHARE_READ, WRITE_DAC, WRITE_OWNER,
                };
                options
                    .access_mode(
                        FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE | WRITE_DAC | WRITE_OWNER,
                    )
                    .share_mode(FILE_SHARE_READ)
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
        let (temp, mut file) = chosen.ok_or_else(|| fail(StageIssue::TempNameExhausted))?;
        // Record identity/security from the still-open create_new handle before
        // writing any payload, not from a later reopen of its pathname.
        #[cfg(windows)]
        let created_identity =
            super::windows_identity::identity(&file).map_err(|e| StageFailure {
                issue: StageIssue::Identity(e),
                recovery_temp: Some(temp.clone()),
            })?;
        #[cfg(windows)]
        let created_security =
            super::windows_security::SecurityDescriptor::read(&file).map_err(|e| StageFailure {
                issue: StageIssue::Io(e),
                recovery_temp: Some(temp.clone()),
            })?;
        #[cfg(windows)]
        if let Some(required) = required_security {
            required
                .apply_to_new_file(&file)
                .map_err(|e| StageFailure {
                    issue: StageIssue::Io(e),
                    recovery_temp: Some(temp.clone()),
                })?;
        }
        #[cfg(windows)]
        let payload_security =
            super::windows_security::SecurityDescriptor::read(&file).map_err(|e| StageFailure {
                issue: StageIssue::Io(e),
                recovery_temp: Some(temp.clone()),
            })?;
        #[cfg(windows)]
        if required_security.is_some_and(|required| required != &payload_security) {
            return Err(StageFailure {
                issue: StageIssue::SecurityMismatch,
                recovery_temp: Some(temp),
            });
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
        work.map_err(|issue| StageFailure {
            issue,
            recovery_temp: Some(temp.clone()),
        })?;
        Ok(StagedSave {
            created: file,
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
