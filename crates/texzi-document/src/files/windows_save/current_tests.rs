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
    let source_id = binding.disk.as_ref().unwrap().version.identity;
    drop(binding);
    assert!(
        prepared
            .source_anchor
            .as_ref()
            .unwrap()
            .retains(source_id)
            .unwrap()
    );
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

fn save_as_source_change(replace_id: bool) {
    let (path, mut store, n, binding) = fixture();
    if replace_id {
        fs::rename(&path, path.with_file_name("retained-original.txt")).unwrap();
        fs::write(&path, b"A\r\n").unwrap();
        assert_ne!(
            ObservedTarget::inspect(&path)
                .unwrap()
                .version()
                .unwrap()
                .identity,
            binding.disk.as_ref().unwrap().version.identity
        );
    } else {
        fs::write(&path, b"outside\r\n").unwrap();
    }
    let actual = fs::read(&path).unwrap();
    let before = observe(&mut store, n.doc);
    let error = failure(
        ObservedTarget::inspect(&path)
            .unwrap()
            .prepare_save_as(&binding, &mut store),
    );
    assert!(matches!(
        &error.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    unchanged(&mut store, &before, &error);
    let destination = path.with_file_name("genuine-copy.txt");
    let prepared = ObservedTarget::inspect(&destination)
        .unwrap()
        .prepare_save_as(&binding, &mut store)
        .unwrap();
    assert_eq!(prepared.capture().operation, 1);
    store.cancel_save(prepared.capture()).unwrap();
    assert!(!destination.exists());
    assert_eq!(fs::read(path).unwrap(), actual);
}
#[test]
fn save_as_source_path_cannot_bypass_external_bytes_check() {
    save_as_source_change(false);
}
#[test]
fn save_as_source_path_cannot_bypass_replaced_file_id_check() {
    save_as_source_change(true);
}
#[test]
fn save_as_hardlink_alias_is_rejected_even_when_source_version_is_unchanged() {
    let (path, mut store, n, binding) = fixture();
    let alias = path.with_file_name("alias.txt");
    fs::hard_link(&path, &alias).unwrap();
    let observed = ObservedTarget::inspect(&alias).unwrap();
    assert_eq!(
        observed.version().unwrap().identity,
        binding.disk.as_ref().unwrap().version.identity
    );
    assert_ne!(
        observed.resolved_path(),
        binding.disk.as_ref().unwrap().path
    );
    let before = observe(&mut store, n.doc);
    let error = failure(observed.prepare_save_as(&binding, &mut store));
    assert!(matches!(
        &error.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    unchanged(&mut store, &before, &error);
    // Same canonical path is routed through Current and passes only when unchanged.
    let prepared = ObservedTarget::inspect(&path)
        .unwrap()
        .prepare_save_as(&binding, &mut store)
        .unwrap();
    assert_eq!(prepared.capture().operation, 1);
    store.cancel_save(prepared.capture()).unwrap();
    assert_eq!(fs::read(path).unwrap(), b"A\r\n");
    assert_eq!(fs::read(alias).unwrap(), b"A\r\n");
}
#[test]
fn initial_memory_binding_is_allowed_but_cannot_be_reused_after_real_publication() {
    let (path, mut store, _, _) = fixture();
    let destination = path.with_file_name("first-published.txt");
    let (n, old) = LoadedBytes::decode(Arc::from(&b"memory N"[..]), Some(Encoding::Utf8))
        .unwrap()
        .attach(&mut store)
        .unwrap();
    assert!(old.disk.is_none());
    let ready = ObservedTarget::inspect(&destination)
        .unwrap()
        .prepare_bound(&old, &mut store)
        .unwrap()
        .begin(&mut store)
        .unwrap()
        .write()
        .unwrap()
        .lock_temporary()
        .unwrap()
        .prepare_new_owner_only()
        .unwrap();
    let published = ready
        .permit(&mut store)
        .unwrap()
        .run()
        .unwrap()
        .accept(&mut store)
        .unwrap();
    assert!(published.binding.disk.is_some());
    assert!(old.disk.is_none());
    let dirty = store
        .submit(Transaction {
            doc: n.doc,
            base: n.revision,
            edits: vec![Edit::input(&n, 0..0, "dirty ")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    let before = observe(&mut store, n.doc);
    let error = failure(
        ObservedTarget::inspect(&destination)
            .unwrap()
            .prepare_bound(&old, &mut store),
    );
    assert!(matches!(
        &error.issue,
        BoundIssue::File(FileError::UnverifiedDiskBinding)
    ));
    unchanged(&mut store, &before, &error);
    let other = path.with_file_name("old-binding-copy.txt");
    let error = failure(
        ObservedTarget::inspect(&other)
            .unwrap()
            .prepare_save_as(&old, &mut store),
    );
    assert!(matches!(
        &error.issue,
        BoundIssue::File(FileError::UnverifiedDiskBinding)
    ));
    unchanged(&mut store, &before, &error);
    assert!(!other.exists());
    // New disk binding still describes saved N, and encodes dirty N+1 at preparation.
    let prepared = ObservedTarget::inspect(&destination)
        .unwrap()
        .prepare_current(&published.binding, &mut store)
        .unwrap();
    assert_eq!(prepared.capture().operation, 2);
    assert_eq!(prepared.capture().snapshot.token, dirty.token);
    assert_eq!(prepared.prepared.bytes(), b"dirty memory N");
    store.cancel_save(prepared.capture()).unwrap();
    assert!(store.dirty(n.doc).unwrap());
    assert_eq!(fs::read(destination).unwrap(), b"memory N");
}
#[test]
fn closed_disk_and_memory_docs_are_rejected_before_metadata_indexing() {
    let (path, mut store, n, disk) = fixture();
    let (memory, legacy) = LoadedBytes::decode(Arc::from(&b"memory"[..]), Some(Encoding::Utf8))
        .unwrap()
        .attach(&mut store)
        .unwrap();
    store.close_document(n.doc).unwrap();
    store.close_document(memory.doc).unwrap();
    for binding in [&disk, &legacy] {
        for mode in 0..3 {
            let observed = ObservedTarget::inspect(&path).unwrap();
            let result = match mode {
                0 => observed.prepare_bound(binding, &mut store),
                1 => observed.prepare_current(binding, &mut store),
                _ => observed.prepare_save_as(binding, &mut store),
            };
            let error = failure(result);
            assert!(matches!(
                &error.issue,
                BoundIssue::Document(Error::InvalidDoc)
            ));
            assert!(error.capture.is_none());
            assert!(error.recovery_temp.is_none());
        }
    }
    assert_eq!(fs::read(path).unwrap(), b"A\r\n");
}

#[test]
fn save_as_case_alias_of_replaced_source_cannot_skip_current_verification() {
    let (path, mut store, n, binding) = fixture();
    fs::rename(&path, path.with_file_name("old-object.txt")).unwrap();
    fs::write(&path, b"replacement").unwrap();
    let alias = path.with_file_name("ORIGINAL.TXT");
    let observed = ObservedTarget::inspect(&alias).unwrap();
    // This fixture targets the normal case-insensitive Windows temp directory.
    assert!(
        observed.version().is_some(),
        "case-insensitive fixture precondition"
    );
    assert_ne!(
        observed.version().unwrap().identity,
        binding.disk.as_ref().unwrap().version.identity
    );
    let before = observe(&mut store, n.doc);
    let error = failure(observed.prepare_save_as(&binding, &mut store));
    assert!(matches!(
        &error.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    unchanged(&mut store, &before, &error);
    assert_eq!(fs::read(path).unwrap(), b"replacement");
}
#[test]
fn absent_unicode_sibling_requires_name_verification_but_distinct_parent_is_allowed() {
    let (path, mut store, original, old_binding) = fixture();
    let unicode = path.with_file_name("文档.txt");
    fs::rename(&path, &unicode).unwrap();
    store.close_document(original.doc).unwrap();
    drop(old_binding);
    let (n, binding) = match ObservedTarget::inspect(&unicode)
        .unwrap()
        .open_existing(None)
        .unwrap()
        .attach(&mut store)
        .unwrap()
    {
        OpenOutcome::Imported { snapshot, binding } => (snapshot, binding),
        _ => panic!("new Unicode source import expected"),
    };
    let sibling = unicode.with_file_name("另存.txt");
    let before = observe(&mut store, n.doc);
    let error = failure(
        ObservedTarget::inspect(&sibling)
            .unwrap()
            .prepare_save_as(&binding, &mut store),
    );
    assert!(matches!(
        &error.issue,
        BoundIssue::Identity(IdentityError::InvalidPath)
    ));
    unchanged(&mut store, &before, &error);
    assert!(!sibling.exists());
    let other_parent = unicode.parent().unwrap().join("different-parent");
    fs::create_dir(&other_parent).unwrap();
    let destination = other_parent.join("另存.txt");
    let prepared = ObservedTarget::inspect(&destination)
        .unwrap()
        .prepare_save_as(&binding, &mut store)
        .unwrap();
    assert_eq!(prepared.capture().operation, 1);
    store.cancel_save(prepared.capture()).unwrap();
    assert!(!destination.exists());
    assert_eq!(fs::read(unicode).unwrap(), b"A\r\n");
}

fn generated_directory_bytes(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut entries: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}
#[test]
fn save_as_absent_source_is_not_recreated_or_reserved() {
    let (path, mut store, n, binding) = fixture();
    let retained = path.with_file_name("retained-before-delete.txt");
    fs::rename(&path, &retained).unwrap();
    let observed = ObservedTarget::inspect(&path).unwrap();
    assert!(observed.version().is_none());
    let bytes = generated_directory_bytes(&path);
    let before = observe(&mut store, n.doc);
    let error = failure(observed.prepare_save_as(&binding, &mut store));
    assert!(matches!(
        &error.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    unchanged(&mut store, &before, &error);
    assert!(!path.exists());
    assert_eq!(generated_directory_bytes(&path), bytes);
    assert_eq!(fs::read(retained).unwrap(), b"A\r\n");
    let destination = path.with_file_name("genuine-after-missing.txt");
    let prepared = ObservedTarget::inspect(&destination)
        .unwrap()
        .prepare_save_as(&binding, &mut store)
        .unwrap();
    assert_eq!(prepared.capture().operation, 1);
    store.cancel_save(prepared.capture()).unwrap();
    assert!(!path.exists());
    assert!(!destination.exists());
    assert_eq!(generated_directory_bytes(&path), bytes);
}
#[test]
fn dirty_unbound_first_publication_uses_baseline_not_current_token_and_keeps_later_edit() {
    let (path, mut store, _, _) = fixture();
    let destination = path.with_file_name("dirty-first-publication.txt");
    let (initial, memory) = LoadedBytes::decode(Arc::from(&b"A"[..]), Some(Encoding::Utf8))
        .unwrap()
        .attach(&mut store)
        .unwrap();
    let edited = store
        .submit(Transaction {
            doc: initial.doc,
            base: initial.revision,
            edits: vec![Edit::input(&initial, 0..0, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    let mut profile = edited.body.profile;
    profile.file_format = Some(Format {
        encoding: Encoding::Utf16Be,
        bom: true,
    });
    let n = store
        .set_serialization(edited.doc, edited.revision, profile, None)
        .unwrap()
        .snapshot;
    assert_ne!(n.token, initial.token);
    assert!(store.dirty(n.doc).unwrap());
    assert!(memory.disk.is_none());
    let before = observe(&mut store, n.doc);
    assert_eq!(before.state.global.metadata.baseline.token, initial.token);
    assert_eq!(before.state.global.metadata.attention.target, None);
    assert_eq!(before.state.global.metadata.attention.binding_epoch, 0);
    assert!(!before.state.global.metadata.baseline.backed_by_disk);
    let prepared = ObservedTarget::inspect(&destination)
        .unwrap()
        .prepare_bound(&memory, &mut store)
        .unwrap();
    assert_eq!(prepared.capture().operation, 1);
    assert_eq!(prepared.capture().snapshot.token, n.token);
    assert_eq!(prepared.prepared.bytes(), [254, 255, 0, 66, 0, 65]);
    let later = store
        .submit(Transaction {
            doc: n.doc,
            base: n.revision,
            edits: vec![Edit::input(&n, 2..2, "C")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    let depth = store.history_depth(n.doc).unwrap();
    // This is real native publication only when a future Windows harness runs it.
    let result = prepared
        .begin(&mut store)
        .unwrap()
        .write()
        .unwrap()
        .lock_temporary()
        .unwrap()
        .prepare_new_owner_only()
        .unwrap()
        .permit(&mut store)
        .unwrap()
        .run()
        .unwrap()
        .accept(&mut store)
        .unwrap();
    assert_eq!(fs::read(&destination).unwrap(), [254, 255, 0, 66, 0, 65]);
    assert_eq!(result.binding_epoch, 1);
    assert_eq!(result.saved_snapshot.token, n.token);
    assert_eq!(result.binding.format(), profile.file_format.unwrap());
    assert_eq!(result.binding.disk.as_ref().unwrap().epoch, 1);
    assert_eq!(store.snapshot(n.doc).unwrap().token, later.token);
    assert_eq!(store.history_depth(n.doc).unwrap(), depth);
    assert!(store.dirty(n.doc).unwrap());
    let after = observe(&mut store, n.doc);
    assert_eq!(after.state.global.metadata.baseline.token, n.token);
    let next = ObservedTarget::inspect(&destination)
        .unwrap()
        .prepare_current(&result.binding, &mut store)
        .unwrap();
    assert_eq!(next.capture().operation, 2);
    assert_eq!(next.capture().snapshot.token, later.token);
    assert_eq!(next.prepared.bytes(), [254, 255, 0, 66, 0, 65, 0, 67]);
    store.cancel_save(next.capture()).unwrap();
    assert!(store.dirty(n.doc).unwrap());
    assert_eq!(fs::read(path).unwrap(), b"A\r\n");
}
