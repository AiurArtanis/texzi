use super::*;

fn assert_creation_dacl_representation(case: TestCreationDacl) -> SecurityDescriptor {
    let captured = SecurityDescriptor::for_test_creation_dacl(case)
        .expect("construct isolated self-relative creation descriptor");
    let descriptor = captured.storage.as_ptr().cast_mut().cast();
    // SAFETY: the fixture retains initialized, aligned owned memory; all APIs
    // below inspect that buffer without opening or modifying a file.
    assert_ne!(unsafe { IsValidSecurityDescriptor(descriptor) }, 0);
    assert_eq!(
        unsafe { GetSecurityDescriptorLength(descriptor) } as usize,
        captured.length
    );
    let mut control = 0;
    let mut revision = 0;
    assert_ne!(
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) },
        0
    );
    assert_eq!(revision, 1);
    assert_ne!(control & SE_SELF_RELATIVE, 0);
    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl = std::ptr::null_mut();
    assert_ne!(
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) },
        0
    );
    // SAFETY: the constructor initialized this aligned relative header, whose
    // validity and SE_SELF_RELATIVE control bit were checked above.
    let relative = unsafe {
        &*captured
            .storage
            .as_ptr()
            .cast::<SECURITY_DESCRIPTOR_RELATIVE>()
    };
    match case {
        TestCreationDacl::Absent => {
            assert_eq!(control & SE_DACL_PRESENT, 0);
            assert_eq!(present, 0);
            assert_eq!(relative.Dacl, 0);
            // When present is false, Windows does not define the other DACL
            // outputs. The relative zero offset above proves the fixture's
            // representation without interpreting the returned pointer.
        }
        TestCreationDacl::Null => {
            assert_ne!(control & SE_DACL_PRESENT, 0);
            assert_ne!(present, 0);
            assert_eq!(relative.Dacl, 0);
            assert!(dacl.is_null());
        }
        TestCreationDacl::Empty => {
            assert_ne!(control & SE_DACL_PRESENT, 0);
            assert_ne!(present, 0);
            assert_ne!(relative.Dacl, 0);
            assert!(!dacl.is_null());
            let offset = relative.Dacl as usize;
            assert!(offset >= std::mem::size_of::<SECURITY_DESCRIPTOR_RELATIVE>());
            assert!(offset + std::mem::size_of::<ACL>() <= captured.length);
            // SAFETY: the nonzero relative offset and complete ACL were
            // initialized inside this live allocation and checked above.
            let expected_dacl = unsafe {
                captured
                    .storage
                    .as_ptr()
                    .cast::<u8>()
                    .add(offset)
                    .cast::<ACL>()
            };
            assert_eq!(dacl.cast_const(), expected_dacl);
            assert_ne!(unsafe { IsValidAcl(dacl) }, 0);
            assert_eq!(
                unsafe { (*dacl).AclSize } as usize,
                std::mem::size_of::<ACL>()
            );
            assert_eq!(unsafe { (*dacl).AceCount }, 0);
        }
    }
    captured
}

#[test]
fn creation_dacl_restore_rejects_absent_and_null_descriptors() {
    for case in [TestCreationDacl::Absent, TestCreationDacl::Null] {
        let captured = assert_creation_dacl_representation(case);
        assert_eq!(
            captured
                .validate_creation_dacl_restore()
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData,
            "unsupported creation DACL representation: {case:?}"
        );
    }
}

#[test]
fn creation_dacl_restore_accepts_present_empty_acl() {
    let empty = assert_creation_dacl_representation(TestCreationDacl::Empty);
    for (protected, captured) in [
        (false, empty.clone()),
        (true, empty.with_flipped_dacl_protection().unwrap()),
    ] {
        captured
            .validate_creation_dacl_restore()
            .expect("a present non-NULL ACL with zero ACEs is a supported representation");
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: this owned relative descriptor was validated above; the
        // test-only flipped clone changes only its DACL protection control bit.
        assert_ne!(
            unsafe {
                GetSecurityDescriptorControl(
                    captured.storage.as_ptr().cast_mut().cast(),
                    &mut control,
                    &mut revision,
                )
            },
            0
        );
        assert_eq!(control & SE_DACL_PROTECTED != 0, protected);
        let (dacl, flags) = captured
            .creation_dacl_restore_parts()
            .expect("parse the isolated empty ACL without applying it to a file");
        assert!(!dacl.is_null());
        let inheritance = if protected {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
        assert_eq!(flags, DACL_SECURITY_INFORMATION | inheritance);
    }
}
