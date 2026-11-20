use super::*;
use crate::{
    BoundaryPolicy, Edit, HistoryBudget, Snapshot, Transaction,
    files::windows_identity::OpenOutcome,
};
use std::{
    fs,
    os::windows::io::{FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1, SE_FILE_OBJECT,
            SetSecurityInfo,
        },
        CreateRestrictedToken, CreateWellKnownSid, DACL_SECURITY_INFORMATION,
        DISABLE_MAX_PRIVILEGE, GetSecurityDescriptorDacl, ImpersonateLoggedOnUser,
        PROTECTED_DACL_SECURITY_INFORMATION, RevertToSelf, SECURITY_MAX_SID_SIZE,
        SID_AND_ATTRIBUTES, TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_QUERY, WinWorldSid,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

fn allow_reader_on_fixture(path: &Path) {
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL | WRITE_DAC)
        .open(path)
        .unwrap();
    let sddl: Vec<_> = "D:P(A;;FA;;;OW)(A;;FA;;;SY)(A;;FR;;;WD)\0"
        .encode_utf16()
        .collect();
    let mut descriptor = ptr::null_mut();
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = ptr::null_mut();
    // SAFETY: Only the generated original is changed, through its live handle.
    // The terminated test descriptor stays valid through SetSecurityInfo.
    unsafe {
        assert_ne!(
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut()
            ),
            0
        );
        let got_acl = GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted);
        let applied = if got_acl != 0 && present != 0 && !acl.is_null() {
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                acl,
                ptr::null_mut(),
            )
        } else {
            u32::MAX
        };
        LocalFree(descriptor);
        assert_eq!(applied, 0);
    }
}

fn as_reader(path: &Path) -> io::Result<Vec<u8>> {
    struct Revert;
    impl Drop for Revert {
        fn drop(&mut self) {
            // SAFETY: Impersonation was enabled only on this test thread.
            assert_ne!(unsafe { RevertToSelf() }, 0);
        }
    }
    let mut original = ptr::null_mut();
    let mut restricted = ptr::null_mut();
    let mut sid = [0u64; (SECURITY_MAX_SID_SIZE as usize).div_ceil(8)];
    let mut sid_size = SECURITY_MAX_SID_SIZE;
    // SAFETY: The test uses a separate restricted token, never changes the
    // process token, and always reverts its thread before returning.
    let _revert = unsafe {
        assert_ne!(
            CreateWellKnownSid(
                WinWorldSid,
                ptr::null_mut(),
                sid.as_mut_ptr().cast(),
                &mut sid_size
            ),
            0
        );
        let everyone = SID_AND_ATTRIBUTES {
            Sid: sid.as_mut_ptr().cast(),
            Attributes: 0,
        };
        assert_ne!(
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_IMPERSONATE,
                &mut original
            ),
            0
        );
        let original = OwnedHandle::from_raw_handle(original);
        assert_ne!(
            CreateRestrictedToken(
                original.as_raw_handle(),
                DISABLE_MAX_PRIVILEGE,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                &everyone,
                &mut restricted
            ),
            0
        );
        let restricted = OwnedHandle::from_raw_handle(restricted);
        assert_ne!(ImpersonateLoggedOnUser(restricted.as_raw_handle()), 0);
        Revert
    };
    fs::read(path)
}

fn fixture() -> (PathBuf, Store, Snapshot, FileBinding) {
    let (path, _, _, binding) = super::super::current_tests::fixture();
    drop(binding);
    allow_reader_on_fixture(&path);
    assert_eq!(as_reader(&path).unwrap(), b"A\r\n");
    let mut store = Store::new(HistoryBudget::default());
    let OpenOutcome::Imported { snapshot, binding } = ObservedTarget::inspect(&path)
        .unwrap()
        .open_existing(None)
        .unwrap()
        .attach(&mut store)
        .unwrap()
    else {
        panic!("expected fresh fixture import")
    };
    (path, store, snapshot, binding)
}

fn insert(store: &mut Store, snapshot: &Snapshot, text: &str) -> Snapshot {
    store
        .submit(Transaction {
            doc: snapshot.doc,
            base: snapshot.revision,
            edits: vec![Edit::input(snapshot, 0..0, text)],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot
}

fn assert_private(source: &Path) {
    assert_eq!(fs::read(source).unwrap(), b"NA\r\n");
    assert_eq!(as_reader(source).unwrap_err().raw_os_error(), Some(5));
}

#[test]
fn current_replace_private_until_commit_for_original_reader() {
    let (path, mut store, initial, binding) = fixture();
    let n = insert(&mut store, &initial, "N");
    let prepared = tests::ready(&path, &mut store, &binding);
    assert_private(prepared.workspace.source_path());
    assert_eq!(as_reader(&path).unwrap(), b"A\r\n");
    let newer = insert(&mut store, &n, "X");
    let permitted = prepared.permit(&mut store).unwrap();
    assert_private(permitted.ready.workspace.source_path());
    let accepted = permitted.run().unwrap().accept(&mut store).unwrap();
    assert_eq!(accepted.imported.token, n.token);
    assert_eq!(store.snapshot(initial.doc).unwrap().token, newer.token);
    assert!(store.dirty(initial.doc).unwrap());
    assert_eq!(as_reader(&path).unwrap(), b"NA\r\n");
    assert_eq!(
        accepted.disk.as_ref().unwrap().version.security,
        binding.disk.as_ref().unwrap().version.security
    );
    eprintln!(
        "restricted reader: original readable, prepared/permit source denied 5, published N readable, N+1 dirty"
    );
}

#[test]
fn current_replace_keeps_publication_copy_private_on_permit_and_sharing_failures() {
    for before_permit in [true, false] {
        let (path, mut store, initial, binding) = fixture();
        let n = insert(&mut store, &initial, "N");
        let prepared = tests::ready(&path, &mut store, &binding);
        let source = prepared.workspace.source_path().to_owned();
        let newer = insert(&mut store, &n, "X");
        let failure = if before_permit {
            store.cancel_save(prepared.staged.capture()).unwrap();
            prepared
                .permit(&mut store)
                .err()
                .expect("cancelled capture should reject permit")
        } else {
            let read_lock = OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .open(&path)
                .unwrap();
            let failure = prepared
                .permit(&mut store)
                .unwrap()
                .run()
                .err()
                .expect("sharing conflict should stop replace");
            assert!(failure.attempted);
            assert!(
                matches!(&failure.issue, PublishIssue::Io(error) if error.raw_os_error() == Some(32))
            );
            assert!(failure.source_protection_error.is_none());
            drop(read_lock);
            failure.record_failure(&mut store).unwrap();
            failure
        };
        assert!(failure._workspace.is_some());
        assert_private(&source);
        assert_eq!(as_reader(&path).unwrap(), b"A\r\n");
        assert_eq!(failure.staged.read_created_bytes().unwrap(), b"NA\r\n");
        assert_eq!(store.snapshot(initial.doc).unwrap().token, newer.token);
        assert!(store.dirty(initial.doc).unwrap());
        eprintln!(
            "restricted reader: before_permit={before_permit}, target A readable, retained source denied 5, staged N and dirty N+1 preserved"
        );
    }
}
