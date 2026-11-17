//! Generated fixtures; save receipts are not disk-write evidence.
use super::*;
use crate::files::{Encoding, LoadedBytes, windows_identity::OpenOutcome};
use crate::{BoundaryPolicy, Edit, HistoryBudget, ObserverScope, Transaction};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
fn fixture() -> (PathBuf, Store, crate::Snapshot, FileBinding) {
    let dir = std::env::temp_dir().join(format!(
        "texzi-current-binding-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    let path = dir.join("original.txt");
    fs::write(&path, b"A\r\n").unwrap();
    eprintln!(
        "T07 generated current-binding fixture retained: {}",
        dir.display()
    );
    let mut store = Store::new(HistoryBudget::default());
    let (snapshot, binding) = match ObservedTarget::inspect(&path)
        .unwrap()
        .open_existing(None)
        .unwrap()
        .attach(&mut store)
        .unwrap()
    {
        OpenOutcome::Imported { snapshot, binding } => (snapshot, binding),
        _ => panic!("new import expected"),
    };
    (path, store, snapshot, binding)
}
fn failure(result: Result<BoundPrepared, BoundFailure>) -> BoundFailure {
    match result {
        Ok(_) => panic!("unexpected save preparation"),
        Err(failure) => failure,
    }
}
fn observe(store: &mut Store, doc: crate::DocId) -> crate::ObserverSnapshot {
    store
        .observe_and_subscribe(doc, ObserverScope::new(vec![]), 8)
        .unwrap()
        .0
}
fn unchanged(store: &mut Store, before: &crate::ObserverSnapshot, failure: &BoundFailure) {
    assert!(failure.capture.is_none());
    assert!(failure.recovery_temp.is_none());
    let after = observe(store, before.state.global.document.doc);
    assert_eq!(before.state, after.state);
    assert_eq!(before.cursor, after.cursor);
}
#[test]
fn current_capture_freezes_format_and_n_while_n_plus_one_stays_dirty() {
    let (path, mut store, n, binding) = fixture();
    let mut profile = n.body.profile;
    profile.file_format = Some(Format {
        encoding: Encoding::Utf16Le,
        bom: true,
    });
    let n = store
        .set_serialization(n.doc, n.revision, profile, None)
        .unwrap()
        .snapshot;
    let prepared = ObservedTarget::inspect(&path)
        .unwrap()
        .prepare_current(&binding, &mut store)
        .unwrap();
    assert_eq!(prepared.capture().snapshot.token, n.token);
    assert_eq!(prepared.prepared.format(), profile.file_format.unwrap());
    assert_eq!(prepared.prepared.bytes(), [255, 254, 65, 0, 13, 0, 10, 0]);
    let next = store
        .submit(Transaction {
            doc: n.doc,
            base: n.revision,
            edits: vec![Edit::input(&n, 0..0, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    assert_eq!(prepared.capture().snapshot.token, n.token);
    assert_eq!(prepared.prepared.bytes(), [255, 254, 65, 0, 13, 0, 10, 0]);
    store.cancel_save(prepared.capture()).unwrap();
    assert_eq!(store.snapshot(n.doc).unwrap().token, next.token);
    assert!(store.dirty(n.doc).unwrap());
    assert_eq!(fs::read(path).unwrap(), b"A\r\n"); // No write or saved receipt.
}
#[test]
fn external_bytes_are_rejected_before_reservation_but_explicit_save_as_is_allowed() {
    let (path, mut store, n, binding) = fixture();
    fs::write(&path, b"external\r\n").unwrap();
    let before = observe(&mut store, n.doc);
    let error = failure(
        ObservedTarget::inspect(&path)
            .unwrap()
            .prepare_bound(&binding, &mut store),
    );
    assert!(matches!(
        &error.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    unchanged(&mut store, &before, &error);
    let target = path.with_file_name("copy.txt");
    let copy = ObservedTarget::inspect(&target)
        .unwrap()
        .prepare_save_as(&binding, &mut store)
        .unwrap();
    assert_eq!(copy.capture().operation, 1);
    assert_eq!(copy.capture().snapshot.token, n.token);
    store.cancel_save(copy.capture()).unwrap();
    assert!(!target.exists());
    assert_eq!(fs::read(path).unwrap(), b"external\r\n");
}
#[test]
fn replacement_identity_with_equal_bytes_is_rejected_before_reservation() {
    let (path, mut store, n, binding) = fixture();
    let backup = path.with_file_name("external-original.txt");
    fs::rename(&path, &backup).unwrap();
    fs::write(&path, b"A\r\n").unwrap();
    let observed = ObservedTarget::inspect(&path).unwrap();
    assert_ne!(
        observed.version().unwrap().identity,
        binding.disk.as_ref().unwrap().version.identity
    );
    let before = observe(&mut store, n.doc);
    let error = failure(observed.prepare_current(&binding, &mut store));
    assert!(matches!(
        &error.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    unchanged(&mut store, &before, &error);
    assert_eq!(fs::read(path).unwrap(), b"A\r\n");
    assert_eq!(fs::read(backup).unwrap(), b"A\r\n");
}
#[test]
fn stale_generation_rejects_current_and_save_as_without_state_changes() {
    let (path, mut store, n, binding) = fixture();
    let saved = store
        .begin_save(
            n.doc,
            ObservedTarget::inspect(&path).unwrap().reservation_key(),
            String::new(),
        )
        .unwrap();
    store.advance_save(&saved, SaveStage::WritingTemp).unwrap();
    store
        .advance_save(&saved, SaveStage::ReadyToReplace)
        .unwrap();
    store.commit_permit(&saved).unwrap();
    store.advance_save(&saved, SaveStage::Verifying).unwrap();
    store.accept_verified_save(&saved).unwrap(); // Memory receipt only; no file write.
    let before = observe(&mut store, n.doc);
    let error = failure(
        ObservedTarget::inspect(&path)
            .unwrap()
            .prepare_current(&binding, &mut store),
    );
    assert!(matches!(
        &error.issue,
        BoundIssue::File(FileError::StaleDiskBinding)
    ));
    unchanged(&mut store, &before, &error);
    let target = path.with_file_name("new.txt");
    let error = failure(
        ObservedTarget::inspect(&target)
            .unwrap()
            .prepare_save_as(&binding, &mut store),
    );
    assert!(matches!(
        &error.issue,
        BoundIssue::File(FileError::StaleDiskBinding)
    ));
    unchanged(&mut store, &before, &error);
    assert!(!target.exists());
    assert_eq!(fs::read(path).unwrap(), b"A\r\n");
}
#[test]
fn different_destination_requires_explicit_save_as_and_keeps_old_binding_on_cancel() {
    let (path, mut store, n, binding) = fixture();
    let target = path.with_file_name("save-as.txt");
    let before = observe(&mut store, n.doc);
    let error = failure(
        ObservedTarget::inspect(&target)
            .unwrap()
            .prepare_bound(&binding, &mut store),
    );
    assert!(matches!(
        &error.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    unchanged(&mut store, &before, &error);
    let prepared = ObservedTarget::inspect(&target)
        .unwrap()
        .prepare_save_as(&binding, &mut store)
        .unwrap();
    assert_eq!(prepared.capture().operation, 1);
    assert_eq!(prepared.capture().binding_epoch, 1);
    assert_eq!(prepared.prepared.bytes(), b"A\r\n");
    store.cancel_save(prepared.capture()).unwrap();
    let after = observe(&mut store, n.doc);
    assert_eq!(
        before.state.global.metadata.baseline,
        after.state.global.metadata.baseline
    );
    assert_eq!(
        before.state.global.metadata.attention.target,
        after.state.global.metadata.attention.target
    );
    assert_eq!(
        before.state.global.metadata.attention.binding_epoch,
        after.state.global.metadata.attention.binding_epoch
    );
    assert!(!target.exists());
    assert_eq!(fs::read(path).unwrap(), b"A\r\n");
}
#[test]
fn decoded_memory_import_is_not_a_verified_current_file_binding() {
    let (path, mut store, _, _) = fixture();
    let (n, binding) = LoadedBytes::decode(Arc::from(&b"memory"[..]), Some(Encoding::Utf8))
        .unwrap()
        .attach(&mut store)
        .unwrap();
    assert!(binding.disk.is_none());
    let before = observe(&mut store, n.doc);
    let error = failure(
        ObservedTarget::inspect(&path)
            .unwrap()
            .prepare_current(&binding, &mut store),
    );
    assert!(matches!(
        &error.issue,
        BoundIssue::File(FileError::UnverifiedDiskBinding)
    ));
    unchanged(&mut store, &before, &error);
    assert_eq!(fs::read(path).unwrap(), b"A\r\n");
}
