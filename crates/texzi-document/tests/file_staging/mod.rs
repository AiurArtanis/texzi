use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use texzi_document::{
    files::{staging::*, *},
    *,
};
fn fixture() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "texzi-t07-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&path).unwrap();
    eprintln!("T07 generated fixture retained: {}", path.display());
    path
}
fn prepared(store: &mut Store, raw: &[u8], target: &std::path::Path) -> (Snapshot, PreparedSave) {
    let loaded = LoadedBytes::decode(Arc::from(raw), Some(Encoding::Utf8)).unwrap();
    let format = loaded.format();
    let (s, b) = loaded.attach(store).unwrap();
    let p = b
        .prepare(
            store,
            TargetKey(target.to_string_lossy().into_owned()),
            "fixture-content-guard".into(),
            format,
        )
        .unwrap();
    (s, p)
}
#[test]
fn staged_payload_is_captured_and_never_replaces_original() {
    let dir = fixture();
    let target = dir.join("original.txt");
    fs::write(&target, b"old\r\n").unwrap();
    let mut store = Store::new(HistoryBudget::default());
    let (s, p) = prepared(&mut store, b"new\r\n", &target);
    let work = p.begin_staging(&mut store).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let worker_barrier = barrier.clone();
    let worker_target = target.clone();
    let worker = std::thread::spawn(move || {
        worker_barrier.wait();
        work.write_temp(
            &worker_target,
            &ContentGuard::Exact(Arc::from(&b"old\r\n"[..])),
        )
    });
    store
        .submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![Edit::input(&s, 0..0, "later")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap();
    barrier.wait();
    let staged = worker.join().unwrap().unwrap();
    assert_eq!(staged.capture().snapshot.token, s.token);
    assert_eq!(staged.expected_bytes(), b"new\r\n");
    assert_eq!(staged.recovery_temp().parent(), target.parent());
    assert_eq!(fs::read(staged.recovery_temp()).unwrap(), b"new\r\n");
    assert_eq!(fs::read(&target).unwrap(), b"old\r\n");
    assert!(store.dirty(s.doc).unwrap());
    assert_eq!(store.commit_permit(p.capture()), Err(Error::InvalidSave));
    assert_eq!(
        store.accept_verified_save(p.capture()),
        Err(Error::InvalidSave)
    );
    assert!(p.begin_staging(&mut store).is_err());
    store.cancel_save(p.capture()).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"old\r\n");
}
#[test]
fn conflict_and_save_as_staging_do_not_overwrite_a_target() {
    let dir = fixture();
    let target = dir.join("save-as.txt");
    let mut store = Store::new(HistoryBudget::default());
    let (s, p) = prepared(&mut store, b"payload", &target);
    fs::write(&target, b"external").unwrap();
    assert!(matches!(
        p.begin_staging(&mut store)
            .unwrap()
            .write_temp(&target, &ContentGuard::Absent),
        Err(StageFailure {
            issue: StageIssue::Conflict,
            ..
        })
    ));
    assert_eq!(fs::read(&target).unwrap(), b"external");
    assert!(matches!(
        store.begin_save(s.doc, TargetKey("other".into()), String::new()),
        Err(Error::Busy)
    ));
    store.cancel_save(p.capture()).unwrap();
    let new_target = dir.join("absent.txt");
    let (_, p) = prepared(&mut store, b"payload", &new_target);
    let staged = p
        .begin_staging(&mut store)
        .unwrap()
        .write_temp(&new_target, &ContentGuard::Absent)
        .unwrap();
    assert!(!new_target.exists());
    assert_eq!(fs::read(staged.recovery_temp()).unwrap(), b"payload");
    store.cancel_save(p.capture()).unwrap();
}
#[test]
fn read_only_uses_original_bytes_and_preserves_generated_file() {
    let dir = fixture();
    let path = dir.join("utf16.txt");
    let raw = [0xff, 0xfe, 65, 0, 13, 0, 10, 0];
    fs::write(&path, raw).unwrap();
    let before = fs::metadata(&path).unwrap();
    let loaded = LoadedBytes::read_only(&path, None).unwrap();
    assert_eq!(loaded.original(), raw);
    let format = loaded.format();
    let mut store = Store::new(HistoryBudget::default());
    let (s, b) = loaded.attach(&mut store).unwrap();
    assert!(!store.backed_by_disk(s.doc).unwrap()); // No verified file identity yet.
    assert_eq!(&*b.encode_snapshot(&s, format).unwrap(), raw);
    assert_eq!(fs::read(&path).unwrap(), raw);
    let after = fs::metadata(&path).unwrap();
    assert_eq!(before.len(), after.len());
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
}
#[test]
fn late_cancelled_worker_cannot_advance_a_new_save() {
    let dir = fixture();
    let target = dir.join("late.txt");
    let mut store = Store::new(HistoryBudget::default());
    let (s, p) = prepared(&mut store, b"captured", &target);
    let work = p.begin_staging(&mut store).unwrap();
    store.cancel_save(p.capture()).unwrap();
    let current = store
        .begin_save(
            s.doc,
            TargetKey(target.to_string_lossy().into_owned()),
            "new-guard".into(),
        )
        .unwrap();
    let late = work.write_temp(&target, &ContentGuard::Absent).unwrap();
    assert_eq!(
        store.advance_save(late.capture(), SaveStage::ReadyToReplace),
        Err(Error::InvalidSave)
    );
    assert!(matches!(
        store.begin_save(s.doc, TargetKey("still-busy".into()), String::new()),
        Err(Error::Busy)
    ));
    assert!(!target.exists());
    assert_eq!(std::fs::read(late.recovery_temp()).unwrap(), b"captured");
    store.cancel_save(&current).unwrap();
}

