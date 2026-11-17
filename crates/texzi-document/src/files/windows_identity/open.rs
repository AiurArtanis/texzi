//! Read-only captured open. Store identity/baseline attachment is a separate step.
use super::*;
use crate::files::{Encoding, FileError, Format, LoadedBytes};

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
