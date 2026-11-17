//! Isolated generated-file fixtures; retained paths are evidence only after execution.
//! NOT_RUN: source review has not established native OS 5/32 behavior.
use super::{ExistingReadCapture, ExistingReadFailure, ExistingReadIssue};
use crate::files::{
    windows_identity::{FileVersion, IdentityError, ObservedTarget, OpenOutcome},
    windows_save::LockedStaged,
};
use crate::{
    BaselineKind, BoundaryPolicy, DocId, Edit, Error, HistoryBudget, ObserverScope,
    ObserverSnapshot, SaveCapture, SaveStage, Snapshot, StateToken, Store, Transaction,
};
use std::{
    fs::{self, File, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    target: PathBuf,
    expected_path: PathBuf,
    expected_target: Option<FileVersion>,
    store: Store,
    baseline: StateToken,
    n: Snapshot,
    capture: SaveCapture,
    staged: LockedStaged,
}

fn fixture(label: &str, existing: bool) -> Fixture {
    let dir = std::env::temp_dir().join(format!(
        "texzi-t07-existing-read-{label}-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    eprintln!(
        "T07 generated existing-read fixture retained: {}",
        dir.display()
    );
    let source = dir.join("original.txt");
    fs::write(&source, b"A\r\n").unwrap();
    let target = if existing {
        source.clone()
    } else {
        dir.join("absent.txt")
    };
    let mut store = Store::new(HistoryBudget::default());
    let (initial, binding) = match ObservedTarget::inspect(&source)
        .unwrap()
        .open_existing(None)
        .unwrap()
        .attach(&mut store)
        .unwrap()
    {
        OpenOutcome::Imported { snapshot, binding } => (snapshot, binding),
        _ => panic!("generated source should attach as a new verified document"),
    };
    let n = store
        .submit(Transaction {
            doc: initial.doc,
            base: initial.revision,
            edits: vec![Edit::input(&initial, 0..0, "N")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    let observed = ObservedTarget::inspect(&target).unwrap();
    let expected_path = observed.resolved_path();
    let expected_target = observed.version().cloned();
    let prepared = if existing {
        observed.prepare_current(&binding, &mut store)
    } else {
        observed.prepare_save_as(&binding, &mut store)
    }
    .unwrap();
    let capture = prepared.capture().clone();
    let staged = prepared
        .begin(&mut store)
        .unwrap()
        .write()
        .unwrap()
        .lock_temporary()
        .unwrap();
    assert_eq!(
        store.captured_save_stage(&capture),
        Ok(SaveStage::WritingTemp)
    );
    Fixture {
        target,
        expected_path,
        expected_target,
        store,
        baseline: initial.token,
        n,
        capture,
        staged,
    }
}

struct Frozen {
    observed: ObserverSnapshot,
    history_depth: (usize, usize),
    history_charge: usize,
}

fn freeze(store: &mut Store, doc: DocId) -> Frozen {
    Frozen {
        observed: store
            .observe_and_subscribe(doc, ObserverScope::new(vec![]), 8)
            .unwrap()
            .0,
        history_depth: store.history_depth(doc).unwrap(),
        history_charge: store.history_charge(),
    }
}

fn assert_unchanged(store: &mut Store, before: &Frozen) {
    let doc = before.observed.state.global.document.doc;
    let (after, _) = store
        .observe_and_subscribe(doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(after.state, before.observed.state);
    assert_eq!(after.cursor, before.observed.cursor);
    assert_eq!(store.history_depth(doc).unwrap(), before.history_depth);
    assert_eq!(store.history_charge(), before.history_charge);
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

fn assert_retained(staged: &LockedStaged, capture: &SaveCapture, temp: &Path) {
    assert_capture(staged.capture(), capture);
    assert_eq!(staged.recovery_temp(), temp);
    assert_eq!(staged.payload(), b"NA\r\n");
    assert_eq!(staged.read_created_bytes().unwrap(), b"NA\r\n");
    assert_eq!(
        staged.format(),
        capture.snapshot.body.profile.file_format.unwrap()
    );
}

fn capture_failure<T>(result: Result<T, ExistingReadFailure>) -> ExistingReadFailure {
    match result {
        Ok(_) => panic!("existing-read capture unexpectedly succeeded"),
        Err(failure) => failure,
    }
}

// Sequential convenience for fixture code only. Production callers receive
// the owned worker so no Store borrow needs to cross its filesystem reads.
fn capture_read(
    staged: LockedStaged,
    store: &Store,
) -> Result<ExistingReadCapture, ExistingReadFailure> {
    staged.begin_existing_read(store)?.run()?.accept(store)
}

fn ordinary_writer(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
}

#[test]
fn current_read_capture_retries_after_writer_and_keeps_baseline_n_and_n_plus_one_separate() {
    let Fixture {
        target,
        expected_path,
        expected_target,
        mut store,
        baseline,
        n,
        capture,
        staged,
    } = fixture("current-writer", true);
    let temp = staged.recovery_temp().to_owned();
    let work = staged.begin_existing_read(&store).unwrap();
    assert_capture(work.capture(), &capture);
    // Mutable Store access is available while the worker input is owned.
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
    let before = freeze(&mut store, n.doc);
    assert_eq!(
        before.observed.state.global.metadata.baseline.token,
        baseline
    );
    assert_eq!(
        before.observed.state.global.metadata.baseline.kind,
        BaselineKind::VerifiedOpen
    );
    assert!(before.observed.state.global.metadata.dirty);
    assert_ne!(capture.snapshot.token, next.token);

    // A future native execution must reject an already-open ordinary writer.
    let writer = ordinary_writer(&target).unwrap();
    let failure = capture_failure(work.run());
    assert!(matches!(
        &failure.issue,
        ExistingReadIssue::Identity(IdentityError::Io(error))
            if error.raw_os_error() == Some(32)
    ));
    assert_retained(&failure.staged, &capture, &temp);
    assert_eq!(fs::read(&target).unwrap(), b"A\r\n");
    assert_unchanged(&mut store, &before);
    drop(writer);

    let held = capture_read(failure.staged, &store).unwrap();
    assert_capture(held.capture(), &capture);
    assert_eq!(held.format(), n.body.profile.file_format.unwrap());
    assert_eq!(held.payload(), b"NA\r\n");
    assert_eq!(held.read_staged_bytes().unwrap(), b"NA\r\n");
    let expected = expected_target.unwrap();
    assert_eq!(held.expected_target_version(), &expected);
    assert_eq!(held.current_target_version().unwrap(), expected);
    assert_eq!(held.current_target_path().unwrap(), expected_path);
    assert_eq!(held.expected_target_version().bytes.as_ref(), b"A\r\n");
    assert_eq!(fs::read(&target).unwrap(), b"A\r\n");
    held.recheck(&store).unwrap();
    assert_eq!(
        ordinary_writer(&target).unwrap_err().raw_os_error(),
        Some(32)
    );
    assert_eq!(
        fs::remove_file(&target).unwrap_err().raw_os_error(),
        Some(32)
    );
    assert_unchanged(&mut store, &before);

    let staged = held.into_staged();
    assert_retained(&staged, &capture, &temp);
    staged.check_owner(&store).unwrap();
    drop(ordinary_writer(&target).unwrap());
    assert_eq!(
        store.captured_save_stage(&capture),
        Ok(SaveStage::WritingTemp)
    );
    assert_eq!(store.snapshot(n.doc).unwrap().token, next.token);
    assert_eq!(store.snapshot(n.doc).unwrap().body.to_utf8(), b"BNA\r\n");
    assert_eq!(fs::read(&target).unwrap(), b"A\r\n");
    assert_unchanged(&mut store, &before);
}

#[test]
fn changed_target_rejects_read_capture_and_retains_staged_n_and_external_bytes() {
    let Fixture {
        target,
        mut store,
        n,
        capture,
        staged,
        ..
    } = fixture("external-change", true);
    let temp = staged.recovery_temp().to_owned();
    let before = freeze(&mut store, n.doc);
    fs::write(&target, b"external complete bytes\r\n").unwrap();
    let failure = capture_failure(capture_read(staged, &store));
    assert!(matches!(
        &failure.issue,
        ExistingReadIssue::Identity(IdentityError::Changed)
    ));
    assert_retained(&failure.staged, &capture, &temp);
    failure.staged.check_owner(&store).unwrap();
    assert_eq!(
        store.captured_save_stage(&capture),
        Ok(SaveStage::WritingTemp)
    );
    assert_eq!(fs::read(&target).unwrap(), b"external complete bytes\r\n");
    assert_unchanged(&mut store, &before);
}

#[test]
fn cancelled_read_capture_preserves_staged_n_and_new_same_target_encoding_reservation() {
    let Fixture {
        target,
        mut store,
        n,
        capture,
        staged,
        ..
    } = fixture("cancelled-old", true);
    let temp = staged.recovery_temp().to_owned();
    store.cancel_save(&capture).unwrap();
    let next = {
        let observed = ObservedTarget::inspect(&target).unwrap();
        store
            .begin_scoped_save(
                n.doc,
                observed.reservation_key(),
                "new reservation after cancelling the read-capture owner".into(),
                observed.save_scope(),
            )
            .unwrap()
    };
    assert_ne!(next.operation, capture.operation);
    assert_eq!(next.target, capture.target);
    let before = freeze(&mut store, n.doc);
    let failure = capture_failure(capture_read(staged, &store));
    assert!(matches!(
        &failure.issue,
        ExistingReadIssue::Owner(Error::InvalidSave)
    ));
    assert_retained(&failure.staged, &capture, &temp);
    assert_eq!(failure.staged.check_owner(&store), Err(Error::InvalidSave));
    assert_eq!(store.captured_save_stage(&capture), Err(Error::InvalidSave));
    assert_eq!(store.captured_save_stage(&next), Ok(SaveStage::Encoding));
    assert_eq!(fs::read(&target).unwrap(), b"A\r\n");
    assert_unchanged(&mut store, &before);
}

#[test]
fn absent_destination_rejects_read_capture_without_creating_target_or_leaving_writing_temp() {
    let Fixture {
        target,
        expected_target,
        mut store,
        n,
        capture,
        staged,
        ..
    } = fixture("absent-target", false);
    assert!(expected_target.is_none());
    assert!(!target.try_exists().unwrap());
    let temp = staged.recovery_temp().to_owned();
    let before = freeze(&mut store, n.doc);
    let failure = capture_failure(capture_read(staged, &store));
    assert!(matches!(
        &failure.issue,
        ExistingReadIssue::NoExistingTarget
    ));
    assert_retained(&failure.staged, &capture, &temp);
    failure.staged.check_owner(&store).unwrap();
    assert_eq!(
        store.captured_save_stage(&capture),
        Ok(SaveStage::WritingTemp)
    );
    assert!(!target.try_exists().unwrap());
    assert_unchanged(&mut store, &before);
}

#[test]
fn held_capture_recheck_rejects_cancelled_and_closed_owner_without_losing_n() {
    let Fixture {
        target,
        mut store,
        n,
        capture,
        staged,
        ..
    } = fixture("held-owner-close", true);
    let temp = staged.recovery_temp().to_owned();
    let held = capture_read(staged, &store).unwrap();
    store.cancel_save(&capture).unwrap();
    let cancelled = freeze(&mut store, n.doc);
    assert!(matches!(
        held.recheck(&store),
        Err(ExistingReadIssue::Owner(Error::InvalidSave))
    ));
    assert_unchanged(&mut store, &cancelled);

    // This unit explicitly closes its generated in-memory document after
    // cancellation. It does not drive a UI discard or claim that N was saved.
    store.close_document(n.doc).unwrap();
    assert!(matches!(store.snapshot(n.doc), Err(Error::InvalidDoc)));
    assert!(matches!(
        held.recheck(&store),
        Err(ExistingReadIssue::Owner(Error::InvalidSave))
    ));
    assert_eq!(
        held.current_target_version().unwrap().bytes.as_ref(),
        b"A\r\n"
    );
    assert_eq!(held.read_staged_bytes().unwrap(), b"NA\r\n");
    assert_capture(held.capture(), &capture);
    let retained = held.into_staged();
    assert_retained(&retained, &capture, &temp);
    assert_eq!(fs::read(target).unwrap(), b"A\r\n");
}

#[test]
fn late_worker_candidate_rejects_the_new_owner_and_releases_its_read_capture() {
    let Fixture {
        target,
        mut store,
        n,
        capture,
        staged,
        ..
    } = fixture("late-worker-candidate", true);
    let temp = staged.recovery_temp().to_owned();
    let work = staged.begin_existing_read(&store).unwrap();
    let candidate = work.run().unwrap();
    assert_capture(candidate.capture(), &capture);
    store.cancel_save(&capture).unwrap();
    let next = {
        let observed = ObservedTarget::inspect(&target).unwrap();
        store
            .begin_scoped_save(
                n.doc,
                observed.reservation_key(),
                "new reservation before accepting the old read".into(),
                observed.save_scope(),
            )
            .unwrap()
    };
    let before = freeze(&mut store, n.doc);
    let failure = capture_failure(candidate.accept(&store));
    assert!(matches!(
        &failure.issue,
        ExistingReadIssue::Owner(Error::InvalidSave)
    ));
    assert_retained(&failure.staged, &capture, &temp);
    assert_eq!(store.captured_save_stage(&next), Ok(SaveStage::Encoding));
    assert_eq!(store.captured_save_stage(&capture), Err(Error::InvalidSave));
    // Rejecting a stale candidate releases only its target read File. The
    // original temp/context remain in failure.staged; no terminal is recorded.
    drop(ordinary_writer(&target).unwrap());
    assert_eq!(fs::read(target).unwrap(), b"A\r\n");
    assert_unchanged(&mut store, &before);
}
