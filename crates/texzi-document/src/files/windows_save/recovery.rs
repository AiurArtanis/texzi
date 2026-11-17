//! Process-local ownership after a publication permit. No crash journal or rollback.
use super::*;
use crate::{SaveFailure, SaveFailureOutcome};
use std::{
    collections::BTreeMap,
    io::{self, Read, Seek, SeekFrom},
    sync::{Arc, Mutex, Weak},
};

#[derive(Default)]
pub(crate) struct Registry(Mutex<BTreeMap<u64, Entry>>);
struct Entry {
    resources: Arc<LockedStaged>,
    security_policy: NewFileSecurity,
    dropped: bool,
    recorded: bool,
}
pub(super) struct Guard {
    registry: Weak<Registry>,
    operation: u64,
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            if let Some(entry) = registry
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(&self.operation)
            {
                entry.dropped = true;
            }
        }
    }
}
impl Registry {
    pub(super) fn register(
        self: &Arc<Self>,
        resources: Arc<LockedStaged>,
        security_policy: NewFileSecurity,
    ) -> Guard {
        let operation = resources.capture().operation;
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // Only the owner can grant a permit, once per monotonically allocated operation.
        assert!(!entries.contains_key(&operation));
        entries.insert(
            operation,
            Entry {
                resources,
                security_policy,
                dropped: false,
                recorded: false,
            },
        );
        Guard {
            registry: Arc::downgrade(self),
            operation,
        }
    }
    pub(crate) fn recorded(&self, operation: u64) {
        if let Some(entry) = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&operation)
        {
            entry.recorded = true;
        }
    }
    pub(crate) fn complete(&self, operation: u64) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&operation);
    }
    fn dropped(&self) -> Vec<(Arc<LockedStaged>, bool, NewFileSecurity)> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|e| e.dropped)
            .map(|e| (e.resources.clone(), e.recorded, e.security_policy))
            .collect()
    }
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// A retained original file and parent handle, not a path-based recovery permit.
/// Dropping this receipt does not release the Store's retained copy.
pub struct PublicationRecovery {
    resources: Arc<LockedStaged>,
    security_policy: NewFileSecurity,
    pub terminal_recorded: bool,
    pub owner_error: Option<Error>,
}
impl PublicationRecovery {
    /// Intended final policy, not proof it was applied or a recovery permit.
    pub fn security_policy(&self) -> NewFileSecurity {
        self.security_policy
    }
    pub fn capture(&self) -> &SaveCapture {
        self.resources.capture()
    }
    pub fn candidate_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![
            self.resources.recovery_temp().to_owned(),
            self.resources.staged.target_path(),
        ];
        if let Ok(path) = self.current_path() {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        paths
    }
    pub fn current_path(&self) -> Result<PathBuf, IdentityError> {
        let file = self.resources.staged.staged.created_file_recovery();
        super::super::windows_identity::volume_path(&file)
    }
    /// Inspection only. A poisoned worker lock never authorizes publication again.
    pub fn read_retained_bytes(&self) -> io::Result<Vec<u8>> {
        let mut file = self.resources.staged.staged.created_file_recovery();
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}
impl Store {
    /// Owner-thread reconciliation; Drop itself performs no IO or Store mutation.
    /// Repeated calls are idempotent after the terminal result was recorded.
    pub fn drain_dropped_publications(&mut self) -> Vec<PublicationRecovery> {
        self.publication_recovery
            .dropped()
            .into_iter()
            .map(|(resources, recorded, security_policy)| {
                let mut notice = PublicationRecovery {
                    resources,
                    security_policy,
                    terminal_recorded: recorded,
                    owner_error: None,
                };
                if !recorded {
                    let result = self.captured_save_stage(notice.capture()).and_then(|stage| {
                        let before = matches!(stage,
                            SaveStage::Encoding | SaveStage::WritingTemp | SaveStage::ReadyToReplace);
                        self.finish_save_failure(notice.capture(), SaveFailure {
                            outcome: if before { SaveFailureOutcome::BeforeReplace }
                                else { SaveFailureOutcome::OutcomeUnknown },
                            conflict: !before,
                            os_error: None,
                            message: format!("publication result dropped ({security_policy:?}); inspect retained original handle before recovery"),
                            recovery_paths: notice.candidate_paths(),
                        })
                    });
                    match result {
                        Ok(()) => {
                            notice.terminal_recorded = true;
                        }
                        Err(error) => notice.owner_error = Some(error),
                    }
                }
                notice
            })
            .collect()
    }

    /// Recover receipts even if a previous caller lost its returned Vec.
    pub fn retained_publication_recoveries(&self) -> Vec<PublicationRecovery> {
        self.publication_recovery
            .dropped()
            .into_iter()
            .map(
                |(resources, recorded, security_policy)| PublicationRecovery {
                    resources,
                    security_policy,
                    terminal_recorded: recorded,
                    owner_error: None,
                },
            )
            .collect()
    }
    /// Release only registry ownership after explicit owner acknowledgement.
    /// Does not delete files, clear conflicts, unlock save scopes, or change baseline.
    pub fn acknowledge_publication_recovery(&mut self, operation: u64) -> Result<(), Error> {
        let mut entries = self
            .publication_recovery
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !entries
            .get(&operation)
            .is_some_and(|e| e.dropped && e.recorded)
        {
            return Err(Error::InvalidSave);
        }
        entries.remove(&operation);
        Ok(())
    }
}
