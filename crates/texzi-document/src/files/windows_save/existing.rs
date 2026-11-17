//! Read-only existing-target capture. This type never reaches ReadyToReplace
//! and is not compatible with a replacement operation while its read lock lives.
use super::*;
use std::{fs::File, io, sync::Mutex};

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub enum ExistingReadIssue {
    NoExistingTarget,
    Owner(Error),
    Identity(IdentityError),
}

/// Original staging resources remain owned on every returned error. A local
/// target File that did not pass capture validation is not a disk credential.
pub struct ExistingReadFailure {
    pub issue: ExistingReadIssue,
    pub staged: LockedStaged,
}
impl std::fmt::Debug for ExistingReadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExistingReadFailure")
            .field("issue", &self.issue)
            .field("operation", &self.staged.capture().operation)
            .finish()
    }
}

/// A held, read-only target and the original staged N. It is not an existing
/// publication permit, backup of the victim, or accepted disk baseline.
#[must_use = "retain the captured resources or explicitly return the original staging object"]
pub struct ExistingReadCapture {
    staged: LockedStaged,
    target: Mutex<File>,
    expected: FileVersion,
    path: PathBuf,
}

/// Owned worker input: no Store reference crosses the filesystem operation.
#[must_use = "run the owned read or retain its original staging resources"]
pub struct ExistingReadWork {
    staged: LockedStaged,
    expected: FileVersion,
    path: PathBuf,
}

/// Completed read awaiting the single owner's fresh capture validation. This
/// candidate cannot publish or update a disk baseline, even when its bytes match.
#[must_use = "validate the returned read with its owner or release the resources"]
pub struct ExistingReadCandidate {
    captured: ExistingReadCapture,
}

impl LockedStaged {
    /// Quick owner preflight only. No File read, Store mutation, stage advance,
    /// caller-supplied path/version, or permissive fallback.
    pub fn begin_existing_read(
        self,
        store: &Store,
    ) -> Result<ExistingReadWork, ExistingReadFailure> {
        let result = (|| -> Result<_, ExistingReadIssue> {
            self.check_owner(store).map_err(ExistingReadIssue::Owner)?;
            let expected = self
                .staged
                .observed
                .version()
                .cloned()
                .ok_or(ExistingReadIssue::NoExistingTarget)?;
            Ok((expected, self.staged.observed.resolved_path()))
        })();
        match result {
            Ok((expected, path)) => Ok(ExistingReadWork {
                staged: self,
                expected,
                path,
            }),
            Err(issue) => Err(ExistingReadFailure {
                issue,
                staged: self,
            }),
        }
    }
}

impl ExistingReadWork {
    pub fn capture(&self) -> &SaveCapture {
        self.staged.capture()
    }
    /// Blocking filesystem worker. Caller may continue editing the Store while
    /// this owned input exists; no Store callback or terminal result is produced.
    pub fn run(self) -> Result<ExistingReadCandidate, ExistingReadFailure> {
        let result = (|| -> Result<_, ExistingReadIssue> {
            self.staged
                .recheck_files()
                .map_err(ExistingReadIssue::Identity)?;
            let target = self
                .staged
                .staged
                .observed
                .hold_existing_read()
                .map_err(ExistingReadIssue::Identity)?;
            // The target now excludes ordinary write/delete opens. Revalidate
            // the original staged File, parent and historical source as well.
            self.staged
                .recheck_files()
                .map_err(ExistingReadIssue::Identity)?;
            Ok(target)
        })();
        match result {
            Ok(target) => Ok(ExistingReadCandidate {
                captured: ExistingReadCapture {
                    staged: self.staged,
                    target: Mutex::new(target),
                    expected: self.expected,
                    path: self.path,
                },
            }),
            Err(issue) => Err(ExistingReadFailure {
                issue,
                staged: self.staged,
            }),
        }
    }
}

impl ExistingReadCandidate {
    pub fn capture(&self) -> &SaveCapture {
        self.captured.capture()
    }
    /// Pure owner validation after IO. Late results never acquire a new save's
    /// identity, advance its stage, or overwrite its baseline. No fallible File
    /// observation occurs after this owner check.
    pub fn accept(self, store: &Store) -> Result<ExistingReadCapture, ExistingReadFailure> {
        match self.captured.staged.check_owner(store) {
            Ok(()) => Ok(self.captured),
            Err(error) => Err(ExistingReadFailure {
                issue: ExistingReadIssue::Owner(error),
                staged: self.captured.into_staged(),
            }),
        }
    }
}

impl ExistingReadCapture {
    pub fn capture(&self) -> &SaveCapture {
        self.staged.capture()
    }
    pub fn format(&self) -> Format {
        self.staged.format()
    }
    pub fn payload(&self) -> &[u8] {
        self.staged.payload()
    }
    pub fn read_staged_bytes(&self) -> io::Result<Vec<u8>> {
        self.staged.read_created_bytes()
    }
    /// Original observed target version, not this operation's new N payload.
    pub fn expected_target_version(&self) -> &FileVersion {
        &self.expected
    }
    fn target_file(&self) -> Result<std::sync::MutexGuard<'_, File>, IdentityError> {
        self.target
            .lock()
            .map_err(|_| IdentityError::Io(io::Error::other("existing read capture lock poisoned")))
    }
    /// Actual same-handle read; alone this is not an owner/capture revalidation.
    pub fn current_target_version(&self) -> Result<FileVersion, IdentityError> {
        super::super::windows_identity::read_version_handle(&mut self.target_file()?)
    }
    pub fn current_target_path(&self) -> Result<PathBuf, IdentityError> {
        super::super::windows_identity::volume_path(&self.target_file()?)
    }
    /// Blocking diagnostic recheck, not a UI worker entrypoint. Ordinary sharing
    /// does not freeze ACLs, mapped writers or
    /// the whole filesystem, so compare actual full versions again when asked.
    pub fn recheck(&self, store: &Store) -> Result<(), ExistingReadIssue> {
        self.staged
            .check_owner(store)
            .map_err(ExistingReadIssue::Owner)?;
        self.staged
            .recheck_files()
            .map_err(ExistingReadIssue::Identity)?;
        let mut file = self.target_file().map_err(ExistingReadIssue::Identity)?;
        let version = super::super::windows_identity::read_version_handle(&mut file)
            .map_err(ExistingReadIssue::Identity)?;
        let path = super::super::windows_identity::volume_path(&file)
            .map_err(ExistingReadIssue::Identity)?;
        self.staged
            .staged
            .observed
            .recheck_parent()
            .map_err(ExistingReadIssue::Identity)?;
        if version != self.expected || path != self.path {
            return Err(ExistingReadIssue::Identity(IdentityError::Changed));
        }
        drop(file);
        self.staged
            .check_owner(store)
            .map_err(ExistingReadIssue::Owner)?;
        Ok(())
    }
    /// Releases only the target's read capture. This does not cancel or accept
    /// the save, and the returned stage needs fresh checks for any next action.
    pub fn into_staged(self) -> LockedStaged {
        let Self { staged, target, .. } = self;
        drop(target);
        staged
    }
}
