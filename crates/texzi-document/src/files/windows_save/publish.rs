//! New targets only, with explicit final DACL policy. Existing replacement is excluded.
use super::recovery::Guard;
use super::*;
use crate::{SaveFailure, SaveFailureOutcome, Snapshot};
use std::sync::Arc;
use std::{
    fs::File,
    io,
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
};
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_RENAME_INFORMATION, FileRenameInformation, NtSetInformationFile,
};
use windows_sys::Win32::Foundation::{
    NTSTATUS, RtlNtStatusToDosError, STATUS_PENDING, STATUS_SUCCESS,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

#[derive(Debug)]
pub enum PublishIssue {
    ExistingTargetUnsupported,
    InvalidName,
    Identity(IdentityError),
    Io(io::Error),
    Owner(Error),
    VerificationMismatch,
}
/// Final security of a new name. Every temporary file remains private first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewFileSecurity {
    OwnerOnly,
    /// Restore the DACL captured from the original create_new handle, only
    /// after successful rename. Does not promise arbitrary inheritance rules.
    CreationDefault,
}
impl NewFileSecurity {
    pub(super) fn expected(self, resources: &LockedStaged) -> &SecurityDescriptor {
        match self {
            Self::OwnerOnly => resources.temporary_security(),
            Self::CreationDefault => resources.creation_security(),
        }
    }

    fn matches_published(self, resources: &LockedStaged, actual: &SecurityDescriptor) -> bool {
        let expected = self.expected(resources);
        match self {
            Self::OwnerOnly => expected == actual,
            Self::CreationDefault => expected.matches_restored_creation(actual),
        }
    }
}
impl From<io::Error> for PublishIssue {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<IdentityError> for PublishIssue {
    fn from(e: IdentityError) -> Self {
        Self::Identity(e)
    }
}
#[derive(Clone, Copy)]
enum Phase {
    BeforePermit,
    Permitted,
    Attempted,
}
#[must_use = "record the terminal result and retain or explicitly resolve recovery resources"]
pub struct PublishFailure {
    pub issue: PublishIssue,
    security_policy: NewFileSecurity,
    resources: Arc<LockedStaged>,
    phase: Phase,
    rename_failure_status: Option<NTSTATUS>,
    _guard: Option<Guard>,
}
impl std::fmt::Debug for PublishFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublishFailure")
            .field(
                "rename_failure_status",
                &self
                    .rename_failure_status
                    .map(|status| format!("0x{:08X}", status as u32)),
            )
            .field("issue", &self.issue)
            .field("operation", &self.capture().operation)
            .finish()
    }
}
impl PublishFailure {
    pub fn security_policy(&self) -> NewFileSecurity {
        self.security_policy
    }
    pub fn capture(&self) -> &SaveCapture {
        self.resources.capture()
    }
    pub fn read_retained_bytes(&self) -> io::Result<Vec<u8>> {
        self.resources.read_created_bytes()
    }
    pub fn current_path(&self) -> Result<PathBuf, IdentityError> {
        let file = self.resources.staged.staged.created_file()?;
        super::super::windows_identity::volume_path(&file)
    }
    pub fn current_version(&self) -> Result<FileVersion, IdentityError> {
        let mut file = self.resources.staged.staged.created_file()?;
        super::super::windows_identity::read_version_handle(&mut file)
    }
    /// Record a terminal result without dropping the owned exclusive handle.
    /// Candidate paths still require identity checks before any recovery action.
    pub fn record_failure(&self, store: &mut Store) -> Result<(), Error> {
        let mut paths = vec![
            self.resources.recovery_temp().to_owned(),
            self.resources.staged.target_path(),
        ];
        if let Ok(current) = self.current_path() {
            if !paths.contains(&current) {
                paths.push(current);
            }
        }
        store.finish_save_failure(
            self.capture(),
            SaveFailure {
                outcome: match self.phase {
                    Phase::BeforePermit => SaveFailureOutcome::BeforeReplace,
                    Phase::Permitted => SaveFailureOutcome::ReplaceFailed,
                    Phase::Attempted => SaveFailureOutcome::OutcomeUnknown,
                },
                conflict: matches!(
                    self.issue,
                    PublishIssue::Identity(IdentityError::Changed)
                        | PublishIssue::VerificationMismatch
                ),
                os_error: match &self.issue {
                    PublishIssue::Io(e) => e.raw_os_error(),
                    _ => None,
                },
                message: format!(
                    "new-target publication ({:?}): {:?}{}; candidate recovery paths require verification",
                    self.security_policy,
                    self.issue,
                    self.rename_failure_status
                        .map(|status| format!(" (NTSTATUS 0x{:08X})", status as u32))
                        .unwrap_or_default()
                ),
                recovery_paths: paths,
            },
        )?;
        Ok(())
    }
}
#[must_use = "obtain the owner permit or explicitly handle the still-active save"]
pub struct NewReady {
    security_policy: NewFileSecurity,
    resources: Arc<LockedStaged>,
    parent: File,
    name: Vec<u16>,
}
#[must_use = "a granted permit requires publication or an explicit failed outcome"]
pub struct NewPublication {
    security_policy: NewFileSecurity,
    guard: Option<Guard>,
    resources: Arc<LockedStaged>,
    parent: File,
    name: Vec<u16>,
    #[cfg(test)]
    before_rename: Option<Box<dyn Fn(&Path) -> io::Result<()> + Send>>,
    #[cfg(test)]
    post_rename_flush_handle: Option<Box<dyn Fn(&File) -> io::Result<File> + Send>>,
    #[cfg(test)]
    verification_format: Option<Format>,
    #[cfg(test)]
    creation_dacl_handle: Option<Box<dyn Fn(&File) -> io::Result<File> + Send>>,
}
#[must_use = "accept the verified result or explicitly record the uncertain outcome"]
pub struct PublishedNew {
    security_policy: NewFileSecurity,
    guard: Option<Guard>,
    resources: Arc<LockedStaged>,
    version: FileVersion,
    path: PathBuf,
}
pub struct CommittedNew {
    pub security_policy: NewFileSecurity,
    pub binding: FileBinding,
    pub version: FileVersion,
    pub path: PathBuf,
    pub saved_snapshot: Snapshot,
    pub binding_epoch: u64,
}

