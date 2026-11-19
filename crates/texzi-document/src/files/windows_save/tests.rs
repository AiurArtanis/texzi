//! Generated-file Windows unit fixtures. Never run against user documents.
use super::*;
mod creation_security;
mod returned_errors;
use crate::{
    BoundaryPolicy, Edit, HistoryBudget, ObserverScope, SaveFailure, SaveFailureOutcome,
    Transaction,
};
use std::{
    fs::{self, File, OpenOptions},
    os::windows::{
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
    },
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(1);
use windows_sys::Win32::{
    Foundation::DuplicateHandle,
    Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL},
    System::Threading::GetCurrentProcess,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SecurityCase {
    Mismatch,
    Match,
    PermissionDenied,
}
fn assert_separate_reader_denied(path: &Path) {
    match OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
    {
        Ok(_) => panic!("separate reader acquired the temporary file"),
        Err(e) => assert_eq!(e.raw_os_error(), Some(32)),
    }
}
fn duplicate_read_control(file: &File) -> std::io::Result<File> {
    let mut duplicate = std::ptr::null_mut();
    // SAFETY: duplicate this fixture's live handle into the current process with
    // fewer rights, no pathname reopen. File closes the new handle exactly once.
    let process = unsafe { GetCurrentProcess() };
    if unsafe {
        DuplicateHandle(
            process,
            file.as_raw_handle(),
            process,
            &mut duplicate,
            READ_CONTROL,
            0,
            0,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_handle(duplicate) })
}

fn security_boundary(case: SecurityCase) {
    let mismatch = case == SecurityCase::Mismatch;
    let denied = case == SecurityCase::PermissionDenied;
    let dir = std::env::temp_dir().join(format!(
        "texzi-t07-security-boundary-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    eprintln!(
        "T07 generated security boundary fixture retained: {}",
        dir.display()
    );
    let target = dir.join("original.txt");
    let reader_control = dir.join("reader-control.txt");
    fs::write(&reader_control, b"readable inherited ACL control").unwrap();
    assert_eq!(
        fs::read(&reader_control).unwrap(),
        b"readable inherited ACL control"
    );
    fs::write(&target, b"A\r\n").unwrap();
    let observation = ObservedTarget::inspect(&target).unwrap();
    let original = observation.version().unwrap().clone();
    let expected = original.security.clone();
    let changed = expected.with_flipped_dacl_protection().unwrap();
    assert_ne!(changed, expected);
    let mut store = Store::new(HistoryBudget::default());
    let (s, binding) = super::super::LoadedBytes::decode(
        Arc::from(&b"A\r\n"[..]),
        Some(super::super::Encoding::Utf8),
    )
    .unwrap()
    .attach(&mut store)
    .unwrap();
    let n = store
        .submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![Edit::input(&s, 0..0, "N")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    let prepared = observation.prepare_bound(&binding, &mut store).unwrap();
    let capture = prepared.capture().clone();
    let mut worker = prepared.begin(&mut store).unwrap();
    let created_path = Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let creation_calls = Arc::new(AtomicUsize::new(0));
    let probe_path = created_path.clone();
    let probe_calls = creation_calls.clone();
    worker.work.set_creation_hook(move |new_file, path| {
        assert_eq!(new_file.metadata()?.len(), 0);
        // Before SetSecurityInfo, even a reader sharing all access is denied.
        assert_separate_reader_denied(path);
        *probe_path.lock().unwrap() = Some(path.to_owned());
        probe_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    if denied {
        worker.work.set_copy_handle_hook(duplicate_read_control);
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let hook_calls = calls.clone();
    let required = expected.clone();
    let applied = if mismatch { changed } else { expected };
    let after_copy_path = created_path.clone();
    worker.work.set_security_hook(move |new_file| {
        assert_eq!(new_file.metadata()?.len(), 0);
        assert_separate_reader_denied(after_copy_path.lock().unwrap().as_ref().unwrap());
        // Real API on this operation's new empty temp. Product copying already
        // ran; alter only its protection bit, then use the real validator below.
        applied.apply_to_new_file(new_file)?;
        let actual = SecurityDescriptor::read(new_file)?;
        if mismatch {
            assert_ne!(actual, required);
        } else {
            assert_eq!(actual, required);
        }
        hook_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    let current = store
        .submit(Transaction {
            doc: s.doc,
            base: n.revision,
            edits: vec![Edit::input(&n, 0..0, "M")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    let depth = store.history_depth(s.doc).unwrap();
    let (before, _) = store
        .observe_and_subscribe(s.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    let recovery;
    let retained_recovery;
    if mismatch || denied {
        let failure = match worker.write() {
            Ok(_) => panic!("mismatched security accepted"),
            Err(e) => e,
        };
        if denied {
            assert!(
                matches!(&failure.issue, BoundIssue::Stage(StageIssue::Io(e)) if e.raw_os_error() == Some(5))
            );
        } else {
            assert!(matches!(
                &failure.issue,
                BoundIssue::Stage(StageIssue::SecurityMismatch)
            ));
        }
        let returned = failure.capture.as_ref().unwrap().clone();
        assert_eq!(returned.operation, capture.operation);
        assert_eq!(returned.binding_epoch, capture.binding_epoch);
        assert_eq!(returned.snapshot.token, capture.snapshot.token);
        assert!(Arc::ptr_eq(&returned.snapshot.body, &capture.snapshot.body));
        recovery = failure.recovery_temp.as_ref().unwrap().clone();
        let retained = failure.recovery().unwrap().clone();
        let temporary = retained.temporary().unwrap();
        assert_eq!(temporary.capture().operation, returned.operation);
        assert!(temporary.read_created_bytes().unwrap().is_empty());
        assert_separate_reader_denied(&recovery);
        retained.recheck_parent().unwrap();
        retained_recovery = Some(retained);
        store
            .finish_save_failure(
                &returned,
                SaveFailure {
                    outcome: SaveFailureOutcome::BeforeReplace,
                    conflict: false,
                    os_error: None,
                    message: "generated security mismatch".into(),
                    recovery_paths: vec![recovery.clone()],
                },
            )
            .unwrap();
        assert_eq!(
            store
                .recovery_snapshot(s.doc, returned.operation)
                .unwrap()
                .token,
            n.token
        );
    } else {
        retained_recovery = None;
        let staged = worker.write().unwrap();
        assert_eq!(staged.payload(), b"NA\r\n");
        assert_eq!(staged.capture().snapshot.token, n.token);
        assert_separate_reader_denied(staged.recovery_temp());
        staged.check_owner(&store).unwrap();
        recovery = staged.recovery_temp().to_owned();
        assert_eq!(staged.read_created_bytes().unwrap(), b"NA\r\n");
        let locked = staged.lock_temporary().unwrap();
        assert_eq!(
            locked.original_security().unwrap(),
            locked.temporary_security()
        );
        store.cancel_save(locked.capture()).unwrap();
    }
    assert_eq!(creation_calls.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), if denied { 0 } else { 1 });
    let (after, _) = store
        .observe_and_subscribe(s.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(
        before.state.global.metadata.baseline,
        after.state.global.metadata.baseline
    );
    assert_eq!(after.state.global.document.token, current.token);
    assert_eq!(after.state.global.document.body.to_utf8(), b"MNA\r\n");
    assert!(after.state.global.metadata.save.active.is_none());
    let terminal = after
        .state
        .global
        .metadata
        .save
        .last_terminal
        .as_ref()
        .unwrap();
    assert_eq!(terminal.operation, capture.operation);
    assert_eq!(
        terminal.result,
        if mismatch || denied {
            crate::SaveResult::Failed
        } else {
            crate::SaveResult::Cancelled
        }
    );
    assert_eq!(store.history_depth(s.doc).unwrap(), depth);
    assert!(store.dirty(s.doc).unwrap());
    assert_eq!(
        ObservedTarget::inspect(&target).unwrap().version().unwrap(),
        &original
    );
    assert_eq!(fs::read(&target).unwrap(), b"A\r\n");
    assert!(recovery.exists());
    if let Some(retained) = retained_recovery {
        assert_separate_reader_denied(&recovery);
        assert!(
            retained
                .temporary()
                .unwrap()
                .read_created_bytes()
                .unwrap()
                .is_empty()
        );
        retained.recheck_parent().unwrap();
    } else {
        assert_eq!(fs::read(&recovery).unwrap(), b"NA\r\n");
    }
}

#[test]
fn mismatched_security_rejects_before_payload_and_finishes_original_capture() {
    security_boundary(SecurityCase::Mismatch);
}
#[test]
fn matching_security_keeps_captured_payload_and_newer_edit_independent() {
    security_boundary(SecurityCase::Match);
}
#[test]
fn insufficient_copy_handle_rights_leave_zero_payload_and_original_baseline() {
    security_boundary(SecurityCase::PermissionDenied);
}

fn new_ready_fixture() -> (PathBuf, PathBuf, Store, crate::Snapshot, NewReady) {
    let dir = std::env::temp_dir().join(format!(
        "texzi-new-publish-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    eprintln!(
        "T07 generated new-publish fixture retained: {}",
        dir.display()
    );
    let target = dir.join("saved.txt");
    let mut store = Store::new(HistoryBudget::default());
    let (s, binding) = super::super::LoadedBytes::decode(
        Arc::from(&b"A\r\n"[..]),
        Some(super::super::Encoding::Utf8),
    )
    .unwrap()
    .attach(&mut store)
    .unwrap();
    let mut profile = s.body.profile;
    profile.file_format = Some(Format {
        encoding: super::super::Encoding::Utf16Le,
        bom: true,
    });
    let n = store
        .set_serialization(s.doc, s.revision, profile, None)
        .unwrap()
        .snapshot;
    let ready = ObservedTarget::inspect(&target)
        .unwrap()
        .prepare_bound(&binding, &mut store)
        .unwrap()
        .begin(&mut store)
        .unwrap()
        .write()
        .unwrap()
        .lock_temporary()
        .unwrap()
        .prepare_new_owner_only()
        .unwrap();
    (dir, target, store, n, ready)
}

#[test]
fn new_handle_publication_verifies_final_identity_and_keeps_n_plus_one_dirty() {
    let (dir, target, mut store, n, ready) = new_ready_fixture();
    let capture = ready.capture().clone();
    let mut job = ready.permit(&mut store).unwrap();
    assert_eq!(store.cancel_save(&capture), Err(Error::TooLate));
    let (waiting_tx, waiting_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    job.set_before_rename(move |_| {
        waiting_tx
            .send(())
            .map_err(|_| std::io::Error::other("fixture owner ended"))?;
        release_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|_| std::io::Error::other("fixture owner did not release publication"))?;
        Ok(())
    });
    let worker = std::thread::spawn(move || job.run());
    waiting_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    let mut profile = n.body.profile;
    profile.file_format = Some(Format {
        encoding: super::super::Encoding::Utf16Be,
        bom: false,
    });
    let next_format = store
        .set_serialization(n.doc, n.revision, profile, None)
        .unwrap()
        .snapshot;
    let current = store
        .submit(Transaction {
            doc: n.doc,
            base: next_format.revision,
            edits: vec![Edit::input(&next_format, 0..0, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    let depth = store.history_depth(n.doc).unwrap();
    release_tx.send(()).unwrap();
    let published = worker.join().unwrap().unwrap();
    let actual_id = published.identity();
    assert_separate_reader_denied(&target); // Same creation handle is still exclusive.
    let committed = published.accept(&mut store).unwrap();
    let disk = committed.binding.disk.as_ref().unwrap();
    assert_eq!(disk.version, committed.version);
    assert_eq!(disk.path, committed.path);
    assert_eq!(disk.epoch, committed.binding_epoch);
    assert!(disk.anchor.retains(committed.version.identity).unwrap());
    assert_eq!(store.publication_recovery.len(), 0);
    let raw = [0xff, 0xfe, 65, 0, 13, 0, 10, 0];
    assert_eq!(fs::read(&target).unwrap(), raw);
    assert_eq!(committed.version.identity, actual_id);
    assert_eq!(
        ObservedTarget::inspect(&target)
            .unwrap()
            .version()
            .unwrap()
            .identity,
        actual_id
    );
    assert_eq!(committed.binding_epoch, 1);
    assert_eq!(committed.saved_snapshot.token, n.token);
    assert_eq!(
        committed
            .binding
            .encode_snapshot(&n, n.body.profile.file_format.unwrap())
            .unwrap()
            .as_ref(),
        raw
    );
    let (observed, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(
        observed.state.global.metadata.attention.target,
        Some(actual_id.target_key())
    );
    assert_ne!(
        observed.state.global.metadata.attention.target,
        Some(capture.target)
    );
    assert_eq!(observed.state.global.metadata.baseline.token, n.token);
    assert_eq!(observed.state.global.document.token, current.token);
    assert!(store.dirty(n.doc).unwrap());
    assert_eq!(store.history_depth(n.doc).unwrap(), depth);
    assert_eq!(store.undo(n.doc).unwrap().snapshot.token, next_format.token);
    assert!(store.dirty(n.doc).unwrap());
    assert_eq!(store.undo(n.doc).unwrap().snapshot.token, n.token);
    assert!(!store.dirty(n.doc).unwrap());
    store.redo(n.doc).unwrap();
    assert_eq!(store.redo(n.doc).unwrap().snapshot.token, current.token);
    let (other, other_binding) = super::super::LoadedBytes::decode(
        Arc::from(&b"other"[..]),
        Some(super::super::Encoding::Utf8),
    )
    .unwrap()
    .attach(&mut store)
    .unwrap();
    assert!(matches!(
        store.begin_save(other.doc, actual_id.target_key(), String::new()),
        Err(Error::Busy)
    ));
    // Success released both the parent scope and old new-in-directory key.
    let next = ObservedTarget::inspect(&dir.join("other.txt"))
        .unwrap()
        .prepare_bound(&other_binding, &mut store)
        .unwrap();
    store.cancel_save(next.capture()).unwrap();
}

#[test]
fn target_created_after_check_is_not_overwritten_and_unknown_result_is_retained() {
    let (_, target, mut store, n, ready) = new_ready_fixture();
    let capture = ready.capture().clone();
    let mut job = ready.permit(&mut store).unwrap();
    let foreign_version = Arc::new(std::sync::Mutex::new(None::<FileVersion>));
    let receipt = foreign_version.clone();
    job.set_before_rename(move |path| {
        use std::io::Write;
        let mut foreign = OpenOptions::new().write(true).create_new(true).open(path)?;
        foreign.write_all(b"external complete version\r\n")?;
        foreign.sync_all()?;
        drop(foreign);
        *receipt.lock().unwrap() = Some(
            ObservedTarget::inspect(path)
                .unwrap()
                .version()
                .unwrap()
                .clone(),
        );
        Ok(())
    });
    let current = store
        .submit(Transaction {
            doc: n.doc,
            base: n.revision,
            edits: vec![Edit::input(&n, 0..0, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    let failure = match job.run() {
        Ok(_) => panic!("overwrote raced target"),
        Err(e) => e,
    };
    assert!(
        matches!(&failure.issue, PublishIssue::Io(e) if matches!(e.raw_os_error(), Some(80 | 183)))
    );
    assert_eq!(
        ObservedTarget::inspect(&target).unwrap().version().unwrap(),
        foreign_version.lock().unwrap().as_ref().unwrap()
    );
    assert_eq!(failure.capture().operation, capture.operation);
    assert_eq!(
        failure.read_retained_bytes().unwrap(),
        [0xff, 0xfe, 65, 0, 13, 0, 10, 0]
    );
    failure.record_failure(&mut store).unwrap();
    let (observed, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(
        observed
            .state
            .global
            .metadata
            .save
            .last_terminal
            .as_ref()
            .unwrap()
            .result,
        crate::SaveResult::OutcomeUnknown
    );
    assert_eq!(observed.state.global.document.token, current.token);
    assert!(store.dirty(n.doc).unwrap());
    assert_eq!(
        store
            .recovery_snapshot(n.doc, capture.operation)
            .unwrap()
            .token,
        n.token
    );
    assert_eq!(failure.record_failure(&mut store), Err(Error::InvalidSave));
    assert_eq!(fs::read(&target).unwrap(), b"external complete version\r\n");
}

#[test]
fn cancelled_new_ready_cannot_permit_or_release_a_new_reservation() {
    let (_, target, mut store, n, ready) = new_ready_fixture();
    let old = ready.capture().clone();
    store.cancel_save(&old).unwrap();
    let observed = ObservedTarget::inspect(&target).unwrap();
    let new = store
        .begin_scoped_save(
            n.doc,
            observed.reservation_key(),
            String::new(),
            observed.save_scope(),
        )
        .unwrap();
    let failure = match ready.permit(&mut store) {
        Ok(_) => panic!("cancelled permit accepted"),
        Err(e) => e,
    };
    assert!(matches!(
        &failure.issue,
        PublishIssue::Owner(Error::InvalidSave)
    ));
    assert_eq!(failure.record_failure(&mut store), Err(Error::InvalidSave));
    assert!(!target.exists());
    store.cancel_save(&new).unwrap();
}

#[test]
fn existing_target_has_no_owner_only_new_publication_path() {
    let dir = std::env::temp_dir().join(format!(
        "texzi-existing-exclusion-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    eprintln!(
        "T07 generated existing-target exclusion fixture retained: {}",
        dir.display()
    );
    let target = dir.join("existing.txt");
    fs::write(&target, b"existing bytes\r\n").unwrap();
    let observed = ObservedTarget::inspect(&target).unwrap();
    let original = observed.version().unwrap().clone();
    let mut store = Store::new(HistoryBudget::default());
    let (s, binding) = super::super::LoadedBytes::decode(
        Arc::from(&b"memory"[..]),
        Some(super::super::Encoding::Utf8),
    )
    .unwrap()
    .attach(&mut store)
    .unwrap();
    let locked = observed
        .prepare_bound(&binding, &mut store)
        .unwrap()
        .begin(&mut store)
        .unwrap()
        .write()
        .unwrap()
        .lock_temporary()
        .unwrap();
    let failure = match locked.prepare_new_owner_only() {
        Ok(_) => panic!("existing target accepted"),
        Err(e) => e,
    };
    assert!(matches!(
        &failure.issue,
        PublishIssue::ExistingTargetUnsupported
    ));
    failure.record_failure(&mut store).unwrap();
    assert_eq!(store.snapshot(s.doc).unwrap().token, s.token);
    assert_eq!(
        ObservedTarget::inspect(&target).unwrap().version().unwrap(),
        &original
    );
}

fn renamed_then_failed(flush_failure: bool) {
    let (dir, target, mut store, n, ready) = new_ready_fixture();
    let initial_paths: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(initial_paths.len(), 1);
    let original_temp = initial_paths[0].clone();
    assert_ne!(original_temp, target);
    let capture = ready.capture().clone();
    let mut job = ready.permit(&mut store).unwrap();
    let flush_calls = Arc::new(AtomicUsize::new(0));
    if flush_failure {
        let calls = flush_calls.clone();
        job.set_post_rename_flush_handle(move |file| {
            // This callback is reachable only after native rename returned success.
            let current_path = super::super::windows_identity::volume_path(file)
                .map_err(|e| std::io::Error::other(format!("fixture path query: {e:?}")))?;
            assert_eq!(
                current_path.file_name(),
                Some(std::ffi::OsStr::new("saved.txt"))
            );
            let duplicate = duplicate_read_control(file)?;
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(duplicate)
        });
    } else {
        // Fault only the verifier's expected format, after real rename/flush/read.
        // Physical N bytes stay intact; no fake IO result or alternate exit path.
        job.set_verification_format(Format {
            encoding: super::super::Encoding::Utf8,
            bom: false,
        });
    }
    let current = store
        .submit(Transaction {
            doc: n.doc,
            base: n.revision,
            edits: vec![Edit::input(&n, 0..0, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    let depth = store.history_depth(n.doc).unwrap();
    let (before, subscription) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    let failure = match job.run() {
        Ok(_) => panic!("post-rename fault was accepted"),
        Err(e) => e,
    };
    if flush_failure {
        assert_eq!(flush_calls.load(Ordering::SeqCst), 1);
        assert!(matches!(&failure.issue, PublishIssue::Io(e) if e.raw_os_error() == Some(5)));
    } else {
        assert!(matches!(&failure.issue, PublishIssue::VerificationMismatch));
    }
    let actual_path = failure.current_path().unwrap();
    assert_eq!(actual_path.file_name(), target.file_name());
    assert!(!original_temp.exists());
    assert_separate_reader_denied(&target);
    let raw = [0xff, 0xfe, 65, 0, 13, 0, 10, 0];
    assert_eq!(failure.read_retained_bytes().unwrap(), raw);
    assert_eq!(failure.current_version().unwrap().bytes.as_ref(), raw);
    assert_eq!(failure.capture().operation, capture.operation);
    assert_eq!(failure.capture().snapshot.token, n.token);
    failure.record_failure(&mut store).unwrap();
    assert!(matches!(
        store.poll_observer(&subscription).unwrap(),
        crate::ObserverPoll::Batch(_)
    ));
    assert!(matches!(
        store.poll_observer(&subscription).unwrap(),
        crate::ObserverPoll::Pending
    ));
    let (after, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(
        after.state.global.metadata.baseline,
        before.state.global.metadata.baseline
    );
    assert_eq!(
        after.state.global.metadata.attention.target,
        before.state.global.metadata.attention.target
    );
    assert_eq!(
        after.state.global.metadata.attention.binding_epoch,
        before.state.global.metadata.attention.binding_epoch
    );
    assert_eq!(after.state.global.document.token, current.token);
    assert_eq!(
        after.state.global.document.body.to_utf8(),
        current.body.to_utf8()
    );
    assert_eq!(store.history_depth(n.doc).unwrap(), depth);
    assert!(store.dirty(n.doc).unwrap());
    assert!(after.state.global.metadata.save.active.is_none());
    let terminal = after
        .state
        .global
        .metadata
        .save
        .last_terminal
        .as_ref()
        .unwrap();
    assert_eq!(terminal.operation, capture.operation);
    assert_eq!(terminal.result, crate::SaveResult::OutcomeUnknown);
    let record = after
        .state
        .global
        .metadata
        .attention
        .recoveries
        .last()
        .unwrap();
    assert!(record.detail.recovery_paths.contains(&actual_path));
    assert_eq!(
        store
            .recovery_snapshot(n.doc, capture.operation)
            .unwrap()
            .token,
        n.token
    );
    // A distinct existing file key proves the retained parent scope is held,
    // rather than only testing the old new-in-directory reservation key.
    let other_path = dir.join("other-existing.txt");
    fs::write(&other_path, b"other").unwrap();
    let (_, binding) = super::super::LoadedBytes::decode(
        Arc::from(&b"other"[..]),
        Some(super::super::Encoding::Utf8),
    )
    .unwrap()
    .attach(&mut store)
    .unwrap();
    let other = ObservedTarget::inspect(&other_path).unwrap();
    assert_ne!(other.reservation_key(), capture.target);
    assert!(matches!(
        other.prepare_bound(&binding, &mut store),
        Err(BoundFailure {
            issue: BoundIssue::File(FileError::Document(Error::UnresolvedConflict)),
            ..
        })
    ));
    assert_eq!(failure.record_failure(&mut store), Err(Error::InvalidSave));
    assert_eq!(failure.read_retained_bytes().unwrap(), raw);
    drop(failure); // Registry retains the original handle until owner acknowledgement.
    assert_separate_reader_denied(&target);
    let notices = store.drain_dropped_publications();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].terminal_recorded);
    assert!(notices[0].owner_error.is_none());
    store
        .acknowledge_publication_recovery(capture.operation)
        .unwrap();
    drop(notices);
    assert_eq!(fs::read(&target).unwrap(), raw);
    assert!(!original_temp.exists());
    assert!(matches!(
        ObservedTarget::inspect(&other_path)
            .unwrap()
            .prepare_bound(&binding, &mut store),
        Err(BoundFailure {
            issue: BoundIssue::File(FileError::Document(Error::UnresolvedConflict)),
            ..
        })
    ));
}

#[test]
fn post_rename_flush_failure_keeps_resources_baseline_and_held_scope() {
    renamed_then_failed(true);
}
#[test]
fn post_rename_verification_failure_keeps_resources_baseline_and_held_scope() {
    renamed_then_failed(false);
}

fn dropped_publication(after_rename: bool) {
    let (_dir, target, mut store, n, ready) = new_ready_fixture();
    let capture = ready.capture().clone();
    let job = ready.permit(&mut store).unwrap();
    let current = store
        .submit(Transaction {
            doc: n.doc,
            base: n.revision,
            edits: vec![Edit::input(&n, 0..0, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    let depth = store.history_depth(n.doc).unwrap();
    let (before, subscription) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    if after_rename {
        drop(job.run().unwrap());
        assert!(target.exists());
        assert_separate_reader_denied(&target);
    } else {
        drop(job);
        assert!(!target.exists());
    }
    assert_eq!(
        store.acknowledge_publication_recovery(capture.operation),
        Err(Error::InvalidSave)
    );
    let notices = store.drain_dropped_publications();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].terminal_recorded);
    assert!(notices[0].owner_error.is_none());
    assert_eq!(notices[0].capture().snapshot.token, n.token);
    assert_eq!(
        notices[0].read_retained_bytes().unwrap(),
        [255, 254, 65, 0, 13, 0, 10, 0]
    );
    let path = notices[0].current_path().unwrap();
    assert_eq!(path.file_name() == target.file_name(), after_rename);
    assert_separate_reader_denied(&path);
    let (after, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(
        after.state.global.metadata.baseline,
        before.state.global.metadata.baseline
    );
    assert_eq!(
        after.state.global.metadata.attention.target,
        before.state.global.metadata.attention.target
    );
    assert_eq!(
        after.state.global.metadata.attention.binding_epoch,
        before.state.global.metadata.attention.binding_epoch
    );
    assert_eq!(store.snapshot(n.doc).unwrap().token, current.token);
    assert_eq!(store.history_depth(n.doc).unwrap(), depth);
    assert!(store.dirty(n.doc).unwrap());
    assert_eq!(
        after
            .state
            .global
            .metadata
            .save
            .last_terminal
            .as_ref()
            .unwrap()
            .result,
        crate::SaveResult::OutcomeUnknown
    );
    assert!(matches!(
        store.poll_observer(&subscription).unwrap(),
        crate::ObserverPoll::Batch(_)
    ));
    drop(notices); // Lost receipt must not close the original file or lose discovery.
    assert_separate_reader_denied(&path);
    let again = store.drain_dropped_publications();
    assert_eq!(again.len(), 1);
    assert!(again[0].owner_error.is_none());
    assert!(matches!(
        store.poll_observer(&subscription).unwrap(),
        crate::ObserverPoll::Pending
    ));
    store
        .acknowledge_publication_recovery(capture.operation)
        .unwrap();
    assert_separate_reader_denied(&path); // Receipt itself also owns the original handle.
    drop(again);
    assert!(store.retained_publication_recoveries().is_empty());
    assert_eq!(fs::read(&path).unwrap(), [255, 254, 65, 0, 13, 0, 10, 0]);
    // Acknowledgement releases handles only, never the unresolved logical save scope.
    let (_, binding) = super::super::LoadedBytes::decode(
        Arc::from(&b"other"[..]),
        Some(super::super::Encoding::Utf8),
    )
    .unwrap()
    .attach(&mut store)
    .unwrap();
    let other = path.parent().unwrap().join("other.txt");
    fs::write(&other, b"other").unwrap();
    assert!(matches!(
        ObservedTarget::inspect(&other)
            .unwrap()
            .prepare_bound(&binding, &mut store),
        Err(BoundFailure {
            issue: BoundIssue::File(FileError::Document(Error::UnresolvedConflict)),
            ..
        })
    ));
}
#[test]
fn dropped_permitted_job_is_retained_and_reconciled_once() {
    dropped_publication(false);
}
#[test]
fn dropped_verified_result_keeps_actual_renamed_file_until_owner_ack() {
    dropped_publication(true);
}

#[test]
#[cfg(panic = "unwind")]
fn panicked_after_rename_keeps_poisoned_original_handle_for_inspection_only() {
    let (_dir, target, mut store, _n, ready) = new_ready_fixture();
    let capture = ready.capture().clone();
    let mut job = ready.permit(&mut store).unwrap();
    job.set_post_rename_flush_handle(|_| panic!("controlled post-rename worker unwind"));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run()));
    assert!(result.is_err());
    assert!(target.exists());
    assert_separate_reader_denied(&target);
    let notices = store.drain_dropped_publications();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].terminal_recorded);
    assert!(notices[0].owner_error.is_none());
    assert_eq!(
        notices[0].read_retained_bytes().unwrap(),
        [255, 254, 65, 0, 13, 0, 10, 0]
    );
    store
        .acknowledge_publication_recovery(capture.operation)
        .unwrap();
    drop(notices);
    assert_eq!(fs::read(&target).unwrap(), [255, 254, 65, 0, 13, 0, 10, 0]);
}

#[test]
fn permit_counter_failure_unregisters_only_its_operation_and_keeps_failure_owner() {
    let (dir, other_target, mut store, other_n, other_ready) = new_ready_fixture();
    let other_capture = other_ready.capture().clone();
    let other_job = other_ready.permit(&mut store).unwrap();
    assert_eq!(store.publication_recovery.len(), 1);
    let (other_before, _) = store
        .observe_and_subscribe(other_n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();

    // Separate parent scope, but the same Store and registry as the live other job.
    let overflow_dir = dir.join("overflow");
    fs::create_dir(&overflow_dir).unwrap();
    let target = overflow_dir.join("never-published.txt");
    let raw = b"overflow\r\n";
    let source = overflow_dir.join("overflow-source.txt");
    fs::write(&source, raw).unwrap();
    let (n, mut binding) = match ObservedTarget::inspect(&source)
        .unwrap()
        .open_existing(None)
        .unwrap()
        .attach(&mut store)
        .unwrap()
    {
        super::super::windows_identity::OpenOutcome::Imported { snapshot, binding } => {
            (snapshot, binding)
        }
        _ => panic!("new overflow source import expected"),
    };
    store.test_set_binding_epoch(n.doc, u64::MAX);
    binding.disk.as_mut().unwrap().epoch = u64::MAX;
    let ready = ObservedTarget::inspect(&target)
        .unwrap()
        .prepare_save_as(&binding, &mut store)
        .unwrap()
        .begin(&mut store)
        .unwrap()
        .write()
        .unwrap()
        .lock_temporary()
        .unwrap()
        .prepare_new_owner_only()
        .unwrap();
    let capture = ready.capture().clone();
    assert_eq!(capture.binding_epoch, u64::MAX);
    assert_eq!(
        store.captured_save_stage(&capture),
        Ok(SaveStage::WritingTemp)
    );
    let (before, subscription) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    // CounterExhausted is inside permit_bound_new, after the register call;
    // this is not a cancelled/stale capture rejected by the earlier check.
    let failure = match ready.permit(&mut store) {
        Ok(_) => panic!("exhausted binding epoch received a permit"),
        Err(failure) => failure,
    };
    assert!(matches!(
        &failure.issue,
        PublishIssue::Owner(Error::CounterExhausted)
    ));
    assert_eq!(store.publication_recovery.len(), 1);
    assert!(store.drain_dropped_publications().is_empty());
    assert!(store.retained_publication_recoveries().is_empty());
    assert_eq!(
        store.captured_save_stage(&capture),
        Ok(SaveStage::WritingTemp)
    );
    assert_eq!(
        store.captured_save_stage(&other_capture),
        Ok(SaveStage::Replacing)
    );
    let (after, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(before.state, after.state);
    assert_eq!(before.cursor, after.cursor);
    assert!(matches!(
        store.poll_observer(&subscription).unwrap(),
        crate::ObserverPoll::Pending
    ));
    assert!(!target.exists());
    let temp = failure.current_path().unwrap();
    assert_separate_reader_denied(&temp);
    assert_eq!(failure.read_retained_bytes().unwrap(), raw);
    failure.record_failure(&mut store).unwrap(); // Still owned and active until here.
    assert!(matches!(
        store.poll_observer(&subscription).unwrap(),
        crate::ObserverPoll::Batch(_)
    ));
    assert_eq!(failure.record_failure(&mut store), Err(Error::InvalidSave));
    drop(failure);
    assert_eq!(fs::read(&temp).unwrap(), raw);
    assert!(!target.exists());
    assert_eq!(store.publication_recovery.len(), 1);
    assert!(store.drain_dropped_publications().is_empty());
    let (other_after, _) = store
        .observe_and_subscribe(other_n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(other_before.state, other_after.state);
    assert_eq!(other_before.cursor, other_after.cursor);
    assert_eq!(
        store.captured_save_stage(&other_capture),
        Ok(SaveStage::Replacing)
    );
    // The unrelated entry still owns its real publication and can finish normally.
    other_job.run().unwrap().accept(&mut store).unwrap();
    assert_eq!(store.publication_recovery.len(), 0);
    assert_eq!(
        fs::read(other_target).unwrap(),
        [255, 254, 65, 0, 13, 0, 10, 0]
    );
}

#[test]
fn published_anchor_handoff_failure_keeps_n_plus_one_baseline_and_recovery_resources() {
    let (_dir, target, mut store, n, ready) = new_ready_fixture();
    let capture = ready.capture().clone();
    let job = ready.permit(&mut store).unwrap();
    let current = store
        .submit(Transaction {
            doc: n.doc,
            base: n.revision,
            edits: vec![Edit::input(&n, 0..0, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    let (before, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    let mut published = job.run().unwrap(); // Real native rename/flush/verification in future execution.
    published.set_handoff_path_for_test(target.with_file_name("absent-handoff.txt"));
    let failure = match published.accept(&mut store) {
        Ok(_) => panic!("missing attribute-handoff path accepted"),
        Err(failure) => failure,
    };
    assert!(
        matches!(&failure.issue,PublishIssue::Identity(IdentityError::Io(error)) if error.kind()==std::io::ErrorKind::NotFound)
    );
    assert_eq!(failure.capture().operation, capture.operation);
    let (rejected, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(before.state, rejected.state);
    assert_eq!(before.cursor, rejected.cursor);
    assert_eq!(
        store.captured_save_stage(&capture),
        Ok(SaveStage::Replacing)
    );
    assert_separate_reader_denied(&target);
    assert_eq!(
        failure.read_retained_bytes().unwrap(),
        [255, 254, 65, 0, 13, 0, 10, 0]
    );
    failure.record_failure(&mut store).unwrap();
    let (after, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(
        after.state.global.metadata.baseline,
        before.state.global.metadata.baseline
    );
    assert_eq!(
        after.state.global.metadata.attention.target,
        before.state.global.metadata.attention.target
    );
    assert_eq!(
        after.state.global.metadata.attention.binding_epoch,
        before.state.global.metadata.attention.binding_epoch
    );
    assert_eq!(store.snapshot(n.doc).unwrap().token, current.token);
    assert_eq!(
        after.state.global.metadata.history,
        before.state.global.metadata.history
    );
    assert_eq!(
        after
            .state
            .global
            .metadata
            .save
            .last_terminal
            .as_ref()
            .unwrap()
            .result,
        crate::SaveResult::OutcomeUnknown
    );
    drop(failure);
    let notices = store.drain_dropped_publications();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].terminal_recorded);
    assert!(notices[0].owner_error.is_none());
    assert_eq!(
        notices[0].read_retained_bytes().unwrap(),
        [255, 254, 65, 0, 13, 0, 10, 0]
    );
    store
        .acknowledge_publication_recovery(capture.operation)
        .unwrap();
    drop(notices);
    assert_eq!(fs::read(target).unwrap(), [255, 254, 65, 0, 13, 0, 10, 0]);
}