#[test]
fn temp_target_collision_is_excluded_before_creation_and_preserves_save_state() {
    // Fresh Store: first operation is 1. Freeze exact candidate names, including
    // a later attempt reached only after a preexisting temp is encountered.
    for case in 0..4 {
        let dir = fixture();
        let stem = (u64::from(std::process::id()) + 1) % 100_000_000;
        let target = dir.join(match case {
            0 => format!("{stem:08}.000"),
            1 => format!("{stem:08}.001"),
            2 => format!(".texzi-{}-1-0.tmp", std::process::id()),
            _ => format!(".TEXZI-{}-1-0.TMP", std::process::id()),
        });
        let occupied = dir.join(format!("{stem:08}.000"));
        if case == 1 {
            fs::write(&occupied, b"existing temporary bytes").unwrap();
        }
        let mut store = Store::new(HistoryBudget::default());
        let (s, p) = prepared(&mut store, b"captured", &target);
        assert_eq!(p.capture().operation, 1);
        let work = p.begin_staging(&mut store).unwrap();
        let changed = store
            .submit(Transaction {
                doc: s.doc,
                base: s.revision,
                edits: vec![Edit::input(&s, 8..8, "-N+1")],
                boundary: BoundaryPolicy::Grapheme,
                cancelled: false,
            })
            .unwrap()
            .snapshot;
        let depth = store.history_depth(s.doc).unwrap();
        let (before, _) = store
            .observe_and_subscribe(s.doc, ObserverScope::new(vec![]), 8)
            .unwrap();
        let staged = work.write_temp(&target, &ContentGuard::Absent).unwrap();
        assert_ne!(staged.recovery_temp(), target);
        assert!(!target.exists(), "no pre-permit publication in case {case}");
        assert_eq!(fs::read(staged.recovery_temp()).unwrap(), b"captured");
        if case == 0 {
            assert_eq!(staged.recovery_temp(), dir.join(format!("{stem:08}.001")));
        }
        if case == 1 {
            assert_eq!(staged.recovery_temp(), dir.join(format!("{stem:08}.002")));
            assert_eq!(fs::read(&occupied).unwrap(), b"existing temporary bytes");
        }
        let (after, _) = store
            .observe_and_subscribe(s.doc, ObserverScope::new(vec![]), 8)
            .unwrap();
        assert_eq!(
            before.state.global.metadata.baseline,
            after.state.global.metadata.baseline
        );
        assert_eq!(after.state.global.document.token, changed.token);
        assert_eq!(after.state.global.document.body.to_utf8(), b"captured-N+1");
        assert_eq!(store.history_depth(s.doc).unwrap(), depth);
        assert!(store.dirty(s.doc).unwrap());
        assert_eq!(store.commit_permit(p.capture()), Err(Error::InvalidSave));
        assert!(matches!(
            store.begin_save(s.doc, TargetKey("other".into()), String::new()),
            Err(Error::Busy)
        ));
        store.cancel_save(p.capture()).unwrap();
        assert!(!target.exists());
        assert_eq!(fs::read(staged.recovery_temp()).unwrap(), b"captured");
        let next = store
            .begin_save(
                s.doc,
                TargetKey("released-after-cancel".into()),
                String::new(),
            )
            .unwrap();
        store.cancel_save(&next).unwrap();
    }
}

#[test]
fn ambiguous_windows_target_forms_are_rejected_without_creating_any_file() {
    for name in ["target.", "target ", "target:stream", "target?", "target*"] {
        let dir = fixture();
        let target = dir.join(name);
        let mut store = Store::new(HistoryBudget::default());
        let (s, p) = prepared(&mut store, b"payload", &target);
        let work = p.begin_staging(&mut store).unwrap();
        assert!(matches!(
            work.write_temp(&target, &ContentGuard::Absent),
            Err(StageFailure {
                issue: StageIssue::InvalidTarget,
                recovery_temp: None,
            })
        ));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        assert_eq!(store.snapshot(s.doc).unwrap().token, s.token);
        assert_eq!(store.commit_permit(p.capture()), Err(Error::InvalidSave));
        assert!(matches!(
            store.begin_save(s.doc, TargetKey("other".into()), String::new()),
            Err(Error::Busy)
        ));
        store.cancel_save(p.capture()).unwrap();
    }
}
