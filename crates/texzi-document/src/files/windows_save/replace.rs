//! Filesystem side of Current-save. Retain the original staging handle while
//! ReplaceFileW consumes a separate, verified copy.
use super::recovery::Guard;
use super::*;
use crate::{
    SaveFailure, SaveFailureOutcome,
    files::{
        DiskBinding,
        windows_identity::{self, OpenAnchor},
    },
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_DISPOSITION_INFO, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    FileDispositionInfo, ReplaceFileW, SetFileInformationByHandle, WRITE_DAC, WRITE_OWNER,
};

pub(super) struct ReplacementWorkspace {
    backup: ObservedTarget,
    source: PathBuf,
    source_anchor: OpenAnchor,
    source_version: FileVersion,
    #[cfg(test)]
    before_replace: Option<Box<dyn Fn(&Path) + Send + Sync>>,
}

pub(super) struct ReplacedTarget {
    pub(super) file: File,
    pub(super) version: FileVersion,
    backup_file: File,
}

pub struct PreparedReplacement {
    staged: Arc<LockedStaged>,
    workspace: Arc<ReplacementWorkspace>,
}

pub struct PermittedReplacement {
    ready: PreparedReplacement,
    guard: Guard,
}

pub struct VerifiedReplacement {
    permitted: PermittedReplacement,
    result: ReplacedTarget,
}

pub struct ReplacementFailure {
    pub issue: PublishIssue,
    staged: Arc<LockedStaged>,
    paths: Vec<PathBuf>,
    permitted: bool,
    attempted: bool,
    _guard: Option<Guard>,
}

impl std::fmt::Debug for ReplacementFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplacementFailure")
            .field("issue", &self.issue)
            .field("operation", &self.staged.capture().operation)
            .field("attempted", &self.attempted)
            .field("recovery_paths", &self.paths)
            .finish()
    }
}

impl ReplacementFailure {
    pub fn record_failure(&self, store: &mut Store) -> Result<(), Error> {
        store.finish_save_failure(
            self.staged.capture(),
            SaveFailure {
                outcome: if self.attempted {
                    SaveFailureOutcome::OutcomeUnknown
                } else if self.permitted {
                    SaveFailureOutcome::ReplaceFailed
                } else {
                    SaveFailureOutcome::BeforeReplace
                },
                conflict: self.attempted
                    || matches!(self.issue, PublishIssue::Identity(IdentityError::Changed)),
                os_error: match &self.issue {
                    PublishIssue::Io(error) => error.raw_os_error(),
                    _ => None,
                },
                message: format!("{self:?}"),
                recovery_paths: self.paths.clone(),
            },
        )
    }
}

impl LockedStaged {
    /// Prepare publication only for the actual current binding. An observed
    /// occupied Save As destination cannot borrow the current-save capability.
    pub fn prepare_replacement(self) -> Result<PreparedReplacement, ReplacementFailure> {
        let staged = Arc::new(self);
        let mut paths = vec![
            staged.recovery_temp().to_owned(),
            staged.staged.target_path(),
        ];
        match ReplacementWorkspace::prepare(&staged, &mut paths) {
            Ok(workspace) => Ok(PreparedReplacement {
                staged,
                workspace: Arc::new(workspace),
            }),
            Err(issue) => Err(ReplacementFailure {
                issue,
                staged,
                paths,
                permitted: false,
                attempted: false,
                _guard: None,
            }),
        }
    }
}

impl PreparedReplacement {
    pub fn permit(self, store: &mut Store) -> Result<PermittedReplacement, ReplacementFailure> {
        let scope = self.staged.staged.observed.save_scope();
        if let Err(error) = store.permit_bound_replacement(self.staged.capture(), &scope) {
            return Err(self.fail(PublishIssue::Owner(error), false, None));
        }
        let guard = store
            .publication_recovery
            .register_replacement(self.staged.clone(), self.workspace.clone());
        Ok(PermittedReplacement { ready: self, guard })
    }

    fn fail(
        self,
        issue: PublishIssue,
        attempted: bool,
        guard: Option<Guard>,
    ) -> ReplacementFailure {
        ReplacementFailure {
            issue,
            paths: vec![
                self.staged.recovery_temp().to_owned(),
                self.staged.staged.target_path(),
                self.workspace.backup_path(),
                self.workspace.source_path().to_owned(),
            ],
            staged: self.staged,
            permitted: guard.is_some(),
            attempted,
            _guard: guard,
        }
    }
}

