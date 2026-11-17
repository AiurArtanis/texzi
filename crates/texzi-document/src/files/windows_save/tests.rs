//! Generated-file Windows unit fixtures. Never run against user documents.
use super::*;
use crate::{
    BoundaryPolicy, Edit, HistoryBudget, ObserverScope, SaveFailure, SaveFailureOutcome,
    Transaction,
};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(1);

fn security_boundary(mismatch: bool) {
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
    let calls = Arc::new(AtomicUsize::new(0));
    let hook_calls = calls.clone();
    let required = expected.clone();
    let applied = if mismatch { changed } else { expected };
    worker.work.set_security_hook(move |new_file| {
        assert_eq!(new_file.metadata()?.len(), 0);
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
    if mismatch {
        let failure = match worker.write() {
            Ok(_) => panic!("mismatched security accepted"),
            Err(e) => e,
        };
        assert!(matches!(
            failure.issue,
            BoundIssue::Stage(StageIssue::SecurityMismatch)
        ));
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
        staged.check_owner(&store).unwrap();
        recovery = staged.recovery_temp().to_owned();
        assert_eq!(fs::read(&recovery).unwrap(), b"NA\r\n");
        let locked = staged.lock_temporary().unwrap();
        assert_eq!(
            locked.original_security().unwrap(),
            locked.temporary_security()
        );
        store.cancel_save(locked.capture()).unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
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
        if mismatch {
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
}

#[test]
fn mismatched_security_rejects_before_payload_and_finishes_original_capture() {
    security_boundary(true);
}
#[test]
fn matching_security_keeps_captured_payload_and_newer_edit_independent() {
    security_boundary(false);
}
