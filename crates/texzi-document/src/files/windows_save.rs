//! Bound Windows staging and limited new-target publication; replacement pending.
#[cfg(test)]
mod current_tests;
#[forbid(unsafe_code)]
mod existing;
mod publish;
pub mod recovery;
#[cfg(test)]
mod tests;
use super::{
    FileBinding, FileError, Format, PreparedSave,
    staging::{ContentGuard, StageIssue, StagedSave, StagingWork, TemporaryRecovery},
    windows_identity::{
        FileIdentity, FileVersion, HistoricalSource, IdentityError, ObservedTarget,
    },
    windows_security::SecurityDescriptor,
};
use crate::{Error, SaveCapture, SaveStage, Store};
pub use existing::{
    ExistingReadCandidate, ExistingReadCapture, ExistingReadFailure, ExistingReadIssue,
    ExistingReadWork,
};
pub use publish::{
    CommittedNew, NewFileSecurity, NewPublication, NewReady, PublishFailure, PublishIssue,
    PublishedNew,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

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
    recovery: Option<BoundRecovery>,
}
impl BoundFailure {
    /// Retained resources for this returned error only, not a publication permit.
    pub fn recovery(&self) -> Option<&BoundRecovery> {
        self.recovery.as_ref()
    }
}
/// Read-only resources retained by a returned pre-permit error. No Store state
/// transition, automatic cleanup, or protection after all receipts are dropped.
#[derive(Clone)]
pub struct BoundRecovery {
    capture: SaveCapture,
    format: Format,
    temporary: Option<TemporaryRecovery>,
    _historical_source: Option<Arc<HistoricalSource>>,
    _source_anchor: Option<Arc<super::windows_identity::OpenAnchor>>,
    observed: Arc<ObservedTarget>,
}
impl BoundRecovery {
    pub fn capture(&self) -> &SaveCapture {
        &self.capture
    }
    pub fn format(&self) -> Format {
        self.format
    }
    pub fn temporary(&self) -> Option<&TemporaryRecovery> {
        self.temporary.as_ref()
    }
    pub fn parent_identity(&self) -> FileIdentity {
        self.observed.parent_identity()
    }
    pub fn target_path(&self) -> PathBuf {
        self.observed.resolved_path()
    }
    /// Inspect the retained original parent; does not authorize a save action.
    pub fn recheck_parent(&self) -> Result<(), IdentityError> {
        self.observed.recheck_parent()
    }
    fn failure(&self, issue: BoundIssue, temporary: Option<TemporaryRecovery>) -> BoundFailure {
        let recovery_temp = temporary.as_ref().map(|temp| temp.temp_hint().to_owned());
        let mut recovery = self.clone();
        recovery.temporary = temporary;
        BoundFailure {
            issue,
            capture: Some(self.capture.clone()),
            recovery_temp,
            recovery: Some(recovery),
        }
    }
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
    historical_source: Option<HistoricalSource>,
    source_anchor: Option<super::windows_identity::OpenAnchor>,
    observed: ObservedTarget,
    prepared: PreparedSave,
}
pub struct BoundWork {
    historical_source: Option<Arc<HistoricalSource>>,
    source_anchor: Option<Arc<super::windows_identity::OpenAnchor>>,
    observed: Arc<ObservedTarget>,
    work: StagingWork,
    capture: SaveCapture,
    format: Format,
    #[cfg(test)]
    after_staging: Option<Box<dyn Fn(&Path) -> std::io::Result<()> + Send>>,
}
pub struct BoundStaged {
    historical_source: Option<Arc<HistoricalSource>>,
    _source_anchor: Option<Arc<super::windows_identity::OpenAnchor>>,
    observed: Arc<ObservedTarget>,
    temporary: FileVersion,
    staged: StagedSave,
    format: Format,
}
pub struct LockedStaged {
    staged: BoundStaged,
    security: SecurityDescriptor,
    #[cfg(test)]
    creation_security_override: Option<SecurityDescriptor>,
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
        // Historical memory-only fixtures have no disk provenance. Verified
        // bindings automatically get current-target protection at this entry.
        let current = binding.disk.is_some();
        self.prepare_destination(binding, store, current)
    }
    /// Ordinary current-file preflight requires a verified open/publication.
    /// This does not enable existing-target publication or grant a CAS.
    pub fn prepare_current(
        self,
        binding: &FileBinding,
        store: &mut Store,
    ) -> Result<BoundPrepared, BoundFailure> {
        self.prepare_destination(binding, store, true)
    }
    /// Explicit destination change. Old generation is still checked, but source
    /// fingerprint/path is not asserted to describe the new destination.
    pub fn prepare_save_as(
        self,
        binding: &FileBinding,
        store: &mut Store,
    ) -> Result<BoundPrepared, BoundFailure> {
        self.prepare_destination(binding, store, false)
    }
    fn prepare_destination(
        self,
        binding: &FileBinding,
        store: &mut Store,
        current: bool,
    ) -> Result<BoundPrepared, BoundFailure> {
        let fail = |issue| BoundFailure {
            issue,
            capture: None,
            recovery_temp: None,
            recovery: None,
        };
        // Validate existence before reading metadata, including legacy None bindings.
        let owner_snapshot = store
            .snapshot(binding.doc())
            .map_err(|e| fail(BoundIssue::Document(e)))?;
        if binding.disk.is_none() {
            let metadata = store.observer_metadata(binding.doc());
            if metadata.attention.target.is_some()
                || metadata.attention.binding_epoch != 0
                || metadata.baseline.backed_by_disk
                || metadata.baseline.token != binding.imported.token
                || !matches!(
                    metadata.baseline.kind,
                    crate::BaselineKind::InitialEmpty | crate::BaselineKind::MemoryImport
                )
            {
                return Err(fail(BoundIssue::File(FileError::UnverifiedDiskBinding)));
            }
        }
        let disk = if current || binding.disk.is_some() {
            Some(
                binding
                    .checked_disk_binding(store)
                    .map_err(|e| fail(BoundIssue::File(e)))?,
            )
        } else {
            None
        };
        if let Some(disk) = disk {
            if !disk
                .anchor
                .retains(disk.version.identity)
                .map_err(|e| fail(BoundIssue::Identity(e)))?
            {
                return Err(fail(BoundIssue::Identity(IdentityError::Changed)));
            }
        }
        // Retain the verified source object across the owned worker, even if
        // caller drops FileBinding immediately after this preparation returns.
        let source_anchor = disk
            .map(|disk| disk.anchor.try_clone())
            .transpose()
            .map_err(|e| fail(BoundIssue::Identity(e)))?;
        // Re-resolve the complete historical namespace for SaveAs. A changed
        // parent ID/canonical spelling cannot by itself authorize independence.
        // Errors stay errors, never a fallback to "different destination".
        let historical_source = if !current {
            disk.map(|disk| HistoricalSource::observe(&disk.path))
                .transpose()
                .map_err(|e| fail(BoundIssue::Identity(e)))?
        } else {
            None
        };
        self.recheck().map_err(|e| fail(BoundIssue::Identity(e)))?;
        let canonical = if disk.is_some() {
            self.canonical_observed_path()
                .map_err(|e| fail(BoundIssue::Identity(e)))?
        } else {
            None
        };
        let relation = historical_source
            .as_ref()
            .map(|source| source.relation_to(&self))
            .transpose()
            .map_err(|e| fail(BoundIssue::Identity(e)))?;
        // SaveAs cannot disguise the source path (even after ID replacement) or
        // a hardlink to the source object as an independent destination.
        let current = current
            || relation.is_some_and(|r| r.same_file || (r.same_parent && r.same_leaf))
            || disk.is_some_and(|disk| {
                self.resolved_path() == disk.path
                    || canonical.as_ref() == Some(&disk.path)
                    || self
                        .resolved_path()
                        .to_str()
                        .zip(disk.path.to_str())
                        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
                    || self
                        .version()
                        .is_some_and(|version| version.identity == disk.version.identity)
            });
        if let Some(disk) = disk {
            // A different input/canonical spelling is not proof of a different
            // directory entry for Unicode names, even when a new object exists.
            // Proven Current paths/IDs still take the full version check below.
            if !current
                && (self.parent_identity() == disk.parent
                    || self.resolved_path().parent() == disk.path.parent()
                    || relation.is_some_and(|r| r.same_parent))
                && [self.resolved_path().file_name(), disk.path.file_name()]
                    .iter()
                    .any(|name| !name.and_then(|s| s.to_str()).is_some_and(str::is_ascii))
            {
                return Err(fail(BoundIssue::Identity(IdentityError::InvalidPath)));
            }
        }
        if current
            && disk.is_some_and(|disk| {
                self.version() != Some(&disk.version) || canonical.as_ref() != Some(&disk.path)
            })
        {
            return Err(fail(BoundIssue::Identity(IdentityError::Changed)));
        }
        // Current already owns/rechecks its destination. Keep the fresh source
        // observation only for independent SaveAs; it is not a new saved baseline.
        let historical_source = if current { None } else { historical_source };
        if let Some(source) = &historical_source {
            source
                .recheck()
                .map_err(|e| fail(BoundIssue::Identity(e)))?;
        }
        let snapshot = owner_snapshot;
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
            historical_source,
            source_anchor,
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
        let historical_source = self.historical_source.map(Arc::new);
        let source_anchor = self.source_anchor.map(Arc::new);
        let observed = Arc::new(self.observed);
        let capture = self.prepared.capture().clone();
        let format = self.prepared.format();
        let recovery = BoundRecovery {
            capture: capture.clone(),
            format,
            temporary: None,
            _historical_source: historical_source.clone(),
            _source_anchor: source_anchor.clone(),
            observed: observed.clone(),
        };
        let work = self.prepared.begin_staging(store).map_err(|e| {
            let temporary = e.recovery().cloned();
            recovery.failure(BoundIssue::Stage(e.issue), temporary)
        })?;
        Ok(BoundWork {
            historical_source,
            source_anchor,
            observed,
            work,
            capture,
            format,
            #[cfg(test)]
            after_staging: None,
        })
    }
}
impl BoundWork {
    /// Source-only fixture hook at the real inner/outer validation boundary.
    #[cfg(test)]
    pub(super) fn set_after_staging(
        &mut self,
        hook: impl Fn(&Path) -> std::io::Result<()> + Send + 'static,
    ) {
        self.after_staging = Some(Box::new(hook));
    }
    /// No Store borrow and no caller-supplied path/guard/capture. The pinned
    /// parent and the complete original target observation travel with the work.
    pub fn write(self) -> Result<BoundStaged, BoundFailure> {
        let recovery = BoundRecovery {
            capture: self.capture.clone(),
            format: self.format,
            temporary: None,
            _historical_source: self.historical_source.clone(),
            _source_anchor: self.source_anchor.clone(),
            observed: self.observed.clone(),
        };
        let fail = |issue, temporary| recovery.failure(issue, temporary);
        if let Some(source) = &self.historical_source {
            source
                .recheck()
                .map_err(|e| fail(BoundIssue::Identity(e), None))?;
        }
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
            .map_err(|e| {
                let temporary = e.recovery().cloned();
                fail(BoundIssue::Stage(e.issue), temporary)
            })?;
        let temporary_recovery = staged.recovery();
        #[cfg(test)]
        if let Some(hook) = &self.after_staging {
            hook(&self.observed.resolved_path()).map_err(|e| {
                fail(
                    BoundIssue::Identity(IdentityError::Io(e)),
                    Some(temporary_recovery.clone()),
                )
            })?;
        }
        self.observed
            .recheck()
            .map_err(|e| fail(BoundIssue::Identity(e), Some(temporary_recovery.clone())))?;
        if let Some(source) = &self.historical_source {
            source
                .recheck()
                .map_err(|e| fail(BoundIssue::Identity(e), Some(temporary_recovery.clone())))?;
        }
        let (temp, current_path) = (|| -> Result<_, IdentityError> {
            let mut file = staged.created_file()?;
            Ok((
                super::windows_identity::read_version_handle(&mut file)?,
                super::windows_identity::volume_path(&file)?,
            ))
        })()
        .map_err(|e| fail(BoundIssue::Identity(e), Some(temporary_recovery.clone())))?;
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
            return Err(fail(
                BoundIssue::Identity(IdentityError::Changed),
                Some(temporary_recovery),
            ));
        }
        Ok(BoundStaged {
            historical_source: self.historical_source,
            _source_anchor: self.source_anchor,
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
                #[cfg(test)]
                creation_security_override: None,
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
        self.recheck_historical_source()?;
        self.observed.recheck()?;
        let mut file = self.staged.created_file()?;
        if super::windows_identity::read_version_handle(&mut file)? != self.temporary
            || super::windows_identity::volume_path(&file)? != self.staged.recovery_temp()
        {
            return Err(IdentityError::Changed);
        }
        Ok(())
    }
    pub(super) fn recheck_historical_source(&self) -> Result<(), IdentityError> {
        if let Some(source) = &self.historical_source {
            source.recheck()?;
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
        #[cfg(test)]
        if let Some(descriptor) = &self.creation_security_override {
            return descriptor;
        }
        self.staged.creation_security()
    }
    /// Test-only credential fault; never changes the original StagedSave or ACL.
    #[cfg(test)]
    pub(super) fn set_creation_security_for_test(&mut self, descriptor: SecurityDescriptor) {
        self.creation_security_override = Some(descriptor);
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
