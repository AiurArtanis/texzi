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
fn creation_default_native_save_as_preserves_the_directory_creation_descriptor() {
    let dir = fixture();
    let original = dir.join("original.txt");
    let destination = dir.join("saved.txt");
    fs::write(&original, b"A").unwrap();
    let mut store = Store::new(HistoryBudget::default());
    let OpenOutcome::Imported { snapshot, binding } = ObservedTarget::inspect(&original)
        .unwrap()
        .open_existing(None)
        .unwrap()
        .attach(&mut store)
        .unwrap()
    else {
        unreachable!();
    };
    let n = input(&mut store, &snapshot, 1, "B");
    let staged = ObservedTarget::inspect(&destination)
        .unwrap()
        .prepare_save_as(&binding, &mut store)
        .unwrap()
        .begin(&mut store)
        .unwrap()
        .write()
        .unwrap();
    let expected = staged.creation_security().clone();
    let private = staged.temporary_security().clone();
    let temporary = staged.temporary_identity();
    let published = staged
        .lock_temporary()
        .unwrap()
        .prepare_new_creation_default()
        .unwrap()
        .permit(&mut store)
        .unwrap()
        .run()
        .unwrap_or_else(|failure| {
            if let Ok(actual) = failure.current_version() {
                let before = expected.as_bytes();
                let after = actual.security.as_bytes();
                let different: Vec<_> = before
                    .iter()
                    .zip(after)
                    .enumerate()
                    .filter_map(|(index, (left, right))| (left != right).then_some(index))
                    .collect();
                eprintln!(
                    "CreationDefault diagnostic: expected_len={}, actual_len={}, expected_control={:02X?}, actual_control={:02X?}, differing_offsets={different:?}, private_match={}, identity_match={}, bytes_match={}, at_destination={}",
                    before.len(),
                    after.len(),
                    &before[2..4],
                    &after[2..4],
                    actual.security == private,
                    actual.identity == temporary,
                    actual.bytes.as_ref() == b"AB",
                    failure.current_path().is_ok_and(|path| path.file_name() == destination.file_name()),
                );
            }
            panic!("{failure:?}");
        });
    let committed = published.accept(&mut store).unwrap();
    assert_eq!(committed.saved_snapshot.token, n.token);
    assert_eq!(committed.version.identity, temporary);
    assert_eq!(committed.version.security, expected);
    assert_eq!(fs::read(&original).unwrap(), b"A");
    assert_eq!(fs::read(&destination).unwrap(), b"AB");
    assert!(!store.dirty(n.doc).unwrap());
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
    // Replacing an already-open destination can report ACCESS_DENIED rather
    // than the SHARING_VIOLATION required by the source move/open checks above.
    let error = fs::rename(&substitute, &temp).unwrap_err();
    assert!(
        matches!(error.raw_os_error(), Some(5 | 32)),
        "replace open creation temporary: {error}"
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
fn exclusive_target_handle_denies_replacement_until_it_is_closed() {
    use std::{
        io::{Read, Seek},
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    };
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_ID_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo,
        GetFileInformationByHandleEx,
    };

    fn handle_identity(file: &fs::File) -> FileIdentity {
        let mut info = FILE_ID_INFO::default();
        // SAFETY: this live fixture File owns the handle; info is writable for
        // exactly the documented FILE_ID_INFO buffer size.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&mut info as *mut FILE_ID_INFO).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        };
        assert_ne!(
            ok,
            0,
            "fixture identity: {}",
            std::io::Error::last_os_error()
        );
        FileIdentity {
            volume: info.VolumeSerialNumber,
            id: info.FileId.Identifier,
        }
    }
    fn path_identity(path: &std::path::Path) -> FileIdentity {
        ObservedTarget::inspect(path)
            .unwrap()
            .version()
            .unwrap()
            .identity
    }
    fn handle_bytes(file: &mut fs::File) -> Vec<u8> {
        file.rewind().unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        bytes
    }

    let dir = fixture();
    let source = dir.join("rename-control-source.txt");
    let target = dir.join("rename-control-target.txt");
    fs::write(&source, b"unlocked replacement\r\n").unwrap();
    fs::write(&target, b"original destination\r\n").unwrap();
    let first_source_id = path_identity(&source);
    assert_ne!(path_identity(&target), first_source_id);
    // With no handles open, replacing this destination is permitted by the
    // inherited permissions; retain the resulting file as the locked target.
    fs::rename(&source, &target).unwrap();
    assert!(!source.try_exists().unwrap());
    assert_eq!(path_identity(&target), first_source_id);
    assert_eq!(fs::read(&target).unwrap(), b"unlocked replacement\r\n");

    fs::write(&source, b"replacement after holder closes\r\n").unwrap();
    let source_id = path_identity(&source);
    assert_ne!(source_id, first_source_id);
    let mut holder = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&target)
        .unwrap();
    assert_eq!(handle_identity(&holder), first_source_id);
    assert_eq!(handle_bytes(&mut holder), b"unlocked replacement\r\n");
    // The explicit DELETE open is a separate operation and must retain the
    // precise sharing violation even if MoveFileEx replacement reports 5.
    assert_eq!(
        fs::OpenOptions::new()
            .access_mode(DELETE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&target)
            .unwrap_err()
            .raw_os_error(),
        Some(32)
    );
    let error = fs::rename(&source, &target).unwrap_err();
    eprintln!(
        "T07 open-target replacement control raw_os_error: {:?}",
        error.raw_os_error()
    );
    assert!(
        matches!(error.raw_os_error(), Some(5 | 32)),
        "replace exclusive target: {error}"
    );
    assert!(source.try_exists().unwrap());
    assert!(target.try_exists().unwrap());
    assert_eq!(path_identity(&source), source_id);
    assert_eq!(
        fs::read(&source).unwrap(),
        b"replacement after holder closes\r\n"
    );
    assert_eq!(handle_identity(&holder), first_source_id);
    assert_eq!(handle_bytes(&mut holder), b"unlocked replacement\r\n");

    drop(holder);
    fs::rename(&source, &target).unwrap();
    assert!(!source.try_exists().unwrap());
    assert_eq!(path_identity(&target), source_id);
    assert_eq!(
        fs::read(&target).unwrap(),
        b"replacement after holder closes\r\n"
    );
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
