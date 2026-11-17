//! Bound Windows staging and limited new-target publication; replacement pending.
mod publish;
#[cfg(test)]
mod tests;
use super::{
    FileBinding, FileError, Format, PreparedSave,
    staging::{ContentGuard, StageIssue, StagedSave, StagingWork},
    windows_identity::{FileIdentity, FileVersion, IdentityError, ObservedTarget},
    windows_security::SecurityDescriptor,
};
use crate::{Error, SaveCapture, SaveStage, Store};
pub use publish::{
    CommittedNew, NewPublication, NewReady, PublishFailure, PublishIssue, PublishedNew,
};
use std::path::{Path, PathBuf};

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
    temporary: FileVersion,
    staged: StagedSave,
    format: Format,
}
pub struct LockedStaged {
    staged: BoundStaged,
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
            .write_temp_protected(
                &self.observed.resolved_path(),
                &guard,
                self.observed.version().map(|v| &v.security),
            )
            .map_err(|e| fail(BoundIssue::Stage(e.issue), e.recovery_temp))?;
        let recovery = Some(staged.recovery_temp().to_owned());
        self.observed
            .recheck()
            .map_err(|e| fail(BoundIssue::Identity(e), recovery.clone()))?;
        let (temp, current_path) = (|| -> Result<_, IdentityError> {
            let mut file = staged.created_file()?;
            Ok((
                super::windows_identity::read_version_handle(&mut file)?,
                super::windows_identity::volume_path(&file)?,
            ))
        })()
        .map_err(|e| fail(BoundIssue::Identity(e), recovery.clone()))?;
        if current_path != staged.recovery_temp()
            || current_path.parent() != self.observed.resolved_path().parent()
            || temp.identity != staged.created_identity()
            || &temp.security != staged.payload_security()
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
            temporary: temp,
            staged,
            format: self.format,
        })
    }
}
impl BoundStaged {
    /// Validates the original create_new handle, held with no read/write/delete
    /// sharing since creation. Never close/reopen to establish identity.
    pub fn lock_temporary(self) -> Result<LockedStaged, LockFailure> {
        let attempt = (|| -> Result<SecurityDescriptor, IdentityError> {
            self.recheck_files()?;
            let current = {
                let mut file = self.staged.created_file()?;
                super::windows_identity::read_version_handle(&mut file)?
            };
            if current != self.temporary
                || current.identity != self.staged.created_identity()
                || &current.security != self.staged.payload_security()
            {
                return Err(IdentityError::Changed);
            }
            self.observed.recheck()?;
            Ok(current.security)
        })();
        match attempt {
            Ok(security) => Ok(LockedStaged {
                staged: self,
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
    pub fn read_created_bytes(&self) -> std::io::Result<Vec<u8>> {
        self.staged.read_created_bytes()
    }
    pub fn temporary_security(&self) -> &SecurityDescriptor {
        &self.temporary.security
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
        self.staged.created_identity()
    }
    pub fn creation_security(&self) -> &SecurityDescriptor {
        self.staged.created_security()
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
    /// Owner-held stage recheck; not a CAS or publication authorization.
    pub fn recheck_files(&self) -> Result<(), IdentityError> {
        self.observed.recheck()?;
        let mut file = self.staged.created_file()?;
        if super::windows_identity::read_version_handle(&mut file)? != self.temporary
            || super::windows_identity::volume_path(&file)? != self.staged.recovery_temp()
        {
            return Err(IdentityError::Changed);
        }
        Ok(())
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
    pub fn read_created_bytes(&self) -> std::io::Result<Vec<u8>> {
        self.staged.read_created_bytes()
    }
    pub fn format(&self) -> Format {
        self.staged.format()
    }
    pub fn temporary_security(&self) -> &SecurityDescriptor {
        &self.security
    }
    pub fn creation_security(&self) -> &SecurityDescriptor {
        self.staged.creation_security()
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