impl PermittedReplacement {
    pub fn run(self) -> Result<VerifiedReplacement, ReplacementFailure> {
        let mut attempted = false;
        match self.ready.workspace.run(&self.ready.staged, &mut attempted) {
            Ok(result) => Ok(VerifiedReplacement {
                permitted: self,
                result,
            }),
            Err(issue) => Err(self.ready.fail(issue, attempted, Some(self.guard))),
        }
    }
}

impl VerifiedReplacement {
    pub fn accept(mut self, store: &mut Store) -> Result<FileBinding, ReplacementFailure> {
        let PermittedReplacement { ready, guard } = self.permitted;
        let result = (|| -> Result<FileBinding, PublishIssue> {
            let anchor = self.result.recheck(&ready.staged, &ready.workspace)?;
            let capture = ready.staged.capture();
            let epoch = store
                .accept_published_replacement(capture, self.result.version.identity.target_key())
                .map_err(PublishIssue::Owner)?;
            let version = self.result.version;
            Ok(FileBinding {
                imported: capture.snapshot.clone(),
                original: version.bytes.clone(),
                format: ready.staged.format(),
                disk: Some(DiskBinding {
                    version,
                    anchor,
                    parent: ready.staged.staged.observed.parent_identity(),
                    path: ready.staged.staged.target_path(),
                    epoch,
                }),
            })
        })();
        match result {
            Ok(binding) => {
                retire_staging_copy(&ready.staged);
                Ok(binding)
            }
            Err(issue) => Err(ready.fail(issue, true, Some(guard))),
        }
    }
}

impl ReplacementWorkspace {
    fn prepare(
        staged: &LockedStaged,
        recovery_paths: &mut Vec<PathBuf>,
    ) -> Result<Self, PublishIssue> {
        let original = staged
            .staged
            .observed
            .version()
            .ok_or(PublishIssue::ExistingTargetUnsupported)?;
        let anchor = staged
            .staged
            ._source_anchor
            .as_ref()
            .ok_or(PublishIssue::ExistingTargetUnsupported)?;
        if staged.staged.historical_source.is_some() || !anchor.retains(original.identity)? {
            return Err(PublishIssue::ExistingTargetUnsupported);
        }
        staged.recheck_files()?;
        let target = staged.staged.target_path();
        let parent = target.parent().ok_or(PublishIssue::InvalidName)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
        let nonce = u128::from_le_bytes(random);
        let root = parent.join(format!(".texzi-recovery-{nonce:032x}"));
        let source = parent.join(format!(".texzi-publish-{nonce:032x}.tmp"));
        SecurityDescriptor::create_private_directory(&root)?;
        recovery_paths.push(root.join("original.bak"));
        let backup = ObservedTarget::inspect(&root.join("original.bak"))?;
        if backup.version().is_some()
            || backup.parent_identity().volume != original.identity.volume
            || backup.resolved_path().parent() != Some(root.as_path())
        {
            return Err(PublishIssue::VerificationMismatch);
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .access_mode(FILE_GENERIC_READ | FILE_GENERIC_WRITE | WRITE_DAC | WRITE_OWNER)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .create_new(true)
            .open(&source)?;
        recovery_paths.push(source.clone());
        // Keep the publication copy in the target's actual parent so Windows
        // resolves inherited ACEs in the same namespace before and after rename.
        // Verify the empty file's permissions before writing protected content.
        if SecurityDescriptor::read(&file)? != original.security {
            original.security.apply_to_new_file(&file)?;
        }
        let security = SecurityDescriptor::read(&file)?;
        if !original.security.matches_restored_creation(&security) {
            return Err(PublishIssue::VerificationMismatch);
        }
        file.write_all(staged.payload())?;
        file.sync_all()?;
        let source_version = windows_identity::read_version_handle(&mut file)?;
        let source_anchor = OpenAnchor::from_retained(&file, &source)?;
        backup.recheck()?;
        if source_version.security != security || source_version.bytes.as_ref() != staged.payload()
        {
            return Err(PublishIssue::VerificationMismatch);
        }
        Ok(Self {
            backup,
            source,
            source_anchor,
            source_version,
            #[cfg(test)]
            before_replace: None,
        })
    }

    pub(super) fn backup_path(&self) -> PathBuf {
        self.backup.resolved_path()
    }

    pub(super) fn source_path(&self) -> &Path {
        &self.source
    }

    pub(super) fn run(
        &self,
        staged: &LockedStaged,
        attempted: &mut bool,
    ) -> Result<ReplacedTarget, PublishIssue> {
        let observed = &staged.staged.observed;
        let expected = observed
            .version()
            .ok_or(PublishIssue::ExistingTargetUnsupported)?;
        let target = observed.resolved_path();
        let backup = self.backup.resolved_path();
        let mut victim = observed.hold_replacement_victim()?;
        let source = ObservedTarget::inspect(&self.source)?;
        if source.version() != Some(&self.source_version)
            || !self.source_anchor.retains(self.source_version.identity)?
        {
            return Err(PublishIssue::VerificationMismatch);
        }
        self.backup.recheck()?;
        staged.recheck_files()?;
        #[cfg(test)]
        if let Some(before_replace) = &self.before_replace {
            before_replace(&target);
        }
        *attempted = true;
        replace_file(&target, &self.source, &backup)?;

        // The OS backup is the victim of the actual rename. A concurrent
        // replacement is preserved there and cannot be accepted as our baseline.
        if windows_identity::volume_path(&victim)? != backup
            || windows_identity::read_version_handle(&mut victim)? != *expected
        {
            return Err(PublishIssue::VerificationMismatch);
        }
        let saved = ObservedTarget::inspect(&target)?;
        let backup_observed = ObservedTarget::inspect(&backup)?;
        if saved.parent_identity() != observed.parent_identity()
            || saved.parent_security() != observed.parent_security()
            || backup_observed.parent_identity() != self.backup.parent_identity()
            || backup_observed.version() != Some(expected)
        {
            return Err(PublishIssue::VerificationMismatch);
        }
        let version = saved
            .version()
            .ok_or(PublishIssue::VerificationMismatch)?
            .clone();
        if version.identity != self.source_version.identity
            || version.bytes.as_ref() != staged.payload()
            || !expected
                .security
                .matches_restored_creation(&version.security)
        {
            return Err(PublishIssue::VerificationMismatch);
        }
        let file = saved.hold_existing_read()?;
        let backup_file = backup_observed.hold_existing_read()?;
        observed.recheck_parent()?;
        self.backup.recheck_parent()?;
        Ok(ReplacedTarget {
            file,
            version,
            backup_file,
        })
    }
}

fn retire_staging_copy(staged: &LockedStaged) {
    let Ok(file) = staged.staged.staged.created_file() else {
        return;
    };
    if !windows_identity::volume_path(&file).is_ok_and(|path| path == staged.recovery_temp())
        || !windows_identity::identity(&file)
            .is_ok_and(|identity| identity == staged.staged.temporary_identity())
    {
        return;
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: the original exclusive create_new handle is verified above. This
    // applies only to that staged copy after Store accepted the saved version.
    // If deletion fails, the copy remains available at its existing name.
    unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            std::ptr::from_ref(&disposition).cast(),
            std::mem::size_of_val(&disposition) as u32,
        );
    }
}

