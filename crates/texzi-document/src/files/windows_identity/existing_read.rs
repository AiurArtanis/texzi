//! Existing-target read capture without decoding, attachment, or publication.
use super::{IdentityError, ObservedTarget, check_kind, read_version_handle, volume_path};
use std::{
    fs::{File, OpenOptions},
    os::windows::fs::OpenOptionsExt,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};

/// Only the parent module can request this capture, from its own observation.
/// Sharing failures are returned without retrying with broader permissions.
pub(super) fn hold_existing_read(observed: &ObservedTarget) -> Result<File, IdentityError> {
    let expected = observed.version.as_ref().ok_or(IdentityError::WrongKind)?;
    observed.recheck_parent()?;
    let path = observed.resolved_path();
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)?;
    check_kind(&file, false)?;
    if &read_version_handle(&mut file)? != expected || volume_path(&file)? != path {
        return Err(IdentityError::Changed);
    }
    // Retain the same handle while repeating the original baseline and parent
    // checks. A failed capture drops only this local file, not the observation.
    observed.recheck_parent()?;
    if &read_version_handle(&mut file)? != expected || volume_path(&file)? != path {
        return Err(IdentityError::Changed);
    }
    Ok(file)
}