impl LockedStaged {
    /// Explicit initial policy: a new file remains restricted to owner/SYSTEM.
    /// No claim of destination inheritance or existing-file SaveAs replacement.
    pub fn prepare_new_owner_only(self) -> Result<NewReady, PublishFailure> {
        self.prepare_new(NewFileSecurity::OwnerOnly)
    }
    /// Keep private staging, then restore its original creation DACL only at
    /// the successfully published new name. Not existing-target replacement.
    pub fn prepare_new_creation_default(self) -> Result<NewReady, PublishFailure> {
        self.prepare_new(NewFileSecurity::CreationDefault)
    }
    fn prepare_new(self, security_policy: NewFileSecurity) -> Result<NewReady, PublishFailure> {
        let attempt = (|| -> Result<_, PublishIssue> {
            if self.original_security().is_some() {
                return Err(PublishIssue::ExistingTargetUnsupported);
            }
            self.recheck_files()?;
            if security_policy == NewFileSecurity::CreationDefault {
                self.creation_security().validate_creation_dacl_restore()?;
            }
            let observed = &self.staged.observed;
            let text = observed
                .target_name()
                .to_str()
                .ok_or(PublishIssue::InvalidName)?;
            let base = text.split('.').next().unwrap_or("").to_ascii_uppercase();
            let device = ["CON", "PRN", "AUX", "NUL"].contains(&base.as_str())
                || ["COM", "LPT"].iter().any(|p| {
                    base.strip_prefix(*p).is_some_and(|s| {
                        matches!(
                            s,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                    })
                });
            let name: Vec<u16> = observed.target_name().encode_wide().collect();
            if text.is_empty() || device || name.len() > 255 {
                return Err(PublishIssue::InvalidName);
            }
            Ok((observed.parent_for_new_file()?, name))
        })();
        match attempt {
            Ok((parent, name)) => Ok(NewReady {
                security_policy,
                resources: Arc::new(self),
                parent,
                name,
            }),
            Err(issue) => Err(PublishFailure {
                issue,
                security_policy,
                resources: Arc::new(self),
                phase: Phase::BeforePermit,
                rename_failure_status: None,
                _guard: None,
            }),
        }
    }
}
impl NewReady {
    pub fn security_policy(&self) -> NewFileSecurity {
        self.security_policy
    }
    pub fn capture(&self) -> &SaveCapture {
        self.resources.capture()
    }
    pub fn permit(self, store: &mut Store) -> Result<NewPublication, PublishFailure> {
        let scope = self.resources.staged.observed.save_scope();
        if store.captured_save_stage(self.capture()) != Ok(SaveStage::WritingTemp) {
            return Err(PublishFailure {
                issue: PublishIssue::Owner(Error::InvalidSave),
                security_policy: self.security_policy,
                resources: self.resources,
                phase: Phase::BeforePermit,
                rename_failure_status: None,
                _guard: None,
            });
        }
        // Retain the original handles before the owner transitions to Replacing.
        let guard = store
            .publication_recovery
            .register(self.resources.clone(), self.security_policy);
        match store.permit_bound_new(self.capture(), &scope) {
            Ok(()) => Ok(NewPublication {
                security_policy: self.security_policy,
                guard: Some(guard),
                resources: self.resources,
                parent: self.parent,
                name: self.name,
                #[cfg(test)]
                before_rename: None,
                #[cfg(test)]
                post_rename_flush_handle: None,
                #[cfg(test)]
                verification_format: None,
                #[cfg(test)]
                creation_dacl_handle: None,
            }),
            Err(e) => {
                // No permit was granted. The returned failure remains the sole
                // recovery owner; dropping the local guard must not enqueue it.
                store
                    .publication_recovery
                    .complete(self.capture().operation);
                Err(PublishFailure {
                    issue: PublishIssue::Owner(e),
                    security_policy: self.security_policy,
                    resources: self.resources,
                    phase: Phase::BeforePermit,
                    rename_failure_status: None,
                    _guard: None,
                })
            }
        }
    }
}
fn rename_new(
    file: &File,
    parent: &File,
    name: &[u16],
    rename_status: &mut Option<NTSTATUS>,
) -> io::Result<()> {
    let offset = std::mem::offset_of!(FILE_RENAME_INFORMATION, FileName);
    let bytes = name
        .len()
        .checked_mul(2)
        .ok_or_else(|| io::Error::other("rename name length overflow"))?;
    let size = offset
        .checked_add(bytes)
        .and_then(|n| n.checked_add(2))
        .ok_or_else(|| io::Error::other("rename buffer overflow"))?
        .max(std::mem::size_of::<FILE_RENAME_INFORMATION>());
    let size_u32 = u32::try_from(size).map_err(|_| io::Error::other("rename buffer too large"))?;
    let mut buffer = vec![0u64; size.div_ceil(8)];
    let header = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    let mut completion = IO_STATUS_BLOCK::default();
    completion.Anonymous.Status = STATUS_PENDING;
    // SAFETY: the aligned buffer holds the header and full UTF-16 name.
    // Both handles remain live. Staging opens this File without
    // FILE_FLAG_OVERLAPPED, so the call completes on the synchronous handle
    // before the input buffer and completion block can be dropped.
    unsafe {
        (*header).Anonymous.ReplaceIfExists = false;
        (*header).RootDirectory = parent.as_raw_handle();
        (*header).FileNameLength = bytes as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*header).FileName).cast::<u16>(),
            name.len(),
        );
        let returned = NtSetInformationFile(
            file.as_raw_handle(),
            &mut completion,
            header.cast(),
            size_u32,
            FileRenameInformation,
        );
        let status = if returned == STATUS_SUCCESS {
            completion.Anonymous.Status
        } else {
            returned
        };
        *rename_status = Some(status);
        if status != STATUS_SUCCESS {
            return Err(io::Error::from_raw_os_error(
                RtlNtStatusToDosError(status) as i32
            ));
        }
    }
    Ok(())
}
impl NewPublication {
    pub fn security_policy(&self) -> NewFileSecurity {
        self.security_policy
    }
    #[cfg(test)]
    pub(super) fn set_creation_dacl_handle(
        &mut self,
        hook: impl Fn(&File) -> io::Result<File> + Send + 'static,
    ) {
        self.creation_dacl_handle = Some(Box::new(hook));
    }
    #[cfg(test)]
    pub(super) fn set_post_rename_flush_handle(
        &mut self,
        hook: impl Fn(&File) -> io::Result<File> + Send + 'static,
    ) {
        self.post_rename_flush_handle = Some(Box::new(hook));
    }
    #[cfg(test)]
    pub(super) fn set_verification_format(&mut self, format: Format) {
        self.verification_format = Some(format);
    }
    #[cfg(test)]
    pub(super) fn set_before_rename(
        &mut self,
        hook: impl Fn(&Path) -> io::Result<()> + Send + 'static,
    ) {
        self.before_rename = Some(Box::new(hook));
    }
    pub fn capture(&self) -> &SaveCapture {
        self.resources.capture()
    }
    /// Worker operation: no Store borrow; the original exclusive file survives
    /// rename, flush and post-verification until owner acceptance or recovery.
    pub fn run(self) -> Result<PublishedNew, PublishFailure> {
        if let Err(e) = self.resources.recheck_files() {
            return Err(PublishFailure {
                issue: PublishIssue::Identity(e),
                security_policy: self.security_policy,
                resources: self.resources,
                phase: Phase::Permitted,
                rename_failure_status: None,
                _guard: self.guard,
            });
        }
        #[cfg(test)]
        if let Some(hook) = &self.before_rename {
            if let Err(e) = hook(&self.resources.staged.target_path()) {
                return Err(PublishFailure {
                    issue: PublishIssue::Io(e),
                    security_policy: self.security_policy,
                    resources: self.resources,
                    phase: Phase::Permitted,
                    rename_failure_status: None,
                    _guard: self.guard,
                });
            }
        }
        let path = self.resources.staged.target_path();
        let mut rename_status = None;
        let result = (|| -> Result<FileVersion, PublishIssue> {
            let mut file = self.resources.staged.staged.created_file()?;
            file.sync_all()?;
            rename_new(&file, &self.parent, &self.name, &mut rename_status)?;
            if self.security_policy == NewFileSecurity::CreationDefault {
                // Never broaden a sibling temporary name. Prove that the same
                // exclusive File has reached the intended final name first.
                self.resources.staged.recheck_historical_source()?;
                self.resources.staged.observed.recheck_parent()?;
                let private_version =
                    super::super::windows_identity::read_version_handle(&mut file)?;
                if super::super::windows_identity::volume_path(&file)? != path
                    || private_version.identity != self.resources.staged.temporary_identity()
                    || private_version.bytes.as_ref() != self.resources.payload()
                    || &private_version.security != self.resources.temporary_security()
                    || self.capture().snapshot.body.profile.file_format
                        != Some(self.resources.format())
                {
                    return Err(PublishIssue::VerificationMismatch);
                }
                #[cfg(test)]
                let dacl_handle = self
                    .creation_dacl_handle
                    .as_ref()
                    .map(|hook| hook(&file))
                    .transpose()?;
                #[cfg(test)]
                let dacl_file = dacl_handle.as_ref().unwrap_or(&*file);
                #[cfg(not(test))]
                let dacl_file = &*file;
                self.resources
                    .creation_security()
                    .restore_creation_dacl_to_published_new(dacl_file)?;
            }
            #[cfg(test)]
            let flush_handle = self
                .post_rename_flush_handle
                .as_ref()
                .map(|hook| hook(&file))
                .transpose()?;
            #[cfg(test)]
            let flush_file = flush_handle.as_ref().unwrap_or(&*file);
            #[cfg(not(test))]
            let flush_file = &*file;
            flush_file.sync_all()?;
            self.resources.staged.recheck_historical_source()?;
            self.resources.staged.observed.recheck_parent()?;
            // Query the actual post-rename identity and bytes, never assign the
            // old reservation key or cached temporary ID as the final identity.
            let version = super::super::windows_identity::read_version_handle(&mut file)?;
            let format = self.resources.format();
            #[cfg(test)]
            let format = self.verification_format.unwrap_or(format);
            if super::super::windows_identity::volume_path(&file)? != path
                || version.identity != self.resources.staged.temporary_identity()
                || version.bytes.as_ref() != self.resources.payload()
                || !self
                    .security_policy
                    .matches_published(&self.resources, &version.security)
                || self.capture().snapshot.body.profile.file_format != Some(format)
            {
                return Err(PublishIssue::VerificationMismatch);
            }
            Ok(version)
        })();
        match result {
            Ok(version) => Ok(PublishedNew {
                security_policy: self.security_policy,
                guard: self.guard,
                resources: self.resources,
                version,
                path,
            }),
            Err(issue) => Err(PublishFailure {
                issue,
                security_policy: self.security_policy,
                resources: self.resources,
                _guard: self.guard,
                rename_failure_status: rename_status.filter(|&status| status != STATUS_SUCCESS),
                phase: if rename_status.is_some() {
                    Phase::Attempted
                } else {
                    Phase::Permitted
                },
            }),
        }
    }
}
impl PublishedNew {
    pub fn security_policy(&self) -> NewFileSecurity {
        self.security_policy
    }
    #[cfg(test)]
    pub(super) fn set_handoff_path_for_test(&mut self, path: PathBuf) {
        self.path = path;
    }
    pub fn capture(&self) -> &SaveCapture {
        self.resources.capture()
    }
    pub fn identity(&self) -> FileIdentity {
        self.version.identity
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn accept(self, store: &mut Store) -> Result<CommittedNew, PublishFailure> {
        let snapshot = self.capture().snapshot.clone();
        let format = self.resources.format();
        let anchor = (|| -> Result<_, PublishIssue> {
            self.resources.staged.recheck_historical_source()?;
            self.resources.staged.observed.recheck_parent()?;
            let mut file = self.resources.staged.staged.created_file()?;
            if !self
                .security_policy
                .matches_published(&self.resources, &self.version.security)
                || super::super::windows_identity::read_version_handle(&mut file)? != self.version
            {
                return Err(PublishIssue::VerificationMismatch);
            }
            Ok(super::super::windows_identity::OpenAnchor::from_retained(
                &file, &self.path,
            )?)
        })();
        let anchor = match anchor {
            Ok(anchor) => anchor,
            Err(issue) => {
                return Err(PublishFailure {
                    issue,
                    security_policy: self.security_policy,
                    resources: self.resources,
                    phase: Phase::Attempted,
                    rename_failure_status: None,
                    _guard: self.guard,
                });
            }
        };
        match store.accept_published_new(self.capture(), self.version.identity.target_key()) {
            Ok(binding_epoch) => Ok(CommittedNew {
                security_policy: self.security_policy,
                binding: FileBinding {
                    imported: snapshot.clone(),
                    original: self.version.bytes.clone(),
                    format,
                    disk: Some(crate::files::DiskBinding {
                        anchor,
                        parent: self.resources.staged.observed.parent_identity(),
                        version: self.version.clone(),
                        path: self.path.clone(),
                        epoch: binding_epoch,
                    }),
                },
                version: self.version,
                path: self.path,
                saved_snapshot: snapshot,
                binding_epoch,
            }),
            Err(e) => Err(PublishFailure {
                issue: PublishIssue::Owner(e),
                security_policy: self.security_policy,
                resources: self.resources,
                phase: Phase::Attempted,
                rename_failure_status: None,
                _guard: self.guard,
            }),
        }
    }
}
