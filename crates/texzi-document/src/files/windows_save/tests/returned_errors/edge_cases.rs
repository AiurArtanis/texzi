//! Returned-error boundary fixtures. NOT_RUN: source review only.
//! Native fault expectations below require a future authorized Windows run.
use super::*;
use std::io::Read;

struct FrozenStore {
    observed: crate::ObserverSnapshot,
    history_depth: (usize, usize),
    dirty: bool,
}

fn edit_after_capture_and_freeze(store: &mut Store, capture: &SaveCapture) -> FrozenStore {
    let current = store
        .submit(Transaction {
            doc: capture.snapshot.doc,
            base: capture.snapshot.revision,
            edits: vec![Edit::input(&capture.snapshot, 0..0, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    assert_ne!(current.token, capture.snapshot.token);
    let (observed, _) = store
        .observe_and_subscribe(capture.snapshot.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    FrozenStore {
        observed,
        history_depth: store.history_depth(capture.snapshot.doc).unwrap(),
        dirty: store.dirty(capture.snapshot.doc).unwrap(),
    }
}

fn assert_frozen_store(store: &mut Store, capture: &SaveCapture, frozen: &FrozenStore) {
    let (after, _) = store
        .observe_and_subscribe(capture.snapshot.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    // Global equality includes the N+1 document, baseline, history, dirty state,
    // binding epoch, active save and terminal state; getters/Drop publish none.
    assert_eq!(after.state.global, frozen.observed.state.global);
    assert_eq!(after.cursor, frozen.observed.cursor);
    assert_eq!(
        store.history_depth(capture.snapshot.doc).unwrap(),
        frozen.history_depth
    );
    assert_eq!(store.dirty(capture.snapshot.doc).unwrap(), frozen.dirty);
    assert_eq!(
        store.captured_save_stage(capture),
        Ok(SaveStage::WritingTemp)
    );
}

fn directory_bytes(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

#[test]
fn returned_precreation_changed_keeps_bound_context_without_a_temporary() {
    let ReturnedFixture {
        dir,
        target,
        mut store,
        capture,
        parent,
        mut work,
    } = fixture("precreation-changed");
    let resolved_target = work.observed.resolved_path();
    let creation_calls = Arc::new(AtomicUsize::new(0));
    let hook_calls = creation_calls.clone();
    work.work.set_creation_hook(move |_, _| {
        hook_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    let frozen = edit_after_capture_and_freeze(&mut store, &capture);
    let mut expected_files = directory_bytes(&dir);
    let external = b"external before BoundWork recheck\r\n";
    fs::write(&target, external).unwrap();
    expected_files
        .iter_mut()
        .find(|(path, _)| path == &target)
        .unwrap()
        .1 = external.to_vec();
    let failure = match work.write() {
        Ok(_) => panic!("precreation recheck accepted the changed target"),
        Err(failure) => failure,
    };
    assert!(matches!(
        &failure.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    assert_eq!(creation_calls.load(Ordering::SeqCst), 0);
    assert!(failure.recovery_temp.is_none());
    assert_capture(failure.capture.as_ref().unwrap(), &capture);
    let recovery = failure.recovery().unwrap();
    assert!(recovery.temporary().is_none());
    assert_capture(recovery.capture(), &capture);
    assert_eq!(recovery.target_path(), resolved_target);
    assert_eq!(
        recovery.format(),
        capture.snapshot.body.profile.file_format.unwrap()
    );
    assert!(recovery._historical_source.is_some());
    assert!(recovery._source_anchor.is_some());
    assert_recovery_directory_with_open_children_rejects_rename(recovery, &dir, parent);
    assert_eq!(directory_bytes(&dir), expected_files);
    assert_frozen_store(&mut store, &capture, &frozen);
}

#[test]
fn returned_first_creation_read_denied_keeps_private_capture_and_empty_original() {
    let ReturnedFixture {
        dir,
        target,
        mut store,
        capture,
        parent,
        mut work,
    } = fixture("first-creation-read-denied");
    let created = Arc::new(Mutex::new(None));
    let receipt = created.clone();
    let duplicate_calls = Arc::new(AtomicUsize::new(0));
    let hook_duplicates = duplicate_calls.clone();
    let denied_reads = Arc::new(AtomicUsize::new(0));
    let hook_reads = denied_reads.clone();
    work.work.set_creation_hook(move |file, path| {
        // This first hook precedes the factory's initial identity/security
        // reads. Record only the raw handle and path, without querying metadata.
        *receipt.lock().unwrap() = Some((file.as_raw_handle() as usize, path.to_owned()));
        let mut limited = duplicate_read_control(file)?;
        hook_duplicates.fetch_add(1, Ordering::SeqCst);
        match limited.read(&mut [0u8; 1]) {
            Ok(_) => panic!("read-control-only duplicate unexpectedly allowed ReadFile"),
            Err(error) => {
                assert_eq!(error.raw_os_error(), Some(5));
                hook_reads.fetch_add(1, Ordering::SeqCst);
                Err(error)
            }
        }
    });
    let copy_calls = deny_copy_after_successful_duplicate(&mut work);
    let frozen = edit_after_capture_and_freeze(&mut store, &capture);
    let mut failure = match work.write() {
        Ok(_) => panic!("first creation hook unexpectedly read the limited duplicate"),
        Err(failure) => failure,
    };
    assert_eq!(duplicate_calls.load(Ordering::SeqCst), 1);
    assert_eq!(denied_reads.load(Ordering::SeqCst), 1);
    assert_eq!(copy_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        &failure.issue,
        BoundIssue::Stage(StageIssue::Io(error)) if error.raw_os_error() == Some(5)
    ));
    let (raw, temp_path) = created.lock().unwrap().take().unwrap();
    assert_eq!(failure.recovery_temp.as_deref(), Some(temp_path.as_path()));
    let mut displayed_capture = capture.clone();
    displayed_capture.operation += 1;
    displayed_capture.binding_epoch += 1;
    displayed_capture.target = crate::TargetKey("display-only target".into());
    displayed_capture.guard = "display-only guard".into();
    displayed_capture.snapshot = store.snapshot(capture.snapshot.doc).unwrap();
    failure.capture = Some(displayed_capture);
    failure.recovery_temp = Some(dir.join("display-only-never-created.txt"));

    let recovery = failure.recovery().unwrap();
    let temporary = recovery.temporary().unwrap();
    assert_capture(recovery.capture(), &capture);
    assert_capture(temporary.capture(), &capture);
    assert_eq!(temporary.original_handle_for_test(), raw);
    assert_eq!(temporary.temp_hint(), temp_path);
    assert_eq!(temporary.target_hint(), recovery.target_path());
    assert_eq!(temporary.expected_bytes(), b"NA\r\n");
    assert!(recovery._historical_source.is_some());
    assert!(recovery._source_anchor.is_some());
    // Each current observation is a separate Result on the original handle;
    // these successful observations do not replace the original read Err(5).
    let bytes = temporary.read_created_bytes();
    let path = temporary.current_path();
    let version = temporary.current_version();
    assert!(bytes.unwrap().is_empty());
    assert_eq!(path.unwrap(), temp_path);
    assert!(version.unwrap().bytes.is_empty());
    assert!(temporary.read_created_bytes().unwrap().is_empty());
    assert_capture(recovery.capture(), &capture);
    assert!(matches!(
        &failure.issue,
        BoundIssue::Stage(StageIssue::Io(error)) if error.raw_os_error() == Some(5)
    ));
    assert_ne!(
        failure.capture.as_ref().unwrap().operation,
        capture.operation
    );
    assert_ne!(
        failure.recovery_temp.as_deref(),
        Some(temporary.temp_hint())
    );
    assert_separate_reader_denied(temporary.temp_hint());
    assert_recovery_directory_with_open_children_rejects_rename(recovery, &dir, parent);
    assert_frozen_store(&mut store, &capture, &frozen);
    assert_eq!(fs::read(target).unwrap(), b"A\r\n");
}

#[test]
fn returned_outer_prefix_changed_reports_actual_prefix_separately_from_expected_n() {
    let ReturnedFixture {
        dir,
        target,
        mut store,
        capture,
        parent,
        mut work,
    } = fixture("outer-prefix-changed");
    let created = Arc::new(Mutex::new(None));
    let receipt = created.clone();
    let duplicate = Arc::new(Mutex::new(None));
    let held_duplicate = duplicate.clone();
    let duplicate_calls = Arc::new(AtomicUsize::new(0));
    let hook_duplicates = duplicate_calls.clone();
    work.work.set_creation_hook(move |file, path| {
        let writable_duplicate = file.try_clone()?;
        *receipt.lock().unwrap() = Some((file.as_raw_handle() as usize, path.to_owned()));
        *held_duplicate.lock().unwrap() = Some(writable_duplicate);
        hook_duplicates.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    let prefix = b"NA";
    let truncation_calls = Arc::new(AtomicUsize::new(0));
    let hook_truncations = truncation_calls.clone();
    let take_duplicate = duplicate.clone();
    work.set_after_staging(move |_| {
        // The private fixture duplicate already names the created object; this
        // is not an ordinary external writer reopening past FILE_SHARE_NONE.
        let writable_duplicate = take_duplicate.lock().unwrap().take().unwrap();
        writable_duplicate.set_len(prefix.len() as u64)?;
        writable_duplicate.sync_all()?;
        drop(writable_duplicate);
        hook_truncations.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    let frozen = edit_after_capture_and_freeze(&mut store, &capture);
    let failure = match work.write() {
        Ok(_) => panic!("outer production validation accepted the truncated payload"),
        Err(failure) => failure,
    };
    assert_eq!(duplicate_calls.load(Ordering::SeqCst), 1);
    assert_eq!(truncation_calls.load(Ordering::SeqCst), 1);
    assert!(duplicate.lock().unwrap().is_none());
    assert!(matches!(
        &failure.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    let recovery = failure.recovery().unwrap();
    let temporary = recovery.temporary().unwrap();
    let (raw, temp_path) = created.lock().unwrap().take().unwrap();
    assert_eq!(temporary.original_handle_for_test(), raw);
    assert_eq!(temporary.current_path().unwrap(), temp_path);
    assert_capture(recovery.capture(), &capture);
    assert_capture(temporary.capture(), &capture);
    assert_eq!(temporary.expected_bytes(), b"NA\r\n");
    assert_ne!(prefix.as_slice(), temporary.expected_bytes());
    assert_eq!(temporary.read_created_bytes().unwrap(), prefix);
    assert_eq!(temporary.current_version().unwrap().bytes.as_ref(), prefix);
    // Both byte getters rewind, including after a preceding read reached EOF.
    assert_eq!(temporary.read_created_bytes().unwrap(), prefix);
    assert_eq!(temporary.current_version().unwrap().bytes.as_ref(), prefix);
    assert!(matches!(
        &failure.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    assert!(recovery._historical_source.is_some());
    assert!(recovery._source_anchor.is_some());
    assert_separate_reader_denied(temporary.temp_hint());
    assert_recovery_directory_with_open_children_rejects_rename(recovery, &dir, parent);
    assert_frozen_store(&mut store, &capture, &frozen);
    assert_eq!(fs::read(target).unwrap(), b"A\r\n");
}

#[test]
fn returned_bound_recovery_clones_unlock_only_after_the_last_holder_drops() {
    let ReturnedFixture {
        dir,
        target,
        mut store,
        capture,
        parent,
        mut work,
    } = fixture("bound-recovery-clones");
    let created = Arc::new(Mutex::new(None));
    let receipt = created.clone();
    work.work.set_creation_hook(move |file, path| {
        // Retain no duplicate File that could mask loss of the original owner.
        *receipt.lock().unwrap() = Some((file.as_raw_handle() as usize, path.to_owned()));
        Ok(())
    });
    let external = b"external complete bytes for clone lifecycle\r\n";
    work.set_after_staging(move |target| {
        fs::write(target, external)?;
        Ok(())
    });
    let frozen = edit_after_capture_and_freeze(&mut store, &capture);
    let failure = match work.write() {
        Ok(_) => panic!("outer recheck accepted the changed target"),
        Err(failure) => failure,
    };
    assert!(matches!(
        &failure.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    let first_clone = failure.recovery().unwrap().clone();
    let last_clone = first_clone.clone();
    let (raw, temp_path) = created.lock().unwrap().take().unwrap();
    assert_eq!(
        last_clone.temporary().unwrap().original_handle_for_test(),
        raw
    );
    assert_separate_reader_denied(&temp_path);
    drop(failure);
    assert_separate_reader_denied(&temp_path);
    assert_frozen_store(&mut store, &capture, &frozen);
    drop(first_clone);
    assert_separate_reader_denied(&temp_path);
    assert_capture(last_clone.capture(), &capture);
    {
        let temporary = last_clone.temporary().unwrap();
        assert_capture(temporary.capture(), &capture);
        assert_eq!(temporary.expected_bytes(), b"NA\r\n");
        assert_eq!(temporary.read_created_bytes().unwrap(), b"NA\r\n");
        assert_eq!(
            temporary.current_version().unwrap().bytes.as_ref(),
            b"NA\r\n"
        );
        assert_eq!(temporary.current_path().unwrap(), temp_path);
    }
    assert_recovery_directory_with_open_children_rejects_rename(&last_clone, &dir, parent);
    assert_frozen_store(&mut store, &capture, &frozen);
    drop(last_clone);
    // Closing the final Arc releases the share denial, but does not remove the
    // candidate, restore the target, finish/cancel the operation or alter N+1.
    assert_eq!(fs::read(&temp_path).unwrap(), b"NA\r\n");
    assert!(temp_path.is_file());
    assert_eq!(fs::read(target).unwrap(), external);
    assert_frozen_store(&mut store, &capture, &frozen);
}
