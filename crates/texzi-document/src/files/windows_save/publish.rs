//! New targets only, owner/SYSTEM-only policy. Existing replacement is excluded.
use super::recovery::Guard;
use super::*;
use crate::{SaveFailure, SaveFailureOutcome, Snapshot};
use std::sync::Arc;
use std::{
    fs::File,
    io,
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_RENAME_INFO, FileRenameInfo, SetFileInformationByHandle,
};

#[derive(Debug)]
pub enum PublishIssue {
    ExistingTargetUnsupported,
    InvalidName,
    Identity(IdentityError),
    Io(io::Error),
    Owner(Error),
    VerificationMismatch,
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
    resources: Arc<LockedStaged>,
    phase: Phase,
    _guard: Option<Guard>,
}
impl std::fmt::Debug for PublishFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublishFailure")
            .field("issue", &self.issue)
            .field("operation", &self.capture().operation)
            .finish()
    }
}
impl PublishFailure {
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
                    "new-target publication: {:?}; candidate recovery paths require verification",
                    self.issue
                ),
                recovery_paths: paths,
            },
        )?;
        Ok(())
    }
}
#[must_use = "obtain the owner permit or explicitly handle the still-active save"]
pub struct NewReady {
    resources: Arc<LockedStaged>,
    parent: File,
    name: Vec<u16>,
}
#[must_use = "a granted permit requires publication or an explicit failed outcome"]
pub struct NewPublication {
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
}
#[must_use = "accept the verified result or explicitly record the uncertain outcome"]
pub struct PublishedNew {
    guard: Option<Guard>,
    resources: Arc<LockedStaged>,
    version: FileVersion,
    path: PathBuf,
}
pub struct CommittedNew {
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
        let attempt = (|| -> Result<_, PublishIssue> {
            if self.original_security().is_some() {
                return Err(PublishIssue::ExistingTargetUnsupported);
            }
            self.recheck_files()?;
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
                resources: Arc::new(self),
                parent,
                name,
            }),
            Err(issue) => Err(PublishFailure {
                issue,
                resources: Arc::new(self),
                phase: Phase::BeforePermit,
                _guard: None,
            }),
        }
    }
}
impl NewReady {
    pub fn capture(&self) -> &SaveCapture {
        self.resources.capture()
    }
    pub fn permit(self, store: &mut Store) -> Result<NewPublication, PublishFailure> {
        let scope = self.resources.staged.observed.save_scope();
        if store.captured_save_stage(self.capture()) != Ok(SaveStage::WritingTemp) {
            return Err(PublishFailure {
                issue: PublishIssue::Owner(Error::InvalidSave),
                resources: self.resources,
                phase: Phase::BeforePermit,
                _guard: None,
            });
        }
        // Retain the original handles before the owner transitions to Replacing.
        let guard = store.publication_recovery.register(self.resources.clone());
        match store.permit_bound_new(self.capture(), &scope) {
            Ok(()) => Ok(NewPublication {
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
            }),
            Err(e) => {
                // No permit was granted. The returned failure remains the sole
                // recovery owner; dropping the local guard must not enqueue it.
                store
                    .publication_recovery
                    .complete(self.capture().operation);
                Err(PublishFailure {
                    issue: PublishIssue::Owner(e),
                    resources: self.resources,
                    phase: Phase::BeforePermit,
                    _guard: None,
                })
            }
        }
    }
}
fn rename_new(file: &File, parent: &File, name: &[u16], attempted: &mut bool) -> io::Result<()> {
    let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    let bytes = name
        .len()
        .checked_mul(2)
        .ok_or_else(|| io::Error::other("rename name length overflow"))?;
    let size = offset
        .checked_add(bytes)
        .and_then(|n| n.checked_add(2))
        .ok_or_else(|| io::Error::other("rename buffer overflow"))?
        .max(std::mem::size_of::<FILE_RENAME_INFO>());
    let size_u32 = u32::try_from(size).map_err(|_| io::Error::other("rename buffer too large"))?;
    let mut buffer = vec![0u64; size.div_ceil(8)];
    let header = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: aligned, initialized allocation is large enough for the structure
    // and variable UTF16 name. Both handles remain owned/live throughout the call.
    unsafe {
        (*header).Anonymous.ReplaceIfExists = false;
        (*header).RootDirectory = parent.as_raw_handle();
        (*header).FileNameLength = bytes as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*header).FileName).cast::<u16>(),
            name.len(),
        );
        *attempted = true;
        if SetFileInformationByHandle(
            file.as_raw_handle(),
            FileRenameInfo,
            header.cast(),
            size_u32,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
impl NewPublication {
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
                resources: self.resources,
                phase: Phase::Permitted,
                _guard: self.guard,
            });
        }
        #[cfg(test)]
        if let Some(hook) = &self.before_rename {
            if let Err(e) = hook(&self.resources.staged.target_path()) {
                return Err(PublishFailure {
                    issue: PublishIssue::Io(e),
                    resources: self.resources,
                    phase: Phase::Permitted,
                    _guard: self.guard,
                });
            }
        }
        let path = self.resources.staged.target_path();
        let mut attempted = false;
        let result = (|| -> Result<FileVersion, PublishIssue> {
            let mut file = self.resources.staged.staged.created_file()?;
            file.sync_all()?;
            rename_new(&file, &self.parent, &self.name, &mut attempted)?;
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
                || &version.security != self.resources.temporary_security()
                || self.capture().snapshot.body.profile.file_format != Some(format)
            {
                return Err(PublishIssue::VerificationMismatch);
            }
            Ok(version)
        })();
        match result {
            Ok(version) => Ok(PublishedNew {
                guard: self.guard,
                resources: self.resources,
                version,
                path,
            }),
            Err(issue) => Err(PublishFailure {
                issue,
                resources: self.resources,
                _guard: self.guard,
                phase: if attempted {
                    Phase::Attempted
                } else {
                    Phase::Permitted
                },
            }),
        }
    }
}
impl PublishedNew {
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
        let binding = FileBinding {
            imported: snapshot.clone(),
            original: self.version.bytes.clone(),
            format,
        };
        match store.accept_published_new(self.capture(), self.version.identity.target_key()) {
            Ok(binding_epoch) => Ok(CommittedNew {
                binding,
                version: self.version,
                path: self.path,
                saved_snapshot: snapshot,
                binding_epoch,
            }),
            Err(e) => Err(PublishFailure {
                issue: PublishIssue::Owner(e),
                resources: self.resources,
                phase: Phase::Attempted,
                _guard: self.guard,
            }),
        }
    }
}
