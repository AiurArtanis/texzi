//! Read-only captured open. Store identity/baseline attachment is a separate step.
use super::*;
use crate::files::{Encoding, FileBinding, FileError, Format, LoadedBytes};

#[derive(Debug)]
pub enum AttachIssue {
    Identity(IdentityError),
    Owner(crate::Error),
    ContinuityUnproven,
}
pub struct AttachFailure {
    pub issue: AttachIssue,
    pub capture: OpenedExisting,
}
impl std::fmt::Debug for AttachFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttachFailure")
            .field("issue", &self.issue)
            .finish()
    }
}
pub enum OpenOutcome {
    /// Locate only: requested disk decoding/format was not applied to this Doc.
    /// No binding for the current (possibly dirty) token is manufactured here.
    Existing { snapshot: crate::Snapshot },
    Imported {
        snapshot: crate::Snapshot,
        binding: FileBinding,
    },
}
/// Same-object lifetime witness, not a content lock or saved version witness.
pub(crate) struct OpenAnchor {
    file: File,
    id: FileIdentity,
}
impl OpenAnchor {
    pub(crate) fn try_clone(&self) -> Result<Self, IdentityError> {
        Ok(Self {
            file: self.file.try_clone()?,
            id: self.id,
        })
    }
    pub(crate) fn retains(&self, id: FileIdentity) -> Result<bool, IdentityError> {
        Ok(self.id == id && identity(&self.file)? == id)
    }
    /// Caller retains the source file's restrictive sharing during this handoff.
    pub(crate) fn from_retained(file: &File, path: &Path) -> Result<Self, IdentityError> {
        let id = identity(file)?;
        let anchor = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        check_kind(&anchor, false)?;
        if identity(&anchor)? != id || volume_path(file)? != path || volume_path(&anchor)? != path {
            return Err(IdentityError::Changed);
        }
        Ok(Self { file: anchor, id })
    }
    pub(crate) fn key(&self) -> crate::TargetKey {
        self.id.target_key()
    }
    pub(crate) fn same_object(&self, other: &Self) -> Result<bool, IdentityError> {
        Ok(identity(&self.file)? == self.id
            && identity(&other.file)? == other.id
            && self.id == other.id)
    }
}

