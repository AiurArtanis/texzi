//! Test-only captured-credential faults, not native creation with a NULL DACL.
use super::*;
use crate::files::windows_security::TestCreationDacl;

#[test]
fn unsupported_creation_dacl_is_rejected_before_permit_without_mutating_owner_or_files() {
    for (label, case) in [
        ("absent-dacl", TestCreationDacl::Absent),
        ("null-dacl", TestCreationDacl::Null),
    ] {
        let CreationFixture {
            dir,
            target,
            mut store,
            n,
            mut locked,
            created,
            private,
            private_probe,
        } = creation_fixture(label);
        let capture = locked.capture().clone();
        let temporary = locked.recovery_temp().to_owned();
        // Inject only the private test credential. No descriptor is applied to
        // a filesystem object; the original File remains private and exclusive.
        let unsupported = SecurityDescriptor::for_test_creation_dacl(case).unwrap();
        assert_eq!(
            unsupported
                .validate_creation_dacl_restore()
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
        locked.set_creation_security_for_test(unsupported);
        let current = newer_edit(&mut store, &n);
        let entries = directory_entries(&dir);
        let (before, _) = store
            .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
            .unwrap();
        let depth = store.history_depth(n.doc).unwrap();
        let charge = store.history_charge();
        let failure = match locked.prepare_new_creation_default() {
            Ok(_) => panic!("unsupported captured DACL acquired a publication capability"),
            Err(failure) => failure,
        };
        assert!(matches!(&failure.issue, PublishIssue::Io(e)
            if e.kind() == std::io::ErrorKind::InvalidData));
        assert_eq!(failure.security_policy(), NewFileSecurity::CreationDefault);
        assert_eq!(failure.capture().operation, capture.operation);
        assert_eq!(failure.capture().snapshot.token, n.token);
        assert_eq!(
            store.captured_save_stage(&capture),
            Ok(SaveStage::WritingTemp)
        );
        assert_eq!(store.publication_recovery.len(), 0);
        assert!(!target.exists());
        assert!(temporary.exists());
        assert_eq!(directory_entries(&dir), entries);
        assert_eq!(SecurityDescriptor::read(&private_probe).unwrap(), private);
        assert_separate_reader_denied(&temporary);
        let version = failure.current_version().unwrap();
        assert_eq!(version.identity, created.identity);
        assert_eq!(version.security, private);
        assert_eq!(version.bytes.as_ref(), CAPTURED_UTF16);
        assert_eq!(failure.read_retained_bytes().unwrap(), CAPTURED_UTF16);
        assert_eq!(store.snapshot(n.doc).unwrap().token, current.token);
        assert_eq!(store.history_depth(n.doc).unwrap(), depth);
        assert_eq!(store.history_charge(), charge);
        let (after, _) = store
            .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
            .unwrap();
        assert_eq!(after.cursor, before.cursor);
        assert_eq!(after.state, before.state);
        failure.record_failure(&mut store).unwrap();
        let (recorded, _) = store
            .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
            .unwrap();
        let recovery = &recorded.state.global.metadata.attention.recoveries[0];
        assert_eq!(recovery.detail.outcome, SaveFailureOutcome::BeforeReplace);
        assert!(!recovery.detail.conflict);
        assert_eq!(recovery.token, n.token);
        assert_eq!(store.snapshot(n.doc).unwrap().token, current.token);
        assert!(store.dirty(n.doc).unwrap());
        assert!(!target.exists());
        assert_eq!(SecurityDescriptor::read(&private_probe).unwrap(), private);
    }
}

#[test]
fn non_null_empty_creation_dacl_passes_preflight_without_applying_empty_acl_to_a_file() {
    let CreationFixture {
        dir,
        target,
        mut store,
        n,
        mut locked,
        created,
        private,
        private_probe,
    } = creation_fixture("empty-dacl-preflight");
    let capture = locked.capture().clone();
    let empty = SecurityDescriptor::for_test_creation_dacl(TestCreationDacl::Empty).unwrap();
    empty.validate_creation_dacl_restore().unwrap();
    locked.set_creation_security_for_test(empty);
    let current = newer_edit(&mut store, &n);
    let entries = directory_entries(&dir);
    let (before, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    let ready = locked.prepare_new_creation_default().unwrap();
    assert_eq!(ready.security_policy(), NewFileSecurity::CreationDefault);
    assert_eq!(
        store.captured_save_stage(&capture),
        Ok(SaveStage::WritingTemp)
    );
    assert_eq!(store.publication_recovery.len(), 0);
    assert!(!target.exists());
    assert_eq!(directory_entries(&dir), entries);
    assert_eq!(SecurityDescriptor::read(&private_probe).unwrap(), private);
    assert_separate_reader_denied(&created.path);
    let (prepared, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(prepared.cursor, before.cursor);
    assert_eq!(prepared.state, before.state);
    assert_eq!(store.snapshot(n.doc).unwrap().token, current.token);
    // Do not permit or publish a synthetic empty-ACL credential. Only its
    // representation and factory preflight are under test, never OS inheritance.
    store.cancel_save(&capture).unwrap();
    drop(ready);
    assert!(!target.exists());
    assert_eq!(SecurityDescriptor::read(&private_probe).unwrap(), private);
}
