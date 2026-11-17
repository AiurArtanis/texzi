//! Bound, pre-permit Windows staging. No replace, Ready, or success entry point.
use super::{
    FileBinding, FileError, Format, PreparedSave,
    staging::{ContentGuard, StageIssue, StagedSave, StagingWork},
    windows_identity::{FileIdentity, IdentityError, ObservedTarget},
    windows_security::SecurityDescriptor,
};
use crate::{Error, SaveCapture, SaveStage, Store};
use std::path::{Path, PathBuf};
use std::{
    fs::{File, OpenOptions},
    os::windows::fs::OpenOptionsExt,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};

#[derive(Debug)]
pub enum BoundIssue {
    Identity(IdentityError),
    File(FileError),
    Stage(StageIssue),
    Document(Error),
}
pub struct BoundFailure {
    pub issue: BoundIssue,
    pub capture: Option<SaveCapture>,
    pub recovery_temp: Option<PathBuf>,
}
impl std::fmt::Debug for BoundFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundFailure")
            .field("issue", &self.issue)
            .field("operation", &self.capture.as_ref().map(|c| c.operation))
            .field("recovery_temp", &self.recovery_temp)
            .finish()
    }
}
pub struct BoundPrepared {
    observed: ObservedTarget,
    prepared: PreparedSave,
}
pub struct BoundWork {
    observed: ObservedTarget,
    work: StagingWork,
    capture: SaveCapture,
    format: Format,
}
pub struct BoundStaged {
    observed: ObservedTarget,
    temporary: ObservedTarget,
    staged: StagedSave,
    format: Format,
}
pub struct LockedStaged {
    staged: BoundStaged,
    _file: File,
    security: SecurityDescriptor,
}
pub struct LockFailure {
    pub issue: IdentityError,
    pub staged: BoundStaged,
}
impl std::fmt::Debug for LockFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LockFailure")
            .field("issue", &self.issue)
            .field("recovery_temp", &self.staged.recovery_temp())
            .finish()
    }
}
impl ObservedTarget {
    /// The observation is consumed: callers cannot replace its target, scope or
    /// guard after reserving. This still only authorizes temporary-file creation.
    pub fn prepare_bound(
        self,
        binding: &FileBinding,
        store: &mut Store,
    ) -> Result<BoundPrepared, BoundFailure> {
        let fail = |issue| BoundFailure {
            issue,
            capture: None,
            recovery_temp: None,
        };
        self.recheck().map_err(|e| fail(BoundIssue::Identity(e)))?;
        let snapshot = store
            .snapshot(binding.doc())
            .map_err(|e| fail(BoundIssue::Document(e)))?;
        let format = snapshot
            .body
            .profile
            .file_format
            .ok_or_else(|| fail(BoundIssue::File(FileError::ProfileChangeNeedsCoordination)))?;
        let prepared = binding
            .prepare_with_scope(
                store,
                self.reservation_key(),
                "bound-observation; exact bytes retained by owned worker".into(),
                format,
                Some(self.save_scope()),
            )
            .map_err(|e| fail(BoundIssue::File(e)))?;
        Ok(BoundPrepared {
            observed: self,
            prepared,
        })
    }
}
impl BoundPrepared {
    pub fn capture(&self) -> &SaveCapture {
        self.prepared.capture()
    }
    pub fn begin(self, store: &mut Store) -> Result<BoundWork, BoundFailure> {
        let work = self
            .prepared
            .begin_staging(store)
            .map_err(|e| BoundFailure {
                issue: BoundIssue::Stage(e.issue),
                capture: Some(self.prepared.capture().clone()),
                recovery_temp: e.recovery_temp,
            })?;
        Ok(BoundWork {
            observed: self.observed,
            work,
            capture: self.prepared.capture().clone(),
            format: self.prepared.format(),
        })
    }
}
impl BoundWork {
    /// No Store borrow and no caller-supplied path/guard/capture. The pinned
    /// parent and the complete original target observation travel with the work.
    pub fn write(self) -> Result<BoundStaged, BoundFailure> {
        let fail = |issue, recovery_temp| BoundFailure {
            issue,
            capture: Some(self.capture.clone()),
            recovery_temp,
        };
        self.observed
            .recheck()
            .map_err(|e| fail(BoundIssue::Identity(e), None))?;
        let guard = match self.observed.version() {
            Some(v) => ContentGuard::Exact(v.bytes.clone()),
            None => ContentGuard::Absent,
        };
        let staged = self
            .work
            .write_temp(&self.observed.resolved_path(), &guard)
            .map_err(|e| fail(BoundIssue::Stage(e.issue), e.recovery_temp))?;
        let recovery = Some(staged.recovery_temp().to_owned());
        self.observed
            .recheck()
            .map_err(|e| fail(BoundIssue::Identity(e), recovery.clone()))?;
        let temporary = ObservedTarget::inspect(staged.recovery_temp())
            .map_err(|e| fail(BoundIssue::Identity(e), recovery.clone()))?;
        let temp = temporary.version().ok_or_else(|| {
            fail(
                BoundIssue::Identity(IdentityError::Changed),
                recovery.clone(),
            )
        })?;
        if temporary.parent_identity() != self.observed.parent_identity()
            || temp.identity.volume != self.observed.parent_identity().volume
            || temp.bytes.as_ref() != staged.expected_bytes()
            || self
                .observed
                .version()
                .is_some_and(|v| v.identity == temp.identity)
        {
            return Err(fail(BoundIssue::Identity(IdentityError::Changed), recovery));
        }
        Ok(BoundStaged {
            observed: self.observed,
            temporary,
            staged,
            format: self.format,
        })
    }
}
impl BoundStaged {
    /// Read handle excludes ordinary competing write/delete handles. It cannot
    /// authorize publication and is deliberately not unlocked for a path rename.
    pub fn lock_temporary(self) -> Result<LockedStaged, LockFailure> {
        let attempt = (|| -> Result<(File, SecurityDescriptor), IdentityError> {
            self.recheck_files()?;
            let mut file = OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.recovery_temp())?;
            super::windows_identity::check_kind(&file, false)?;
            let current = super::windows_identity::read_version_handle(&mut file)?;
            if Some(&current) != self.temporary.version() {
                return Err(IdentityError::Changed);
            }
            self.observed.recheck()?;
            Ok((file, current.security))
        })();
        match attempt {
            Ok((file, security)) => Ok(LockedStaged {
                staged: self,
                _file: file,
                security,
            }),
            Err(issue) => Err(LockFailure {
                issue,
                staged: self,
            }),
        }
    }
    pub fn capture(&self) -> &SaveCapture {
        self.staged.capture()
    }
    pub fn format(&self) -> Format {
        self.format
    }
    pub fn payload(&self) -> &[u8] {
        self.staged.expected_bytes()
    }
    pub fn recovery_temp(&self) -> &Path {
        self.staged.recovery_temp()
    }
    pub fn target_path(&self) -> PathBuf {
        self.observed.resolved_path()
    }
    pub fn parent_identity(&self) -> FileIdentity {
        self.observed.parent_identity()
    }
    pub fn temporary_identity(&self) -> FileIdentity {
        self.temporary.version().unwrap().identity
    }
    pub fn original_identity(&self) -> Option<FileIdentity> {
        self.observed.version().map(|v| v.identity)
    }
    /// Validates only the owner's still-active capture/stage, no state mutation.
    /// A late result cannot validate against a new reservation after cancellation.
    pub fn check_owner(&self, store: &Store) -> Result<(), Error> {
        if store.captured_save_stage(self.capture())? != SaveStage::WritingTemp {
            return Err(Error::InvalidSave);
        }
        Ok(())
    }
    /// Not a CAS, ACL check, final identity, or authorization to replace anything.
    pub fn recheck_files(&self) -> Result<(), IdentityError> {
        self.observed.recheck()?;
        self.temporary.recheck()
    }
}
impl LockedStaged {
    pub fn capture(&self) -> &SaveCapture {
        self.staged.capture()
    }
    pub fn recovery_temp(&self) -> &Path {
        self.staged.recovery_temp()
    }
    pub fn payload(&self) -> &[u8] {
        self.staged.payload()
    }
    pub fn format(&self) -> Format {
        self.staged.format()
    }
    pub fn temporary_security(&self) -> &SecurityDescriptor {
        &self.security
    }
    pub fn original_security(&self) -> Option<&SecurityDescriptor> {
        self.staged.observed.version().map(|v| &v.security)
    }
    pub fn check_owner(&self, store: &Store) -> Result<(), Error> {
        self.staged.check_owner(store)
    }
    pub fn recheck_files(&self) -> Result<(), IdentityError> {
        self.staged.recheck_files()
    }
}