impl ReplacedTarget {
    pub(super) fn recheck(
        &mut self,
        staged: &LockedStaged,
        workspace: &ReplacementWorkspace,
    ) -> Result<OpenAnchor, PublishIssue> {
        let observed = &staged.staged.observed;
        let path = observed.resolved_path();
        if windows_identity::read_version_handle(&mut self.file)? != self.version
            || windows_identity::volume_path(&self.file)? != path
            || Some(&windows_identity::read_version_handle(
                &mut self.backup_file,
            )?) != observed.version()
            || windows_identity::volume_path(&self.backup_file)? != workspace.backup_path()
        {
            return Err(PublishIssue::VerificationMismatch);
        }
        workspace.backup.recheck_parent()?;
        observed.recheck_parent()?;
        Ok(OpenAnchor::from_retained(&self.file, &path)?)
    }
}

fn replace_file(target: &Path, source: &Path, backup: &Path) -> io::Result<()> {
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>()
    };
    let target = wide(target);
    let source = wide(source);
    let backup = wide(backup);
    // SAFETY: three validated paths owned by this save, with live terminated
    // buffers and reserved arguments null. Flags do not suppress ACL errors.
    if unsafe {
        ReplaceFileW(
            target.as_ptr(),
            source.as_ptr(),
            backup.as_ptr(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BoundaryPolicy, Edit, ObserverScope, SaveResult, Transaction};
    use std::fs;
    use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

    fn ready(path: &Path, store: &mut Store, binding: &FileBinding) -> PreparedReplacement {
        ObservedTarget::inspect(path)
            .unwrap()
            .prepare_current(binding, store)
            .unwrap()
            .begin(store)
            .unwrap()
            .write()
            .unwrap()
            .lock_temporary()
            .unwrap()
            .prepare_replacement()
            .unwrap()
    }

    #[test]
    fn current_replace_preserves_acl_and_reopens_exact_new_bytes() {
        let (path, mut store, initial, binding) = super::super::current_tests::fixture();
        let expected = binding.disk.as_ref().unwrap().version.clone();
        let mut profile = initial.body.profile;
        profile.file_format = Some(Format {
            encoding: crate::files::Encoding::Utf16Le,
            bom: true,
        });
        let edited = store
            .set_serialization(initial.doc, initial.revision, profile, None)
            .unwrap()
            .snapshot;
        let ready = ready(&path, &mut store, &binding);
        let backup = ready.workspace.backup_path();
        let temp = ready.staged.recovery_temp().to_owned();
        let committed = ready
            .permit(&mut store)
            .unwrap()
            .run()
            .unwrap()
            .accept(&mut store)
            .unwrap();
        assert_eq!(committed.imported.token, edited.token);
        assert!(!store.dirty(initial.doc).unwrap());
        drop(committed);
        drop(binding);
        assert_eq!(fs::read(&path).unwrap(), [0xff, 0xfe, 65, 0, 13, 0, 10, 0]);
        assert_eq!(fs::read(&backup).unwrap(), expected.bytes.as_ref());
        let verified = ObservedTarget::inspect(&path).unwrap();
        assert_eq!(verified.version().unwrap().security, expected.security);
        assert!(!temp.exists());
    }

    #[test]
    fn current_replace_sharing_failure_retains_original_and_staged_bytes() {
        let (path, mut store, initial, binding) = super::super::current_tests::fixture();
        let edited = store
            .submit(Transaction {
                doc: initial.doc,
                base: initial.revision,
                edits: vec![Edit::input(&initial, 0..0, "N")],
                boundary: BoundaryPolicy::Grapheme,
                cancelled: false,
            })
            .unwrap()
            .snapshot;
        let ready = ready(&path, &mut store, &binding);
        let backup = ready.workspace.backup_path();
        let deny_delete = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .unwrap();
        let failure = match ready.permit(&mut store).unwrap().run() {
            Ok(_) => panic!("replaced a target with conflicting sharing"),
            Err(failure) => failure,
        };
        assert!(failure.attempted);
        assert!(matches!(failure.issue, PublishIssue::Io(_)));
        failure.record_failure(&mut store).unwrap();
        assert!(store.dirty(initial.doc).unwrap());
        assert_eq!(store.snapshot(initial.doc).unwrap().token, edited.token);
        assert!(!backup.exists());
        assert_eq!(fs::read(&path).unwrap(), b"A\r\n");
        assert_eq!(failure.staged.read_created_bytes().unwrap(), b"NA\r\n");
        drop(deny_delete);
    }

    #[test]
    fn current_replace_retains_competing_disk_version_and_keeps_dirty() {
        let (path, mut store, initial, binding) = super::super::current_tests::fixture();
        let edited = store
            .submit(Transaction {
                doc: initial.doc,
                base: initial.revision,
                edits: vec![Edit::input(&initial, 0..0, "N")],
                boundary: BoundaryPolicy::Grapheme,
                cancelled: false,
            })
            .unwrap()
            .snapshot;
        let mut ready = ready(&path, &mut store, &binding);
        let backup = ready.workspace.backup_path();
        let moved = path.with_file_name("moved-original.txt");
        let moved_by_worker = moved.clone();
        let race_path = path.clone();
        Arc::get_mut(&mut ready.workspace).unwrap().before_replace = Some(Box::new(move |_| {
            fs::rename(&race_path, &moved_by_worker).expect("rotate the fixture's original name");
            fs::write(&race_path, b"external").expect("publish competing fixture bytes");
        }));
        let failure = match ready.permit(&mut store).unwrap().run() {
            Ok(_) => panic!("competing disk version was accepted"),
            Err(failure) => failure,
        };
        assert!(failure.attempted);
        assert!(matches!(failure.issue, PublishIssue::VerificationMismatch));
        failure.record_failure(&mut store).unwrap();
        let (observed, _) = store
            .observe_and_subscribe(initial.doc, ObserverScope::new(vec![]), 8)
            .unwrap();
        let metadata = &observed.state.global.metadata;
        assert_eq!(metadata.baseline.token, initial.token);
        assert_eq!(observed.state.global.document.token, edited.token);
        assert!(metadata.dirty);
        assert_eq!(
            metadata.save.last_terminal.as_ref().unwrap().result,
            SaveResult::OutcomeUnknown
        );
        assert!(
            metadata.attention.recoveries[0]
                .detail
                .recovery_paths
                .contains(&backup)
        );
        assert_eq!(fs::read(backup).unwrap(), b"external");
        assert_eq!(fs::read(moved).unwrap(), b"A\r\n");
        assert_eq!(fs::read(path).unwrap(), b"NA\r\n");
        assert_eq!(failure.staged.read_created_bytes().unwrap(), b"NA\r\n");
    }
}
