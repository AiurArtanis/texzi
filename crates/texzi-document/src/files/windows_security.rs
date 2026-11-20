//! Owner/group/DACL observation and copying onto a new temporary file only.
//! No SACL access or privilege changes.
use std::{
    fs::File,
    io,
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
            SE_FILE_OBJECT, SetSecurityInfo,
        },
        *,
    },
    Storage::FileSystem::CreateDirectoryW,
};

struct LocalDescriptor(PSECURITY_DESCRIPTOR);
impl Drop for LocalDescriptor {
    fn drop(&mut self) {
        // SAFETY: Windows returned this LocalAlloc-managed descriptor.
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
    fn private_descriptor() -> io::Result<LocalDescriptor> {
        let sddl: Vec<u16> = "D:P(A;;FA;;;OW)(A;;FA;;;SY)\0".encode_utf16().collect();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: terminated static-policy input and a valid output pointer.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if descriptor.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "missing private descriptor",
            ));
        }
        Ok(LocalDescriptor(descriptor))
    }

    /// Create the recovery directory with its restrictive DACL already in place.
    pub(super) fn create_private_directory(path: &Path) -> io::Result<()> {
        let descriptor = Self::private_descriptor()?;
        let path: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        // SAFETY: the internally allocated sibling name, SECURITY_ATTRIBUTES and
        // self-relative descriptor remain live for this synchronous call.
        if unsafe { CreateDirectoryW(path.as_ptr(), &attributes) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Seal a new, empty temporary file without inheriting broader reader ACEs.
    /// Keep its actual owner/group. OWNER_RIGHTS is the file owner, not everyone.
    pub(super) fn protect_new_file(file: &File) -> io::Result<()> {
        let owned = Self::private_descriptor()?;
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl = std::ptr::null_mut();
        // SAFETY: the descriptor was produced by the conversion API and is live.
        if unsafe { GetSecurityDescriptorDacl(owned.0, &mut present, &mut dacl, &mut defaulted) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        if present == 0 || dacl.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "missing private temporary DACL",
            ));
        }
        let code = unsafe {
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                dacl,
                std::ptr::null(),
            )
        };
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        let actual = Self::read(file)?;
        let actual_descriptor = actual.storage.as_ptr().cast_mut().cast();
        let mut control = 0;
        let mut revision = 0;
        let mut actual_present = 0;
        let mut actual_defaulted = 0;
        let mut actual_dacl = std::ptr::null_mut();
        // SAFETY: both descriptors are live OS-produced, valid allocations.
        if unsafe { GetSecurityDescriptorControl(actual_descriptor, &mut control, &mut revision) }
            == 0
            || unsafe {
                GetSecurityDescriptorDacl(
                    actual_descriptor,
                    &mut actual_present,
                    &mut actual_dacl,
                    &mut actual_defaulted,
                )
            } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if actual_present == 0 || actual_dacl.is_null() || control & SE_DACL_PROTECTED == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "private temporary DACL was not protected",
            ));
        }
        let matches = unsafe {
            let expected = std::slice::from_raw_parts(dacl.cast::<u8>(), (*dacl).AclSize as usize);
            let returned = std::slice::from_raw_parts(
                actual_dacl.cast::<u8>(),
                (*actual_dacl).AclSize as usize,
            );
            expected == returned
        };
        if !matches {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "private temporary DACL mismatch",
            ));
        }
        Ok(())
    }
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
    /// Validate only the captured creation DACL representation before a permit.
    /// This does not open a file, change an object, or enable any privilege.
    pub(super) fn validate_creation_dacl_restore(&self) -> io::Result<()> {
        self.creation_dacl_restore_parts().map(|_| ())
    }
    /// SetSecurityInfo can mark an unprotected creation DACL as auto-inherited
    /// when it restores the same owner, group and ACE bytes. Only that stamp
    /// may be added. Protection, ACE inheritance flags and order stay exact.
    pub(super) fn matches_restored_creation(&self, actual: &Self) -> bool {
        if self == actual {
            return true;
        }
        let start = std::mem::offset_of!(SECURITY_DESCRIPTOR_RELATIVE, Control);
        let end = start + std::mem::size_of::<u16>();
        let expected = self.as_bytes();
        let actual = actual.as_bytes();
        let expected_control = u16::from_le_bytes(expected[start..end].try_into().unwrap());
        let actual_control = u16::from_le_bytes(actual[start..end].try_into().unwrap());
        expected_control & (SE_DACL_PROTECTED | SE_DACL_AUTO_INHERITED) == 0
            && actual_control == (expected_control | SE_DACL_AUTO_INHERITED)
            && expected[..start] == actual[..start]
            && expected[end..] == actual[end..]
    }
    #[cfg(test)]
    pub(super) fn with_auto_inherited_for_test(&self, enabled: bool) -> Self {
        let mut copy = self.clone();
        // SAFETY: this is an aligned copy of a live, validated self-relative
        // descriptor. Alter only the DACL control stamp for the fixture.
        assert_ne!(
            unsafe {
                SetSecurityDescriptorControl(
                    copy.storage.as_mut_ptr().cast(),
                    SE_DACL_AUTO_INHERITED,
                    if enabled { SE_DACL_AUTO_INHERITED } else { 0 },
                )
            },
            0
        );
        copy
    }
    /// Internal caller must retain the original FILE_SHARE_NONE create_new File
    /// and prove that its no-overwrite publication succeeded before this call.
    /// This does not authorize an arbitrary path or an existing document. Only
    /// the creation DACL is restored; the publisher must reread and compare the
    /// complete security descriptor before accepting the result.
    pub(super) fn restore_creation_dacl_to_published_new(&self, file: &File) -> io::Result<()> {
        let (dacl, flags) = self.creation_dacl_restore_parts()?;
        // SAFETY: dacl refers into this live, validated OS-produced descriptor;
        // the internal publisher retains the original exclusively held File.
        // OWNER/GROUP/SACL are neither requested nor changed by this operation.
        let code = unsafe {
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                dacl,
                std::ptr::null(),
            )
        };
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        Ok(())
    }
    fn creation_dacl_restore_parts(&self) -> io::Result<(*mut ACL, u32)> {
        let descriptor = self.storage.as_ptr().cast_mut().cast();
        let mut control = 0;
        let mut revision = 0;
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl = std::ptr::null_mut();
        // SAFETY: the aligned self-relative descriptor came from GetSecurityInfo
        // and stays live. Both APIs read its storage without modifying it.
        if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0
            || unsafe {
                GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
            } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if present == 0 || dacl.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "creation DACL restoration requires a present non-NULL DACL",
            ));
        }
        let inheritance = if control & SE_DACL_PROTECTED != 0 {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
        Ok((dacl, DACL_SECURITY_INFORMATION | inheritance))
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

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TestCreationDacl {
    Absent,
    Null,
    Empty,
}

#[cfg(test)]
impl SecurityDescriptor {
    /// Isolated representation fixture; never applies a descriptor to a file.
    pub(super) fn for_test_creation_dacl(case: TestCreationDacl) -> io::Result<Self> {
        let header_length = std::mem::size_of::<SECURITY_DESCRIPTOR_RELATIVE>();
        let dacl_offset = header_length.next_multiple_of(std::mem::align_of::<ACL>());
        let has_acl = case == TestCreationDacl::Empty;
        let length = if has_acl {
            dacl_offset + std::mem::size_of::<ACL>()
        } else {
            header_length
        };
        let mut storage = vec![0u64; length.div_ceil(std::mem::size_of::<u64>())];
        let header = SECURITY_DESCRIPTOR_RELATIVE {
            // The Windows SECURITY_DESCRIPTOR revision is 1. Its constant is
            // in SystemServices, which this adapter does not otherwise need.
            Revision: 1,
            Control: SE_SELF_RELATIVE
                | if case == TestCreationDacl::Absent {
                    0
                } else {
                    SE_DACL_PRESENT
                },
            Dacl: if has_acl { dacl_offset as u32 } else { 0 },
            ..Default::default()
        };
        // SAFETY: the initialized u64 allocation is large and aligned enough
        // for the cached Windows ABI's relative header (alignment 4) and ACL
        // (alignment 2). The optional ACL starts after the header at its own
        // alignment, and every self-relative offset stays inside this buffer.
        unsafe {
            storage
                .as_mut_ptr()
                .cast::<SECURITY_DESCRIPTOR_RELATIVE>()
                .write(header);
            if has_acl {
                storage
                    .as_mut_ptr()
                    .cast::<u8>()
                    .add(dacl_offset)
                    .cast::<ACL>()
                    .write(ACL {
                        AclRevision: ACL_REVISION as u8,
                        AclSize: std::mem::size_of::<ACL>() as u16,
                        AceCount: 0,
                        ..Default::default()
                    });
            }
        }
        let result = Self { storage, length };
        // SAFETY: the descriptor and its optional ACL are initialized above in
        // live, aligned owned storage. This API only validates that memory.
        if unsafe { IsValidSecurityDescriptor(result.storage.as_ptr().cast_mut().cast()) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid test creation DACL descriptor",
            ));
        }
        Ok(result)
    }

    /// Copy captured owner/group SID bytes for equality checks without logging.
    pub(super) fn owner_group_for_test(&self) -> io::Result<(Vec<u8>, Vec<u8>)> {
        let descriptor = self.storage.as_ptr().cast_mut().cast();
        // SAFETY: every descriptor constructor retains a live, aligned owned
        // allocation. Validation here precedes reading either SID pointer.
        if unsafe { IsValidSecurityDescriptor(descriptor) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid descriptor for owner/group observation",
            ));
        }
        let mut owner = std::ptr::null_mut();
        let mut group = std::ptr::null_mut();
        let mut defaulted = 0;
        // SAFETY: both output SID pointers refer into the live, validated
        // descriptor. Neither getter modifies the security descriptor.
        if unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted) } == 0
            || unsafe { GetSecurityDescriptorGroup(descriptor, &mut group, &mut defaulted) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let copy_sid = |sid: PSID| -> io::Result<Vec<u8>> {
            if sid.is_null() {
                return Ok(Vec::new());
            }
            // SAFETY: the getter returned this pointer from the validated,
            // still-live descriptor; both APIs only inspect its SID bytes.
            if unsafe { IsValidSid(sid) } == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid captured owner/group SID",
                ));
            }
            let length = unsafe { GetLengthSid(sid) } as usize;
            if length == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "empty captured owner/group SID",
                ));
            }
            // SAFETY: a valid SID occupies the number of bytes returned by
            // GetLengthSid and remains owned by self throughout this copy.
            Ok(unsafe { std::slice::from_raw_parts(sid.cast::<u8>(), length) }.to_vec())
        };
        Ok((copy_sid(owner)?, copy_sid(group)?))
    }
}

#[cfg(test)]
mod creation_policy_tests;
