//! Generated-file returned-error fixtures. NOT_RUN: source review only.
//! These receipts do not cover a dropped worker, panic, or a discarded receipt.
use super::*;
use crate::files::windows_identity;
use std::sync::Mutex;

struct ReturnedFixture {
    dir: PathBuf,
    target: PathBuf,
    store: Store,
    capture: SaveCapture,
    parent: FileIdentity,
    work: BoundWork,
}

fn fixture(label: &str) -> ReturnedFixture {
    let dir = std::env::temp_dir().join(format!(
        "texzi-t07-returned-{label}-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    eprintln!(
        "T07 generated returned-error fixture retained: {}",
        dir.display()
    );
    let target = dir.join("original.txt");
    let source = dir.join("source.txt");
    fs::write(&target, b"A\r\n").unwrap();
    fs::write(&source, b"A\r\n").unwrap();
    let observed = ObservedTarget::inspect(&target).unwrap();
    let parent = observed.parent_identity();
    let mut store = Store::new(HistoryBudget::default());
    let (initial, binding) = match ObservedTarget::inspect(&source)
        .unwrap()
        .open_existing(None)
        .unwrap()
        .attach(&mut store)
        .unwrap()
    {
        windows_identity::OpenOutcome::Imported { snapshot, binding } => (snapshot, binding),
        _ => panic!("generated source should attach as a new document"),
    };
    store
        .submit(Transaction {
            doc: initial.doc,
            base: initial.revision,
            edits: vec![Edit::input(&initial, 0..0, "N")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap();
    let prepared = observed.prepare_save_as(&binding, &mut store).unwrap();
    let capture = prepared.capture().clone();
    let work = prepared.begin(&mut store).unwrap();
    ReturnedFixture {
        dir,
        target,
        store,
        capture,
        parent,
        work,
    }
}

fn assert_capture(actual: &SaveCapture, expected: &SaveCapture) {
    assert_eq!(actual.operation, expected.operation);
    assert_eq!(actual.binding_epoch, expected.binding_epoch);
    assert_eq!(actual.target, expected.target);
    assert_eq!(actual.guard, expected.guard);
    assert_eq!(actual.snapshot.doc, expected.snapshot.doc);
    assert_eq!(actual.snapshot.revision, expected.snapshot.revision);
    assert_eq!(actual.snapshot.token, expected.snapshot.token);
    assert!(Arc::ptr_eq(&actual.snapshot.body, &expected.snapshot.body));
}

fn assert_parent_pinned(recovery: &BoundRecovery, dir: &Path, parent: FileIdentity) {
    assert_eq!(recovery.parent_identity(), parent);
    recovery.recheck_parent().unwrap();
    let moved = dir.with_file_name(format!(
        "{}-must-not-move",
        dir.file_name().unwrap().to_string_lossy()
    ));
    assert_eq!(fs::rename(dir, moved).unwrap_err().raw_os_error(), Some(32));
}

fn deny_copy_after_successful_duplicate(work: &mut BoundWork) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let hook_calls = calls.clone();
    work.work.set_copy_handle_hook(move |file| {
        let limited = duplicate_read_control(file)?;
        hook_calls.fetch_add(1, Ordering::SeqCst);
        Ok(limited)
    });
    calls
}

#[test]
fn returned_native_copy_error_retains_original_create_handle_and_parent() {
    let ReturnedFixture {
        dir,
        target,
        mut store,
        capture,
        parent,
        mut work,
    } = fixture("early-copy");
    let created = Arc::new(Mutex::new(None));
    let receipt = created.clone();
    work.work.set_creation_hook(move |file, path| {
        *receipt.lock().unwrap() = Some((
            file.as_raw_handle() as usize,
            windows_identity::identity(file).unwrap(),
            path.to_owned(),
        ));
        Ok(())
    });
    // Invoke the real native copy through a duplicate with insufficient rights.
    let copy_calls = deny_copy_after_successful_duplicate(&mut work);
    let (before, _) = store
        .observe_and_subscribe(capture.snapshot.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    let failure = match work.write() {
        Ok(_) => panic!("native copy unexpectedly accepted a read-control-only handle"),
        Err(failure) => failure,
    };
    assert_eq!(copy_calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        &failure.issue,
        BoundIssue::Stage(StageIssue::Io(error)) if error.raw_os_error() == Some(5)
    ));
    let recovery = failure.recovery().unwrap().clone();
    drop(failure);
    let temporary = recovery.temporary().unwrap();
    let (handle, identity, temp_path) = created.lock().unwrap().take().unwrap();
    assert_eq!(temporary.original_handle_for_test(), handle);
    assert_eq!(temporary.current_version().unwrap().identity, identity);
    assert_eq!(temporary.temp_hint(), temp_path);
    assert_eq!(temporary.current_path().unwrap(), temp_path);
    assert_capture(recovery.capture(), &capture);
    assert_capture(temporary.capture(), &capture);
    assert!(recovery._historical_source.is_some());
    assert!(recovery._source_anchor.is_some());
    assert_eq!(temporary.expected_bytes(), b"NA\r\n");
    assert!(temporary.read_created_bytes().unwrap().is_empty());
    assert_separate_reader_denied(temporary.temp_hint());
    assert_parent_pinned(&recovery, &dir, parent);
    let (after, _) = store
        .observe_and_subscribe(capture.snapshot.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(after.state.global, before.state.global);
    assert_eq!(after.cursor, before.cursor);
    assert_eq!(
        store.captured_save_stage(&capture),
        Ok(SaveStage::WritingTemp)
    );
    assert_eq!(fs::read(target).unwrap(), b"A\r\n");
}

#[test]
fn returned_outer_changed_error_retains_the_completed_captured_temporary() {
    let ReturnedFixture {
        dir,
        target,
        mut store,
        capture,
        parent,
        mut work,
    } = fixture("outer-recheck");
    work.set_after_staging(|target| {
        fs::write(target, b"external complete bytes\r\n")?;
        Ok(())
    });
    let (before, _) = store
        .observe_and_subscribe(capture.snapshot.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    let failure = match work.write() {
        Ok(_) => panic!("production outer recheck accepted a changed target"),
        Err(failure) => failure,
    };
    assert!(matches!(
        &failure.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    let recovery = failure.recovery().unwrap().clone();
    drop(failure);
    let temporary = recovery.temporary().unwrap();
    assert_capture(recovery.capture(), &capture);
    assert_capture(temporary.capture(), &capture);
    assert_eq!(temporary.expected_bytes(), b"NA\r\n");
    assert_eq!(temporary.read_created_bytes().unwrap(), b"NA\r\n");
    assert_eq!(
        temporary.current_version().unwrap().bytes.as_ref(),
        b"NA\r\n"
    );
    assert_eq!(temporary.current_path().unwrap(), temporary.temp_hint());
    assert_eq!(temporary.target_hint(), recovery.target_path());
    assert_eq!(
        recovery.format(),
        capture.snapshot.body.profile.file_format.unwrap()
    );
    assert_separate_reader_denied(temporary.temp_hint());
    assert_parent_pinned(&recovery, &dir, parent);
    let (after, _) = store
        .observe_and_subscribe(capture.snapshot.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(after.state.global, before.state.global);
    assert_eq!(after.cursor, before.cursor);
    assert_eq!(
        store.captured_save_stage(&capture),
        Ok(SaveStage::WritingTemp)
    );
    assert_eq!(fs::read(target).unwrap(), b"external complete bytes\r\n");
}

#[test]
fn cancelled_old_worker_error_and_readonly_recovery_preserve_the_new_reservation() {
    let ReturnedFixture {
        dir,
        target,
        mut store,
        capture,
        parent,
        mut work,
    } = fixture("cancelled-old-worker");
    let copy_calls = deny_copy_after_successful_duplicate(&mut work);
    store.cancel_save(&capture).unwrap();
    let next = {
        let observed = ObservedTarget::inspect(&target).unwrap();
        store
            .begin_scoped_save(
                capture.snapshot.doc,
                observed.reservation_key(),
                "new reservation after cancellation".into(),
                observed.save_scope(),
            )
            .unwrap()
    };
    assert_ne!(next.operation, capture.operation);
    let (before, _) = store
        .observe_and_subscribe(next.snapshot.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    let failure = match work.write() {
        Ok(_) => panic!("old native copy unexpectedly succeeded"),
        Err(failure) => failure,
    };
    assert_eq!(copy_calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        &failure.issue,
        BoundIssue::Stage(StageIssue::Io(error)) if error.raw_os_error() == Some(5)
    ));
    let recovery = failure.recovery().unwrap().clone();
    drop(failure);
    assert_capture(recovery.capture(), &capture);
    let temporary = recovery.temporary().unwrap();
    assert_capture(temporary.capture(), &capture);
    assert!(temporary.read_created_bytes().unwrap().is_empty());
    assert_eq!(temporary.current_path().unwrap(), temporary.temp_hint());
    temporary.current_version().unwrap();
    assert_separate_reader_denied(temporary.temp_hint());
    assert_parent_pinned(&recovery, &dir, parent);
    assert_eq!(
        store.finish_save_failure(
            recovery.capture(),
            SaveFailure {
                outcome: SaveFailureOutcome::BeforeReplace,
                conflict: false,
                os_error: None,
                message: "stale returned-error receipt".into(),
                recovery_paths: vec![temporary.temp_hint().to_owned()],
            },
        ),
        Err(Error::InvalidSave)
    );
    assert_eq!(store.captured_save_stage(&capture), Err(Error::InvalidSave));
    assert_eq!(store.captured_save_stage(&next), Ok(SaveStage::Encoding));
    let (after, _) = store
        .observe_and_subscribe(next.snapshot.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(after.state.global, before.state.global);
    assert_eq!(after.cursor, before.cursor);
    assert_eq!(fs::read(target).unwrap(), b"A\r\n");
    store.cancel_save(&next).unwrap();
}

#[forbid(unsafe_code)]
mod edge_cases;
