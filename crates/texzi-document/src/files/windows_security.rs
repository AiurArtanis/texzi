//! Read-only owner/group/DACL observation. No SACL access or privilege changes.
use std::{fs::File, io, os::windows::io::AsRawHandle};
use windows_sys::Win32::{Foundation::ERROR_INSUFFICIENT_BUFFER, Security::*};

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
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: length was checked against initialized aligned storage below.
        unsafe { std::slice::from_raw_parts(self.storage.as_ptr().cast(), self.length) }
    }
    pub(super) fn read(file: &File) -> io::Result<Self> {
        let flags =
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut needed = 0;
        // SAFETY: live handle, null output with zero capacity, valid size pointer.
        let ok = unsafe {
            GetKernelObjectSecurity(
                file.as_raw_handle(),
                flags,
                std::ptr::null_mut(),
                0,
                &mut needed,
            )
        };
        let error = io::Error::last_os_error();
        if ok == 0 && error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
            return Err(error);
        }
        // Windows ACLs are bounded; refuse unexplained allocations before IO.
        if needed == 0 || needed > 1_048_576 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid security descriptor size",
            ));
        }
        let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
        let capacity = (storage.len() * 8) as u32;
        // SAFETY: aligned, zero-initialized output of capacity bytes and live handle.
        if unsafe {
            GetKernelObjectSecurity(
                file.as_raw_handle(),
                flags,
                storage.as_mut_ptr().cast(),
                capacity,
                &mut needed,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if needed == 0 || needed > capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "security descriptor size changed",
            ));
        }
        Ok(Self {
            storage,
            length: needed as usize,
        })
    }
}
