//! Simulated owner protocol only. No filesystem IO or verified disk-save evidence.
use std::{path::PathBuf, sync::Arc};
use texzi_document::*;

const RAW: &str = "\u{feff}A\r\n乙\n";

fn key(value: &str) -> TargetKey {
    TargetKey(value.into())
}

fn scope(parent: &str, object: &str) -> SaveScope {
    SaveScope {
        parent: key(parent),
        object: Some(key(object)),
    }
}

fn fixture() -> (Store, Snapshot) {
    let mut store = Store::new(HistoryBudget::default());
    let snapshot = store
        .open_utf8(
            RAW,
            Profile {
                default_eol: Eol::Cr,
                ..Profile::default()
            },
            false,
        )
        .unwrap();
    (store, snapshot)
}

fn append(store: &mut Store, doc: DocId, text: &str) -> Snapshot {
    let current = store.snapshot(doc).unwrap();
    let end = current.body.len_bytes();
    store
        .submit(Transaction {
            doc,
            base: current.revision,
            edits: vec![Edit::input(&current, end..end, text)],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot
}

fn advance_to(store: &mut Store, capture: &SaveCapture, stage: SaveStage) {
    if stage == SaveStage::Encoding {
        return;
    }
    store.advance_save(capture, SaveStage::WritingTemp).unwrap();
    if stage == SaveStage::WritingTemp {
        return;
    }
    store
        .advance_save(capture, SaveStage::ReadyToReplace)
        .unwrap();
    if stage == SaveStage::ReadyToReplace {
        return;
    }
    store.commit_permit(capture).unwrap();
    if stage == SaveStage::Verifying {
        store.advance_save(capture, stage).unwrap();
    }
}

fn fail(store: &mut Store, capture: &SaveCapture, outcome: SaveFailureOutcome, conflict: bool) {
    store
        .finish_save_failure(
            capture,
            SaveFailure {
                outcome,
                conflict,
                os_error: Some(112),
                message: "simulated acknowledgement fixture failure".into(),
                recovery_paths: vec![PathBuf::from("simulated-recovery-path-no-file")],
            },
        )
        .unwrap();
}

fn observe(store: &mut Store, doc: DocId) -> (ObserverSnapshot, SubscriptionHandle) {
    store
        .observe_and_subscribe(doc, ObserverScope::new(vec![]), 16)
        .unwrap()
}

fn assert_snapshot(actual: &Snapshot, expected: &Snapshot) {
    assert_eq!(actual.doc, expected.doc);
    assert_eq!(actual.revision, expected.revision);
    assert_eq!(actual.token, expected.token);
    assert!(Arc::ptr_eq(&actual.body, &expected.body));
    assert_eq!(actual.body.profile, expected.body.profile);
    assert_eq!(actual.body.eols(), expected.body.eols());
    assert_eq!(actual.body.to_utf8(), expected.body.to_utf8());
}

fn assert_unchanged(store: &mut Store, before: &ObserverSnapshot, handle: &SubscriptionHandle) {
    let (after, temporary) = store
        .observe_and_subscribe(before.cursor.doc, before.cursor.scope.clone(), 16)
        .unwrap();
    assert_eq!(after.cursor, before.cursor);
    assert_eq!(after.state, before.state);
    assert_eq!(store.poll_observer(handle).unwrap(), ObserverPoll::Pending);
    store.unsubscribe(&temporary).unwrap();
}

fn assert_only_acknowledgement(
    store: &mut Store,
    before: &ObserverSnapshot,
    handle: &SubscriptionHandle,
    operation: u64,
) -> ObserverSnapshot {
    let mut expected = before.state.clone();
    let record = Arc::make_mut(&mut expected.global)
        .metadata
        .attention
        .recoveries
        .iter_mut()
        .find(|record| record.operation == operation)
        .unwrap();
    assert!(!record.acknowledged_before_replace);
    record.acknowledged_before_replace = true;

    let batch = match store.poll_observer(handle).unwrap() {
        ObserverPoll::Batch(batch) => batch,
        other => panic!("expected acknowledgement batch, got {other:?}"),
    };
    let mut cursor = before.cursor.clone();
    cursor.seq = EventSeq(cursor.seq.0 + 1);
    assert_eq!(batch.cursor, cursor);
    assert_eq!(batch.subscription, handle.id);
    match batch.events.as_slice() {
        [ObserverEvent::Global(global)] => assert_eq!(global, &expected.global),
        other => panic!("expected one Global and no Content/View, got {other:?}"),
    }
    let (after, temporary) = store
        .observe_and_subscribe(before.cursor.doc, before.cursor.scope.clone(), 16)
        .unwrap();
    assert_eq!(after.cursor, cursor);
    assert_eq!(after.state, expected);
    assert_eq!(store.poll_observer(handle).unwrap(), ObserverPoll::Pending);
    store.unsubscribe(&temporary).unwrap();
    after
}

#[test]
fn pre_replace_stages_acknowledge_once_keep_n_and_allow_explicit_close() {
    for stage in [
        SaveStage::Encoding,
        SaveStage::WritingTemp,
        SaveStage::ReadyToReplace,
    ] {
        let (mut store, n) = fixture();
        let view = store
            .open_view(
                n.doc,
                ModeTag::Source,
                SelectionSet::caret(&n, 0),
                Anchor {
                    position: SourceByte::at(&n, 0),
                    affinity: Affinity::Before,
                },
                7,
            )
            .unwrap();
        let initial = store
            .begin_save(n.doc, key("simulated-original"), "original".into())
            .unwrap();
        super::complete(&mut store, &initial); // Caller receipt simulation, no IO.
        let capture = store
            .begin_scoped_save(
                n.doc,
                key("simulated-failed-save-as"),
                "guard".into(),
                scope("simulated-parent", "simulated-failed-save-as"),
            )
            .unwrap();
        advance_to(&mut store, &capture, stage);
        let newer = append(&mut store, n.doc, "B");
        fail(
            &mut store,
            &capture,
            SaveFailureOutcome::BeforeReplace,
            false,
        );
        let depth = store.history_depth(n.doc).unwrap();
        let charge = store.history_charge();
        let (before, handle) = store
            .observe_and_subscribe(n.doc, ObserverScope::new(vec![view.id]), 16)
            .unwrap();
        assert_eq!(before.state.global.document.revision.0, n.revision.0 + 1);
        assert_eq!(before.state.global.metadata.attention.binding_epoch, 1);
        assert!(before.state.global.metadata.dirty);
        assert_eq!(store.close_document(n.doc), Err(Error::UnresolvedConflict));
        assert_unchanged(&mut store, &before, &handle);

        let mut expected_failure = before.state.global.metadata.attention.recoveries[0].clone();
        assert_eq!(expected_failure.stage, stage);
        assert!(!expected_failure.acknowledged_before_replace);
        expected_failure.acknowledged_before_replace = true;
        let receipt = store
            .acknowledge_pre_replace_failure(n.doc, capture.operation)
            .unwrap();
        assert!(receipt.newly_acknowledged);
        assert_eq!(receipt.failure, expected_failure);
        assert_snapshot(&receipt.snapshot, &n);
        let after = assert_only_acknowledgement(&mut store, &before, &handle, capture.operation);
        assert_snapshot(&store.snapshot(n.doc).unwrap(), &newer);
        assert_snapshot(
            &store.recovery_snapshot(n.doc, capture.operation).unwrap(),
            &n,
        );
        assert_eq!(store.history_depth(n.doc).unwrap(), depth);
        assert_eq!(store.history_charge(), charge);

        let repeated = store
            .acknowledge_pre_replace_failure(n.doc, capture.operation)
            .unwrap();
        assert!(!repeated.newly_acknowledged);
        assert_eq!(repeated.failure, receipt.failure);
        assert_snapshot(&repeated.snapshot, &receipt.snapshot);
        assert_unchanged(&mut store, &after, &handle);

        store.close_document(n.doc).unwrap();
        assert!(matches!(store.snapshot(n.doc), Err(Error::InvalidDoc)));
        assert_snapshot(&receipt.snapshot, &n);
        assert_eq!(receipt.snapshot.body.to_utf8(), RAW.as_bytes());
        assert!(matches!(
            store.poll_observer(&handle).unwrap(),
            ObserverPoll::DocGone { .. }
        ));
    }
}

#[test]
fn wrong_document_operation_and_newer_active_save_publish_nothing() {
    let (mut store, n) = fixture();
    let first = store
        .begin_save(n.doc, key("simulated-first"), String::new())
        .unwrap();
    fail(&mut store, &first, SaveFailureOutcome::BeforeReplace, false);
    let other = store.open_utf8("other", Profile::default(), false).unwrap();
    let foreign = store
        .begin_save(other.doc, key("simulated-foreign"), String::new())
        .unwrap();
    fail(
        &mut store,
        &foreign,
        SaveFailureOutcome::BeforeReplace,
        false,
    );
    let closed = store
        .open_utf8("closed", Profile::default(), false)
        .unwrap();
    store.close_document(closed.doc).unwrap();
    let (before, handle) = observe(&mut store, n.doc);
    let (other_before, other_handle) = observe(&mut store, other.doc);
    for (doc, operation, error) in [
        (n.doc, 0, Error::InvalidSave),
        (n.doc, foreign.operation, Error::InvalidSave),
        (other.doc, first.operation, Error::InvalidSave),
        (closed.doc, first.operation, Error::InvalidDoc),
    ] {
        assert!(matches!(
            store.acknowledge_pre_replace_failure(doc, operation),
            Err(actual) if actual == error
        ));
        assert_unchanged(&mut store, &before, &handle);
        assert_unchanged(&mut store, &other_before, &other_handle);
    }
    let newer = store
        .begin_save(n.doc, key("simulated-newer"), "new guard".into())
        .unwrap();
    let (active, active_handle) = observe(&mut store, n.doc);
    assert!(matches!(
        store.acknowledge_pre_replace_failure(n.doc, first.operation),
        Err(Error::Busy)
    ));
    assert!(matches!(
        store.acknowledge_pre_replace_failure(n.doc, newer.operation),
        Err(Error::InvalidSave)
    ));
    assert_eq!(store.close_document(n.doc), Err(Error::Busy));
    assert_unchanged(&mut store, &active, &active_handle);
    assert_unchanged(&mut store, &other_before, &other_handle);
    store.cancel_save(&newer).unwrap();
    let (cancelled, cancelled_handle) = observe(&mut store, n.doc);
    let receipt = store
        .acknowledge_pre_replace_failure(n.doc, first.operation)
        .unwrap();
    assert!(receipt.newly_acknowledged);
    assert_snapshot(&receipt.snapshot, &n);
    assert_only_acknowledgement(&mut store, &cancelled, &cancelled_handle, first.operation);
    assert_unchanged(&mut store, &other_before, &other_handle);

    let latest = store
        .begin_save(n.doc, key("simulated-active-after-ack"), String::new())
        .unwrap();
    let (acknowledged_active, latest_handle) = observe(&mut store, n.doc);
    assert!(matches!(
        store.acknowledge_pre_replace_failure(n.doc, first.operation),
        Err(Error::Busy)
    ));
    assert_unchanged(&mut store, &acknowledged_active, &latest_handle);
    store.cancel_save(&latest).unwrap();
    let (idle, idle_handle) = observe(&mut store, n.doc);
    let repeated = store
        .acknowledge_pre_replace_failure(n.doc, first.operation)
        .unwrap();
    assert!(!repeated.newly_acknowledged);
    assert_snapshot(&repeated.snapshot, &n);
    assert_unchanged(&mut store, &idle, &idle_handle);
}

#[test]
fn conflict_replace_failed_and_unknown_keep_held_domains_and_independent_save() {
    for (stage, outcome, conflict) in [
        (
            SaveStage::ReadyToReplace,
            SaveFailureOutcome::BeforeReplace,
            true,
        ),
        (
            SaveStage::Replacing,
            SaveFailureOutcome::ReplaceFailed,
            false,
        ),
        (
            SaveStage::Verifying,
            SaveFailureOutcome::OutcomeUnknown,
            false,
        ),
    ] {
        let (mut store, n) = fixture();
        let other = store.open_utf8("other", Profile::default(), false).unwrap();
        let probe = store.open_utf8("probe", Profile::default(), false).unwrap();
        let capture = store
            .begin_scoped_save(
                n.doc,
                key("simulated-held-object"),
                "guard".into(),
                scope("simulated-held-parent", "simulated-held-object"),
            )
            .unwrap();
        advance_to(&mut store, &capture, stage);
        fail(&mut store, &capture, outcome, conflict);
        let independent = store
            .begin_scoped_save(
                other.doc,
                key("simulated-independent-object"),
                "independent".into(),
                scope(
                    "simulated-independent-parent",
                    "simulated-independent-object",
                ),
            )
            .unwrap();
        let (before, handle) = observe(&mut store, n.doc);
        let (other_before, other_handle) = observe(&mut store, other.doc);
        assert_eq!(
            before.state.global.metadata.attention.conflict,
            Unimplemented::NeedsVerification {
                operation: capture.operation,
                target: capture.target.clone(),
            }
        );
        if outcome == SaveFailureOutcome::OutcomeUnknown {
            assert_eq!(
                before.state.global.metadata.attention.save_outcome_unknown,
                Unimplemented::NeedsVerification {
                    operation: capture.operation,
                    target: capture.target.clone(),
                }
            );
        }
        assert!(matches!(
            store.acknowledge_pre_replace_failure(n.doc, capture.operation),
            Err(Error::UnresolvedConflict)
        ));
        assert!(matches!(
            store.begin_save(n.doc, key("simulated-new-target"), String::new()),
            Err(Error::UnresolvedConflict)
        ));
        assert!(matches!(
            store.begin_save(probe.doc, capture.target.clone(), String::new()),
            Err(Error::UnresolvedConflict)
        ));
        assert!(matches!(
            store.begin_scoped_save(
                probe.doc,
                key("simulated-different-object"),
                String::new(),
                scope("simulated-held-parent", "simulated-different-object"),
            ),
            Err(Error::UnresolvedConflict)
        ));
        // The memory API requires object == target. This checks their combined
        // protection; an isolated object-map assertion needs a private lib unit.
        assert!(matches!(
            store.begin_scoped_save(
                probe.doc,
                capture.target.clone(),
                String::new(),
                scope("simulated-other-parent", "simulated-held-object"),
            ),
            Err(Error::UnresolvedConflict)
        ));
        assert_eq!(store.close_document(n.doc), Err(Error::UnresolvedConflict));
        assert_snapshot(
            &store.recovery_snapshot(n.doc, capture.operation).unwrap(),
            &n,
        );
        assert_unchanged(&mut store, &before, &handle);
        assert_unchanged(&mut store, &other_before, &other_handle);
        store.cancel_save(&independent).unwrap();
    }
}

#[test]
fn successful_retry_keeps_old_failure_unacknowledged_and_old_ack_keeps_new_binding() {
    let (mut store, n) = fixture();
    let initial = store
        .begin_save(n.doc, key("simulated-original"), String::new())
        .unwrap();
    super::complete(&mut store, &initial); // Simulated owner receipt only.
    let failed = store
        .begin_save(n.doc, key("simulated-failed"), String::new())
        .unwrap();
    let n_plus_one = append(&mut store, n.doc, "B");
    fail(
        &mut store,
        &failed,
        SaveFailureOutcome::BeforeReplace,
        false,
    );
    let retry = store
        .begin_save(n.doc, key("simulated-successful-retry"), "new guard".into())
        .unwrap();
    assert_snapshot(&retry.snapshot, &n_plus_one);
    let n_plus_two = append(&mut store, n.doc, "C");
    super::complete(&mut store, &retry); // No disk result is being asserted.
    let (before, handle) = observe(&mut store, n.doc);
    let metadata = &before.state.global.metadata;
    assert_eq!(metadata.baseline.token, n_plus_one.token);
    assert_eq!(metadata.attention.binding_epoch, 2);
    assert_eq!(metadata.attention.target, Some(retry.target.clone()));
    assert!(metadata.dirty);
    assert!(!metadata.attention.recoveries[0].acknowledged_before_replace);
    assert_eq!(
        metadata.save.last_terminal.as_ref().unwrap().operation,
        retry.operation
    );
    assert_eq!(
        metadata.save.last_terminal.as_ref().unwrap().result,
        SaveResult::SucceededByCaller
    );
    assert_eq!(store.close_document(n.doc), Err(Error::UnresolvedConflict));
    assert_unchanged(&mut store, &before, &handle);
    let depth = store.history_depth(n.doc).unwrap();
    let charge = store.history_charge();
    let receipt = store
        .acknowledge_pre_replace_failure(n.doc, failed.operation)
        .unwrap();
    assert_snapshot(&receipt.snapshot, &n);
    let after = assert_only_acknowledgement(&mut store, &before, &handle, failed.operation);
    assert_snapshot(&store.snapshot(n.doc).unwrap(), &n_plus_two);
    assert_eq!(store.history_depth(n.doc).unwrap(), depth);
    assert_eq!(store.history_charge(), charge);
    let other = store.open_utf8("other", Profile::default(), false).unwrap();
    assert!(matches!(
        store.begin_save(other.doc, retry.target.clone(), String::new()),
        Err(Error::Busy)
    ));
    assert_unchanged(&mut store, &after, &handle);
    assert_eq!(store.undo(n.doc).unwrap().snapshot.token, n_plus_one.token);
    assert!(!store.dirty(n.doc).unwrap());
    assert_eq!(store.redo(n.doc).unwrap().snapshot.token, n_plus_two.token);
    assert!(store.dirty(n.doc).unwrap());
}

#[test]
fn every_failure_requires_acknowledgement_and_later_unknown_blocks_old_ack() {
    let (mut store, n) = fixture();
    let first = store
        .begin_save(n.doc, key("simulated-first"), String::new())
        .unwrap();
    fail(&mut store, &first, SaveFailureOutcome::BeforeReplace, false);
    let newer = append(&mut store, n.doc, "B");
    let second = store
        .begin_save(n.doc, key("simulated-second"), String::new())
        .unwrap();
    fail(
        &mut store,
        &second,
        SaveFailureOutcome::BeforeReplace,
        false,
    );
    let (before, handle) = observe(&mut store, n.doc);
    assert_eq!(before.state.global.metadata.attention.recoveries.len(), 2);
    let first_receipt = store
        .acknowledge_pre_replace_failure(n.doc, first.operation)
        .unwrap();
    let after_first = assert_only_acknowledgement(&mut store, &before, &handle, first.operation);
    assert_eq!(store.close_document(n.doc), Err(Error::UnresolvedConflict));
    assert_snapshot(
        &store.recovery_snapshot(n.doc, second.operation).unwrap(),
        &newer,
    );
    assert_unchanged(&mut store, &after_first, &handle);
    let second_receipt = store
        .acknowledge_pre_replace_failure(n.doc, second.operation)
        .unwrap();
    assert_only_acknowledgement(&mut store, &after_first, &handle, second.operation);
    store.close_document(n.doc).unwrap();
    assert_snapshot(&first_receipt.snapshot, &n);
    assert_snapshot(&second_receipt.snapshot, &newer);

    for already_acknowledged in [false, true] {
        let (mut store, n) = fixture();
        let old = store
            .begin_save(n.doc, key("simulated-old-eligible"), String::new())
            .unwrap();
        fail(&mut store, &old, SaveFailureOutcome::BeforeReplace, false);
        if already_acknowledged {
            let receipt = store
                .acknowledge_pre_replace_failure(n.doc, old.operation)
                .unwrap();
            assert!(receipt.newly_acknowledged);
            assert_snapshot(&receipt.snapshot, &n);
        }
        let unknown = store
            .begin_save(n.doc, key("simulated-later-unknown"), String::new())
            .unwrap();
        advance_to(&mut store, &unknown, SaveStage::Verifying);
        fail(
            &mut store,
            &unknown,
            SaveFailureOutcome::OutcomeUnknown,
            false,
        );
        let (before, handle) = observe(&mut store, n.doc);
        assert_eq!(
            before.state.global.metadata.attention.recoveries[0].acknowledged_before_replace,
            already_acknowledged
        );
        assert!(matches!(
            store.acknowledge_pre_replace_failure(n.doc, old.operation),
            Err(Error::UnresolvedConflict)
        ));
        assert_eq!(store.close_document(n.doc), Err(Error::UnresolvedConflict));
        assert_unchanged(&mut store, &before, &handle);
    }
}

#[test]
fn acknowledging_old_failure_does_not_release_another_documents_reused_scope() {
    let (mut store, n) = fixture();
    let resolver = scope("simulated-reused-parent", "simulated-reused-object");
    let failed = store
        .begin_scoped_save(
            n.doc,
            key("simulated-reused-object"),
            "old guard".into(),
            resolver.clone(),
        )
        .unwrap();
    fail(
        &mut store,
        &failed,
        SaveFailureOutcome::BeforeReplace,
        false,
    );
    let other = store.open_utf8("other", Profile::default(), false).unwrap();
    let newer = store
        .begin_scoped_save(
            other.doc,
            failed.target.clone(),
            "new guard".into(),
            resolver.clone(),
        )
        .unwrap();
    let (before, handle) = observe(&mut store, n.doc);
    let (other_before, other_handle) = observe(&mut store, other.doc);
    let receipt = store
        .acknowledge_pre_replace_failure(n.doc, failed.operation)
        .unwrap();
    assert!(receipt.newly_acknowledged);
    assert_snapshot(&receipt.snapshot, &n);
    let after = assert_only_acknowledgement(&mut store, &before, &handle, failed.operation);
    assert_unchanged(&mut store, &other_before, &other_handle);
    assert!(matches!(
        store.begin_save(n.doc, failed.target.clone(), String::new()),
        Err(Error::Busy)
    ));
    assert!(matches!(
        store.begin_scoped_save(
            n.doc,
            key("simulated-another-object"),
            String::new(),
            scope("simulated-reused-parent", "simulated-another-object"),
        ),
        Err(Error::Busy)
    ));
    // object == target also applies here; do not claim an isolated object gate.
    assert!(matches!(
        store.begin_scoped_save(
            n.doc,
            failed.target.clone(),
            String::new(),
            scope("simulated-another-parent", "simulated-reused-object"),
        ),
        Err(Error::Busy)
    ));
    assert_unchanged(&mut store, &after, &handle);
    assert_unchanged(&mut store, &other_before, &other_handle);
    store.cancel_save(&newer).unwrap();
    let next = store
        .begin_scoped_save(n.doc, failed.target.clone(), String::new(), resolver)
        .unwrap();
    assert_eq!(next.operation, newer.operation + 1);
    store.cancel_save(&next).unwrap();
}
