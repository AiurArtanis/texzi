use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use texzi_document::{
    files::{windows_identity::*, windows_save::*, *},
    *,
};
fn fixture() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "texzi-t07-bound-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&dir).unwrap();
    eprintln!("T07 generated bound fixture retained: {}", dir.display());
    dir
}
fn input(store: &mut Store, snapshot: &Snapshot, at: usize, text: &str) -> Snapshot {
    store
        .submit(Transaction {
            doc: snapshot.doc,
            base: snapshot.revision,
            edits: vec![Edit::input(snapshot, at..at, text)],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot
}
fn loaded(store: &mut Store) -> (Snapshot, FileBinding) {
    LoadedBytes::decode(Arc::from(&b"A"[..]), Some(Encoding::Utf8))
        .unwrap()
        .attach(store)
        .unwrap()
}
#[test]
fn bound_worker_retains_target_format_payload_and_rejects_cancelled_late_result() {
    let dir = fixture();
    let target = dir.join("original.txt");
    fs::write(&target, b"A").unwrap();
    let mut store = Store::new(HistoryBudget::default());
    let (initial, binding) = loaded(&mut store);
    let captured = input(&mut store, &initial, 1, "B");
    let prepared = ObservedTarget::inspect(&target)
        .unwrap()
        .prepare_bound(&binding, &mut store)
        .unwrap();
    let capture = prepared.capture().clone();
    let work = prepared.begin(&mut store).unwrap();
    let current = input(&mut store, &captured, 2, "C");
    let staged = work.write().unwrap();
    assert_eq!(staged.payload(), b"AB");
    assert_eq!(staged.format(), captured.body.profile.file_format.unwrap());
    assert_eq!(staged.capture().snapshot.token, captured.token);
    assert_eq!(store.snapshot(initial.doc).unwrap().token, current.token);
    assert_ne!(
        staged.original_identity(),
        Some(staged.temporary_identity())
    );
    assert_eq!(
        staged.parent_identity().volume,
        staged.temporary_identity().volume
    );
    staged.check_owner(&store).unwrap();
    staged.recheck_files().unwrap();
    assert_eq!(store.commit_permit(&capture), Err(Error::InvalidSave));
    assert_eq!(fs::read(&target).unwrap(), b"A");
    store.cancel_save(&capture).unwrap();
    let next = ObservedTarget::inspect(&target)
        .unwrap()
        .prepare_bound(&binding, &mut store)
        .unwrap();
    assert_eq!(staged.check_owner(&store), Err(Error::InvalidSave));
    store.cancel_save(next.capture()).unwrap();
    assert_eq!(staged.read_created_bytes().unwrap(), b"AB");
    assert_eq!(fs::read(&target).unwrap(), b"A");
}
#[test]
fn bound_worker_detects_changed_target_before_creating_temp_and_returns_capture() {
    let dir = fixture();
    let target = dir.join("original.txt");
    fs::write(&target, b"A").unwrap();
    let mut store = Store::new(HistoryBudget::default());
    let (s, binding) = loaded(&mut store);
    let prepared = ObservedTarget::inspect(&target)
        .unwrap()
        .prepare_bound(&binding, &mut store)
        .unwrap();
    let capture = prepared.capture().clone();
    let work = prepared.begin(&mut store).unwrap();
    fs::write(&target, b"external").unwrap();
    let failure = match work.write() {
        Ok(_) => panic!("changed target accepted"),
        Err(e) => e,
    };
    assert!(matches!(
        failure.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    assert_eq!(
        failure.capture.as_ref().unwrap().operation,
        capture.operation
    );
    assert!(failure.recovery_temp.is_none());
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    store
        .finish_save_failure(
            &capture,
            SaveFailure {
                outcome: SaveFailureOutcome::BeforeReplace,
                conflict: true,
                os_error: None,
                message: "fixture external change".into(),
                recovery_paths: vec![],
            },
        )
        .unwrap();
    assert_eq!(store.snapshot(s.doc).unwrap().token, s.token);
    assert_eq!(fs::read(&target).unwrap(), b"external");
}
#[test]
fn bound_creation_handle_blocks_same_byte_substitution_before_lock_validation() {
    let dir = fixture();
    let target = dir.join("original.txt");
    fs::write(&target, b"A").unwrap();
    let mut store = Store::new(HistoryBudget::default());
    let (_, binding) = loaded(&mut store);
    let staged = ObservedTarget::inspect(&target)
        .unwrap()
        .prepare_bound(&binding, &mut store)
        .unwrap()
        .begin(&mut store)
        .unwrap()
        .write()
        .unwrap();
    let temp = staged.recovery_temp().to_owned();
    let creation_id = staged.temporary_identity();
    let substitute = dir.join("same-bytes-different-file.txt");
    fs::write(&substitute, staged.payload()).unwrap();
    assert_ne!(
        ObservedTarget::inspect(&substitute)
            .unwrap()
            .version()
            .unwrap()
            .identity,
        creation_id
    );
    assert_eq!(
        fs::write(&temp, b"external temporary bytes")
            .unwrap_err()
            .raw_os_error(),
        Some(32)
    );
    assert_eq!(
        fs::rename(&temp, dir.join("moved-temp.txt"))
            .unwrap_err()
            .raw_os_error(),
        Some(32)
    );
    assert_eq!(
        fs::rename(&substitute, &temp).unwrap_err().raw_os_error(),
        Some(32)
    );
    assert!(
        matches!(ObservedTarget::inspect(&temp), Err(IdentityError::Io(e)) if e.raw_os_error() == Some(32))
    );
    assert_eq!(staged.temporary_identity(), creation_id);
    staged.recheck_files().unwrap();
    assert_eq!(
        store.commit_permit(staged.capture()),
        Err(Error::InvalidSave)
    );
    store.cancel_save(staged.capture()).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"A");
    assert_eq!(staged.read_created_bytes().unwrap(), b"A");
    assert_eq!(fs::read(&substitute).unwrap(), b"A");
    drop(staged);
    fs::write(&temp, b"fixture after creation handle closes").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"A");
}

#[test]
fn locked_temporary_observes_security_and_denies_ordinary_write_delete_and_rename() {
    let dir = fixture();
    let target = dir.join("original.txt");
    fs::write(&target, b"A").unwrap();
    let mut store = Store::new(HistoryBudget::default());
    let (_, binding) = loaded(&mut store);
    let staged = ObservedTarget::inspect(&target)
        .unwrap()
        .prepare_bound(&binding, &mut store)
        .unwrap()
        .begin(&mut store)
        .unwrap()
        .write()
        .unwrap();
    let temporary = staged.recovery_temp().to_owned();
    let observed_security = staged.temporary_security().clone();
    let locked = staged.lock_temporary().unwrap();
    locked.check_owner(&store).unwrap();
    locked.recheck_files().unwrap();
    assert_eq!(locked.temporary_security(), &observed_security);
    assert!(!locked.temporary_security().as_bytes().is_empty());
    assert!(!locked.original_security().unwrap().as_bytes().is_empty());
    assert_eq!(
        locked.original_security().unwrap(),
        locked.temporary_security()
    );
    // Stop at the first unexpected success; do not erase its fixture evidence.
    assert_eq!(
        fs::write(&temporary, b"must not truncate")
            .unwrap_err()
            .raw_os_error(),
        Some(32)
    );
    assert_eq!(
        fs::remove_file(&temporary).unwrap_err().raw_os_error(),
        Some(32)
    );
    assert_eq!(
        fs::rename(&temporary, dir.join("must-not-rename.txt"))
            .unwrap_err()
            .raw_os_error(),
        Some(32)
    );
    assert_eq!(fs::read(&temporary).unwrap_err().raw_os_error(), Some(32));
    assert_eq!(locked.read_created_bytes().unwrap(), b"A");
    assert_eq!(fs::read(&target).unwrap(), b"A");
    assert_eq!(
        store.commit_permit(locked.capture()),
        Err(Error::InvalidSave)
    );
    store.cancel_save(locked.capture()).unwrap();
    drop(locked); // Explicit resource lifetime; never deletes the recovery file.
    fs::write(&temporary, b"fixture after unlock").unwrap();
    assert_eq!(fs::read(&temporary).unwrap(), b"fixture after unlock");
    assert_eq!(fs::read(&target).unwrap(), b"A");
}
