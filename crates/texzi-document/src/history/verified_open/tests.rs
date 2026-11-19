//! Generated Windows files. Save outcomes below are memory protocol simulations.
use super::*;
use crate::files::{
    Encoding, FileBinding, Format,
    windows_identity::{ObservedTarget, OpenOutcome},
};
use crate::{ObserverScope, SaveFailure, SaveFailureOutcome};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);
fn fixture(raw: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "texzi-verified-import-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    let path = dir.join("original.txt");
    fs::write(&path, raw).unwrap();
    eprintln!(
        "T07 generated verified-import fixture retained: {}",
        dir.display()
    );
    path
}
fn imported(store: &mut Store, path: &Path) -> (Snapshot, FileBinding) {
    match ObservedTarget::inspect(path)
        .unwrap()
        .open_existing(None)
        .unwrap()
        .attach(store)
        .unwrap()
    {
        OpenOutcome::Imported { snapshot, binding } => (snapshot, binding),
        _ => panic!("expected new import"),
    }
}
fn observe(store: &mut Store, doc: DocId) -> crate::ObserverSnapshot {
    store
        .observe_and_subscribe(doc, ObserverScope::new(vec![]), 8)
        .unwrap()
        .0
}
fn sizes(store: &Store) -> Vec<usize> {
    vec![
        store.docs.len(),
        store.bound_targets.len(),
        store.opened_identities.len(),
        store.test_observer_document_count(),
        store.view_docs.len(),
        store.saves.len(),
        store.targets.len(),
        store.scope_parents.len(),
        store.scope_objects.len(),
        store.unresolved_targets.len(),
    ]
}
#[test]
fn verified_import_is_initially_bound_clean_and_byte_exact_without_history() {
    let raw = b"\xef\xbb\xbf\xef\xbb\xbfA\r\nB\n";
    let path = fixture(raw);
    let mut store = Store::new(HistoryBudget::default());
    let (snapshot, binding) = imported(&mut store, &path);
    assert_eq!(snapshot.revision, Revision(0));
    assert!(snapshot.body.to_utf8().starts_with(b"\xef\xbb\xbf"));
    assert_eq!(store.history_depth(snapshot.doc).unwrap(), (0, 0));
    assert!(!store.dirty(snapshot.doc).unwrap());
    assert!(store.backed_by_disk(snapshot.doc).unwrap());
    let initial = observe(&mut store, snapshot.doc);
    assert_eq!(initial.cursor.seq, crate::EventSeq(0));
    assert_eq!(
        initial.state.global.metadata.baseline.kind,
        BaselineKind::VerifiedOpen
    );
    assert_eq!(initial.state.global.metadata.baseline.token, snapshot.token);
    assert_eq!(initial.state.global.metadata.attention.binding_epoch, 1);
    assert_eq!(
        initial.state.global.metadata.attention.target,
        Some(ObservedTarget::inspect(&path).unwrap().reservation_key())
    );
    assert_eq!(
        binding
            .encode_snapshot(&snapshot, binding.format())
            .unwrap()
            .as_ref(),
        raw
    );
    assert_eq!(store.opened_identities.len(), 1);
    assert_eq!(store.test_observer_document_count(), 1);
}
#[test]
fn hardlink_existing_preserves_dirty_format_history_and_old_binding() {
    let raw = b"old\r\n";
    let path = fixture(raw);
    let mut store = Store::new(HistoryBudget::default());
    let (n, binding) = imported(&mut store, &path);
    let edited = store
        .submit(Transaction {
            doc: n.doc,
            base: n.revision,
            edits: vec![Edit::input(&n, 0..0, "dirty ")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    let mut profile = edited.body.profile;
    profile.file_format = Some(Format {
        encoding: Encoding::Utf16Le,
        bom: true,
    });
    let dirty = store
        .set_serialization(n.doc, edited.revision, profile, None)
        .unwrap()
        .snapshot;
    // The identity witness allows normal external writes after the strong capture ends.
    fs::write(&path, b"new on disk").unwrap();
    let alias = path.with_file_name("hardlink.txt");
    fs::hard_link(&path, &alias).unwrap();
    let before = observe(&mut store, n.doc);
    let count = sizes(&store);
    let token = store.next_token;
    let opened = ObservedTarget::inspect(&alias)
        .unwrap()
        .open_existing(Some(Encoding::Windows1252))
        .unwrap();
    match opened.attach(&mut store).unwrap() {
        OpenOutcome::Existing { snapshot } => {
            assert_eq!(snapshot.token, dirty.token);
            assert!(Arc::ptr_eq(&snapshot.body, &dirty.body));
        }
        _ => panic!("dirty document was reimported"),
    }
    let after = observe(&mut store, n.doc);
    assert_eq!(before.state, after.state);
    assert_eq!(before.cursor, after.cursor);
    assert_eq!(sizes(&store), count);
    assert_eq!(store.next_token, token);
    assert_eq!(
        binding
            .encode_snapshot(&n, binding.format())
            .unwrap()
            .as_ref(),
        raw
    );
    let encoded = binding
        .encode_snapshot(&dirty, profile.file_format.unwrap())
        .unwrap();
    assert!(encoded.starts_with(&[255, 254]));
    assert_ne!(encoded.as_ref(), b"new on disk");
}
#[test]
fn existing_unknown_is_location_only_and_does_not_release_scope() {
    let path = fixture(b"unchanged");
    let mut store = Store::new(HistoryBudget::default());
    let (n, _) = imported(&mut store, &path);
    let observed = ObservedTarget::inspect(&path).unwrap();
    let save = store
        .begin_scoped_save(
            n.doc,
            observed.reservation_key(),
            String::new(),
            observed.save_scope(),
        )
        .unwrap();
    let active_before = observe(&mut store, n.doc);
    assert!(matches!(
        ObservedTarget::inspect(&path)
            .unwrap()
            .open_existing(None)
            .unwrap()
            .attach(&mut store)
            .unwrap(),
        OpenOutcome::Existing { .. }
    ));
    let active_after = observe(&mut store, n.doc);
    assert_eq!(active_before.state, active_after.state);
    assert_eq!(active_before.cursor, active_after.cursor);
    assert_eq!(store.captured_save_stage(&save), Ok(SaveStage::Encoding));
    store.advance_save(&save, SaveStage::WritingTemp).unwrap();
    store
        .advance_save(&save, SaveStage::ReadyToReplace)
        .unwrap();
    store.commit_permit(&save).unwrap();
    store
        .finish_save_failure(
            &save,
            SaveFailure {
                outcome: SaveFailureOutcome::OutcomeUnknown,
                conflict: true,
                os_error: None,
                message: "memory-only unknown receipt fixture".into(),
                recovery_paths: vec![],
            },
        )
        .unwrap();
    let before = observe(&mut store, n.doc);
    let count = sizes(&store);
    assert!(matches!(
        ObservedTarget::inspect(&path)
            .unwrap()
            .open_existing(None)
            .unwrap()
            .attach(&mut store)
            .unwrap(),
        OpenOutcome::Existing { .. }
    ));
    let after = observe(&mut store, n.doc);
    assert_eq!(before.state, after.state);
    assert_eq!(before.cursor, after.cursor);
    assert_eq!(sizes(&store), count);
    assert!(store.docs[&n.doc].save_blocked);
}
fn occupied_import(cross_parent: bool, held: bool) {
    let path = fixture(b"new file");
    let mut store = Store::new(HistoryBudget::default());
    let original = ObservedTarget::inspect(&path).unwrap();
    let alias = if cross_parent {
        let dir = path.parent().unwrap().join("alias-parent");
        fs::create_dir(&dir).unwrap();
        let alias = dir.join("hardlink.txt");
        fs::hard_link(&path, &alias).unwrap();
        alias
    } else {
        path.clone()
    };
    let capture = ObservedTarget::inspect(&alias)
        .unwrap()
        .open_existing(None)
        .unwrap();
    let sentinel = store
        .open_utf8("sentinel", Profile::default(), false)
        .unwrap();
    let save = store
        .begin_scoped_save(
            sentinel.doc,
            original.reservation_key(),
            String::new(),
            original.save_scope(),
        )
        .unwrap();
    if held {
        store.advance_save(&save, SaveStage::WritingTemp).unwrap();
        store
            .advance_save(&save, SaveStage::ReadyToReplace)
            .unwrap();
        store.commit_permit(&save).unwrap();
        store
            .finish_save_failure(
                &save,
                SaveFailure {
                    outcome: SaveFailureOutcome::OutcomeUnknown,
                    conflict: true,
                    os_error: None,
                    message: "memory-only held scope".into(),
                    recovery_paths: vec![],
                },
            )
            .unwrap();
    }
    let before = observe(&mut store, sentinel.doc);
    let count = sizes(&store);
    let token = store.next_token;
    let failure = match capture.attach(&mut store) {
        Ok(_) => panic!("occupied import accepted"),
        Err(e) => e,
    };
    let expected = if held {
        Error::UnresolvedConflict
    } else {
        Error::Busy
    };
    assert!(matches!(&failure.issue,AttachIssue::Owner(error) if error==&expected));
    let after = observe(&mut store, sentinel.doc);
    assert_eq!(before.state, after.state);
    assert_eq!(before.cursor, after.cursor);
    assert_eq!(sizes(&store), count);
    assert_eq!(store.next_token, token);
    if !held {
        assert_eq!(store.captured_save_stage(&save), Ok(SaveStage::Encoding));
    }
    assert_eq!(fs::read(path).unwrap(), b"new file");
}
#[test]
fn parent_only_active_then_held_blocks_distinct_file_import_without_partial_changes() {
    let a_path = fixture(b"file A");
    let b_path = a_path.parent().unwrap().join("distinct-B.txt");
    let b_raw = b"file B preserved\r\n";
    fs::write(&b_path, b_raw).unwrap();
    let a = ObservedTarget::inspect(&a_path).unwrap();
    let b = ObservedTarget::inspect(&b_path).unwrap();
    let a_key = a.reservation_key();
    let b_key = b.reservation_key();
    let a_scope = a.save_scope();
    let b_scope = b.save_scope();
    assert_ne!(a.version().unwrap().identity, b.version().unwrap().identity);
    assert_ne!(a_key, b_key);
    assert_eq!(a_scope.parent, b_scope.parent);
    assert_eq!(b_scope.object, Some(b_key.clone()));
    // B is captured before the owner reserves the distinct file A.
    let mut capture = b.open_existing(None).unwrap();
    let mut store = Store::new(HistoryBudget::default());
    let sentinel = store
        .open_utf8("sentinel", Profile::default(), false)
        .unwrap();
    let save = store
        .begin_scoped_save(sentinel.doc, a_key.clone(), String::new(), a_scope)
        .unwrap();
    for held in [false, true] {
        if held {
            store.advance_save(&save, SaveStage::WritingTemp).unwrap();
            store
                .advance_save(&save, SaveStage::ReadyToReplace)
                .unwrap();
            store.commit_permit(&save).unwrap();
            store
                .finish_save_failure(
                    &save,
                    SaveFailure {
                        outcome: SaveFailureOutcome::OutcomeUnknown,
                        conflict: true,
                        os_error: None,
                        message: "memory-only Unknown on distinct A; no disk publication".into(),
                        recovery_paths: vec![],
                    },
                )
                .unwrap();
        }
        // These exclusions make the parent domain the only possible blocker.
        assert!(!store.scope_objects.contains_key(&b_key));
        assert!(!store.targets.contains_key(&b_key));
        assert!(!store.unresolved_targets.contains_key(&b_key));
        assert!(!store.bound_targets.contains_key(&b_key));
        let parent = store.scope_parents[&b_scope.parent];
        assert_eq!(parent.operation, save.operation);
        assert_eq!(parent.held, held);
        let before = observe(&mut store, sentinel.doc);
        let count = sizes(&store);
        let token = store.next_token;
        let next_save = store.next_save;
        let targets = store.targets.clone();
        let unresolved = store.unresolved_targets.clone();
        let bindings = store.bound_targets.clone();
        let leases = |map: &HashMap<TargetKey, ScopeLease>| {
            let mut rows: Vec<_> = map
                .iter()
                .map(|(key, lease)| (key.0.clone(), lease.operation, lease.held))
                .collect();
            rows.sort();
            rows
        };
        let parents = leases(&store.scope_parents);
        let objects = leases(&store.scope_objects);
        let failure = match capture.attach(&mut store) {
            Ok(_) => panic!("parent-only reservation did not block distinct B"),
            Err(failure) => failure,
        };
        let expected = if held {
            Error::UnresolvedConflict
        } else {
            Error::Busy
        };
        assert!(matches!(&failure.issue, AttachIssue::Owner(error) if error == &expected));
        let after = observe(&mut store, sentinel.doc);
        assert_eq!(before.state, after.state);
        assert_eq!(before.cursor, after.cursor);
        assert_eq!(sizes(&store), count);
        assert_eq!(store.next_token, token);
        assert_eq!(store.next_save, next_save);
        assert_eq!(store.targets, targets);
        assert_eq!(store.unresolved_targets, unresolved);
        assert_eq!(store.bound_targets, bindings);
        assert_eq!(leases(&store.scope_parents), parents);
        assert_eq!(leases(&store.scope_objects), objects);
        assert!(!store.scope_objects.contains_key(&b_key));
        assert!(!store.targets.contains_key(&b_key));
        assert!(!store.unresolved_targets.contains_key(&b_key));
        if !held {
            assert_eq!(store.captured_save_stage(&save), Ok(SaveStage::Encoding));
        }
        assert_eq!(fs::read(&a_path).unwrap(), b"file A");
        assert_eq!(fs::read(&b_path).unwrap(), b_raw);
        // Same B capture remains owned; the next branch rechecks it at owner entry.
        capture = failure.capture;
    }
}
#[test]
fn object_active_across_hardlink_parent_is_atomic_busy() {
    occupied_import(true, false);
}
#[test]
fn held_object_across_hardlink_parent_is_atomic_conflict() {
    occupied_import(true, true);
}
fn exhausted_import(doc_counter: bool) {
    let path = fixture(b"not imported");
    let mut store = Store::new(HistoryBudget::default());
    let sentinel = store
        .open_utf8("sentinel", Profile::default(), false)
        .unwrap();
    let save = store
        .begin_save(
            sentinel.doc,
            TargetKey("unrelated-fixture".into()),
            String::new(),
        )
        .unwrap();
    if doc_counter {
        store.verified_open_doc_counter = Some(Arc::new(AtomicU64::new(u64::MAX)));
    } else {
        store.next_token = u64::MAX;
    }
    let before = observe(&mut store, sentinel.doc);
    let count = sizes(&store);
    let token = store.next_token;
    let capture = ObservedTarget::inspect(&path)
        .unwrap()
        .open_existing(None)
        .unwrap();
    let failure = match capture.attach(&mut store) {
        Ok(_) => panic!("exhausted import accepted"),
        Err(e) => e,
    };
    assert!(matches!(
        &failure.issue,
        AttachIssue::Owner(Error::CounterExhausted)
    ));
    let after = observe(&mut store, sentinel.doc);
    assert_eq!(before.state, after.state);
    assert_eq!(before.cursor, after.cursor);
    assert_eq!(sizes(&store), count);
    assert_eq!(store.next_token, token);
    assert_eq!(store.captured_save_stage(&save), Ok(SaveStage::Encoding));
    assert_eq!(fs::read(path).unwrap(), b"not imported");
}
#[test]
fn token_exhaustion_has_no_partial_doc_binding_or_journal() {
    exhausted_import(false);
}
#[test]
fn doc_counter_exhaustion_has_no_partial_doc_binding_or_journal() {
    exhausted_import(true);
}
#[test]
fn legacy_key_without_live_identity_witness_requires_verification() {
    let path = fixture(b"old binding");
    let mut store = Store::new(HistoryBudget::default());
    let (n, _) = imported(&mut store, &path);
    // Models historical bindings which never supplied a retained file handle.
    store.opened_identities.remove(&n.doc);
    let before = observe(&mut store, n.doc);
    let count = sizes(&store);
    let token = store.next_token;
    let failure = match ObservedTarget::inspect(&path)
        .unwrap()
        .open_existing(None)
        .unwrap()
        .attach(&mut store)
    {
        Ok(_) => panic!("bare historical ID reused"),
        Err(e) => e,
    };
    assert!(matches!(&failure.issue, AttachIssue::ContinuityUnproven));
    let after = observe(&mut store, n.doc);
    assert_eq!(before.state, after.state);
    assert_eq!(before.cursor, after.cursor);
    assert_eq!(sizes(&store), count);
    assert_eq!(store.next_token, token);
}
#[test]
fn close_releases_identity_witness_and_allows_new_verified_document() {
    let path = fixture(b"close then reopen");
    let mut store = Store::new(HistoryBudget::default());
    let (n, _) = imported(&mut store, &path);
    store.close_document(n.doc).unwrap();
    assert_eq!(store.opened_identities.len(), 0);
    assert_eq!(store.bound_targets.len(), 0);
    assert_eq!(store.test_observer_document_count(), 0);
    let (next, _) = imported(&mut store, &path);
    assert_ne!(next.doc, n.doc);
}

#[test]
fn generic_save_receipt_invalidates_witness_and_requires_reverification() {
    let path = fixture(b"same disk bytes");
    let mut store = Store::new(HistoryBudget::default());
    let (n, _) = imported(&mut store, &path);
    let key = ObservedTarget::inspect(&path).unwrap().reservation_key();
    let save = store.begin_save(n.doc, key.clone(), String::new()).unwrap();
    store.advance_save(&save, SaveStage::WritingTemp).unwrap();
    store
        .advance_save(&save, SaveStage::ReadyToReplace)
        .unwrap();
    store.commit_permit(&save).unwrap();
    store.advance_save(&save, SaveStage::Verifying).unwrap();
    // Trusted memory receipt only; no write/flush/replace is performed here.
    store.accept_verified_save(&save).unwrap();
    assert!(!store.opened_identities.contains_key(&n.doc));
    assert_eq!(store.bound_targets.get(&key), Some(&n.doc));
    let before = observe(&mut store, n.doc);
    let count = sizes(&store);
    let failure = match ObservedTarget::inspect(&path)
        .unwrap()
        .open_existing(None)
        .unwrap()
        .attach(&mut store)
    {
        Ok(_) => panic!("save receipt was treated as a transferred lifetime witness"),
        Err(failure) => failure,
    };
    assert!(matches!(&failure.issue, AttachIssue::ContinuityUnproven));
    let after = observe(&mut store, n.doc);
    assert_eq!(before.state, after.state);
    assert_eq!(before.cursor, after.cursor);
    assert_eq!(sizes(&store), count);
    assert_eq!(fs::read(path).unwrap(), b"same disk bytes");
}
