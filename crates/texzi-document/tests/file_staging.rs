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
    store
        .submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![Edit::input(&s, 0..0, "later")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap();
    let staged = p
        .stage_temp(
            &mut store,
            &target,
            &ContentGuard::Exact(Arc::from(&b"old\r\n"[..])),
        )
        .unwrap();
    assert_eq!(staged.recovery_temp().parent(), target.parent());
    assert_eq!(fs::read(staged.recovery_temp()).unwrap(), b"new\r\n");
    assert_eq!(fs::read(&target).unwrap(), b"old\r\n");
    assert!(store.dirty(s.doc).unwrap());
    assert_eq!(store.commit_permit(p.capture()), Err(Error::InvalidSave));
    assert_eq!(
        store.accept_verified_save(p.capture()),
        Err(Error::InvalidSave)
    );
    assert!(
        p.stage_temp(
            &mut store,
            &target,
            &ContentGuard::Exact(Arc::from(&b"old\r\n"[..]))
        )
        .is_err()
    );
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
        p.stage_temp(&mut store, &target, &ContentGuard::Absent),
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
        .stage_temp(&mut store, &new_target, &ContentGuard::Absent)
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
