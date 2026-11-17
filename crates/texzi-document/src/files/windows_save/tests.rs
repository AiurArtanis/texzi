//! Generated-file Windows unit fixtures. Never run against user documents.
use super::*;
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
        worker.work.set_copy_handle_hook(|new_file| {
            let mut duplicate = std::ptr::null_mut();
            // SAFETY: duplicate this fixture's live handle into the current
            // process with READ_CONTROL only; no reopen or permission elevation.
            let process = unsafe { GetCurrentProcess() };
            if unsafe {
                DuplicateHandle(
                    process,
                    new_file.as_raw_handle(),
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
            // SAFETY: a newly owned duplicate, closed exactly once by File.
            Ok(unsafe { File::from_raw_handle(duplicate) })
        });
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
    if mismatch || denied {
        let failure = match worker.write() {
            Ok(_) => panic!("mismatched security accepted"),
            Err(e) => e,
        };
        if denied {
            assert!(
                matches!(failure.issue, BoundIssue::Stage(StageIssue::Io(e)) if e.raw_os_error() == Some(5))
            );
        } else {
            assert!(matches!(
                failure.issue,
                BoundIssue::Stage(StageIssue::SecurityMismatch)
            ));
        }
        let returned = failure.capture.unwrap();
        assert_eq!(returned.operation, capture.operation);
        assert_eq!(returned.binding_epoch, capture.binding_epoch);
        assert_eq!(returned.snapshot.token, capture.snapshot.token);
        assert!(Arc::ptr_eq(&returned.snapshot.body, &capture.snapshot.body));
        recovery = failure.recovery_temp.unwrap();
        assert_eq!(fs::metadata(&recovery).unwrap().len(), 0);
        assert!(fs::read(&recovery).unwrap().is_empty());
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
    assert_eq!(
        fs::read(&recovery).unwrap(),
        if mismatch || denied {
            &b""[..]
        } else {
            &b"NA\r\n"[..]
        }
    );
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
