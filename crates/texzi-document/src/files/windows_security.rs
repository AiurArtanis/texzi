//! Owner/group/DACL observation and copying onto a new temporary file only.
//! No SACL access or privilege changes.
use std::{fs::File, io, os::windows::io::AsRawHandle};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{GetSecurityInfo, SE_FILE_OBJECT, SetSecurityInfo},
        *,
    },
};

struct LocalDescriptor(PSECURITY_DESCRIPTOR);
impl Drop for LocalDescriptor {
    fn drop(&mut self) {
        // SAFETY: GetSecurityInfo returned this LocalAlloc-managed descriptor.
        unsafe {
            LocalFree(self.0);
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SecurityDescriptor {
    // Aligned storage for the self-relative Windows descriptor, not text/log data.
    storage: Vec<u64>,
    length: usize,
}
impl std::fmt::Debug for SecurityDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecurityDescriptor")
            .field("bytes", &self.length)
            .finish_non_exhaustive()
    }
}
impl SecurityDescriptor {
    #[cfg(test)]
    pub(super) fn with_flipped_dacl_protection(&self) -> io::Result<Self> {
        let mut changed = self.clone();
        let descriptor = changed.storage.as_mut_ptr().cast();
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: live aligned copy of a validated OS-produced descriptor.
        if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe {
            SetSecurityDescriptorControl(
                descriptor,
                SE_DACL_PROTECTED,
                (control ^ SE_DACL_PROTECTED) & SE_DACL_PROTECTED,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(changed)
    }
    /// Internal caller must supply only its own create_new temporary handle,
    /// before writing payload. Never applies to an existing document path.
    pub(super) fn apply_to_new_file(&self, file: &File) -> io::Result<()> {
        let descriptor = self.storage.as_ptr().cast_mut().cast();
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: descriptor came from GetSecurityInfo and remains live;
        // these APIs read the input descriptor and do not modify its buffer.
        if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let inheritance = if control & SE_DACL_PROTECTED != 0 {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
        let flags = OWNER_SECURITY_INFORMATION
            | GROUP_SECURITY_INFORMATION
            | DACL_SECURITY_INFORMATION
            | inheritance;
        let mut owner = std::ptr::null_mut();
        let mut group = std::ptr::null_mut();
        let mut dacl = std::ptr::null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        // SAFETY: read-only OS-produced descriptor; output pointers refer into
        // its live storage for the duration of SetSecurityInfo below.
        if unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted) } == 0
            || unsafe { GetSecurityDescriptorGroup(descriptor, &mut group, &mut defaulted) } == 0
            || unsafe {
                GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
            } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // Do not turn an absent/uninterpretable DACL into a NULL DACL. A present
        // NULL DACL is retained only when explicitly present in the source.
        if owner.is_null() || group.is_null() || present == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported owner/group/DACL representation",
            ));
        }
        // SAFETY: live create_new handle, validated OS-produced descriptor. The
        // OS enforces WRITE_OWNER/WRITE_DAC; no privileges are enabled on error.
        let result = unsafe {
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                flags,
                owner,
                group,
                dacl,
                std::ptr::null(),
            )
        };
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        Ok(())
    }
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: length was checked against initialized aligned storage below.
        unsafe { std::slice::from_raw_parts(self.storage.as_ptr().cast(), self.length) }
    }
    pub(super) fn read(file: &File) -> io::Result<Self> {
        let flags =
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: live file/directory handle and valid output pointer. Other
        // output pointers are optional; request the complete descriptor only.
        let result = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        if descriptor.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "missing security descriptor",
            ));
        }
        let owned = LocalDescriptor(descriptor);
        // SAFETY: successful GetSecurityInfo descriptor remains allocated here.
        if unsafe { IsValidSecurityDescriptor(owned.0) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid security descriptor",
            ));
        }
        let mut control = 0;
        let mut revision = 0;
        if unsafe { GetSecurityDescriptorControl(owned.0, &mut control, &mut revision) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if control & SE_SELF_RELATIVE == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported absolute security descriptor",
            ));
        }
        let needed = unsafe { GetSecurityDescriptorLength(owned.0) };
        // Bound our owned copy; Windows owns the source allocation until drop.
        if needed == 0 || needed > 1_048_576 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid security descriptor size",
            ));
        }
        let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: validated self-relative descriptor of needed bytes; our aligned
        // initialized destination is at least that large and does not overlap.
        unsafe {
            std::ptr::copy_nonoverlapping(
                owned.0.cast::<u8>(),
                storage.as_mut_ptr().cast(),
                needed as usize,
            );
        }
        Ok(Self {
            storage,
            length: needed as usize,
        })
    }
}