#[derive(Debug)]
pub enum OpenIssue {
    Missing,
    Identity(IdentityError),
    Decode(FileError),
}
pub struct OpenFailure {
    pub issue: OpenIssue,
    pub observed: ObservedTarget,
}
impl std::fmt::Debug for OpenFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenFailure")
            .field("issue", &self.issue)
            .finish()
    }
}
/// Keeps the same file handle and pinned parent through inspection and decoding.
/// This is not a Store binding, a saved baseline, or a reusable publication permit.
pub struct OpenedExisting {
    file: File,
    observed: ObservedTarget,
    version: FileVersion,
    loaded: LoadedBytes,
}
enum Choice {
    Detect(Option<Encoding>),
    Exact(Format),
}
impl ObservedTarget {
    /// Sharing permits other ordinary readers, excludes ordinary write/delete
    /// handles, and also fails if an already-open writer conflicts with this share.
    /// No fallback to a less restrictive share mode is performed.
    pub fn open_existing(self, explicit: Option<Encoding>) -> Result<OpenedExisting, OpenFailure> {
        self.capture_existing(Choice::Detect(explicit))
    }
    /// Explicit signature policy can preserve an initial literal FEFF as text.
    pub fn open_existing_exact(self, format: Format) -> Result<OpenedExisting, OpenFailure> {
        self.capture_existing(Choice::Exact(format))
    }
    fn capture_existing(self, choice: Choice) -> Result<OpenedExisting, OpenFailure> {
        let result = (|| {
            let expected = self.version.as_ref().ok_or(OpenIssue::Missing)?;
            self.recheck_parent().map_err(OpenIssue::Identity)?;
            let mut file = OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.resolved_path())
                .map_err(|e| OpenIssue::Identity(e.into()))?;
            check_kind(&file, false).map_err(OpenIssue::Identity)?;
            let version = read_version_handle(&mut file).map_err(OpenIssue::Identity)?;
            if &version != expected
                || volume_path(&file).map_err(OpenIssue::Identity)? != self.resolved_path()
            {
                return Err(OpenIssue::Identity(IdentityError::Changed));
            }
            let loaded = match choice {
                Choice::Detect(explicit) => LoadedBytes::decode(version.bytes.clone(), explicit),
                Choice::Exact(format) => LoadedBytes::decode_exact(version.bytes.clone(), format),
            }
            .map_err(OpenIssue::Decode)?;
            // Decode may take time: repeat the original-handle/parent check before
            // returning the capture. This is still not a filesystem-wide CAS.
            self.recheck_parent().map_err(OpenIssue::Identity)?;
            if read_version_handle(&mut file).map_err(OpenIssue::Identity)? != version
                || volume_path(&file).map_err(OpenIssue::Identity)? != self.resolved_path()
            {
                return Err(OpenIssue::Identity(IdentityError::Changed));
            }
            Ok((file, version, loaded))
        })();
        match result {
            Ok((file, version, loaded)) => Ok(OpenedExisting {
                file,
                observed: self,
                version,
                loaded,
            }),
            Err(issue) => Err(OpenFailure {
                issue,
                observed: self,
            }),
        }
    }
}
impl OpenedExisting {
    /// Consumes a private capture while its original read handle and parent live.
    /// Normal returned errors leave Store unchanged and return the capture.
    pub fn attach(mut self, store: &mut crate::Store) -> Result<OpenOutcome, AttachFailure> {
        let result = (|| {
            self.recheck().map_err(AttachIssue::Identity)?;
            // Keep object lifetime after releasing the strong read lock, while
            // allowing future normal writers/replacement. Verify under that lock.
            let file = OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.observed.resolved_path())
                .map_err(|e| AttachIssue::Identity(e.into()))?;
            check_kind(&file, false).map_err(AttachIssue::Identity)?;
            let id = identity(&file).map_err(AttachIssue::Identity)?;
            if id != self.version.identity {
                return Err(AttachIssue::Identity(IdentityError::Changed));
            }
            self.recheck().map_err(AttachIssue::Identity)?;
            let anchor = OpenAnchor { file, id };
            let binding_anchor = anchor.try_clone().map_err(AttachIssue::Identity)?;
            let outcome =
                store.import_verified_open(&self.loaded, anchor, self.observed.save_scope())?;
            Ok((outcome, binding_anchor))
        })();
        match result {
            Ok((crate::history::OpenDocument::Existing(snapshot), _)) => {
                Ok(OpenOutcome::Existing { snapshot })
            }
            Ok((crate::history::OpenDocument::Imported(snapshot), anchor)) => {
                let binding = FileBinding {
                    imported: snapshot.clone(),
                    original: self.loaded.original,
                    format: self.loaded.format,
                    disk: Some(crate::files::DiskBinding {
                        anchor,
                        version: self.version,
                        path: self.observed.resolved_path(),
                        epoch: 1,
                    }),
                };
                Ok(OpenOutcome::Imported { snapshot, binding })
            }
            Err(issue) => Err(AttachFailure {
                issue,
                capture: self,
            }),
        }
    }
    pub fn loaded(&self) -> &LoadedBytes {
        &self.loaded
    }
    pub fn version(&self) -> &FileVersion {
        &self.version
    }
    pub fn observed(&self) -> &ObservedTarget {
        &self.observed
    }
    /// Revalidates the retained original handle, not a freshly opened path.
    /// It does not update the captured bytes/format or accept a newer version.
    pub fn recheck(&mut self) -> Result<(), IdentityError> {
        self.observed.recheck_parent()?;
        if read_version_handle(&mut self.file)? != self.version
            || volume_path(&self.file)? != self.observed.resolved_path()
        {
            return Err(IdentityError::Changed);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
