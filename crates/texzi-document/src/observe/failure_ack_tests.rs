//! Memory-only counter-boundary fixtures. Only journal.head is injected;
//! the document, captured failure and N -> N+1 edit use ordinary owner APIs.
//! These fixtures do not represent 2^64 emitted batches or filesystem saves.
#![cfg(test)]

use super::*;

const RAW_N: &str = "\u{feff}A\r\n乙\n";

fn fixture() -> (Store, Snapshot, Snapshot, SaveCapture) {
    let mut store = Store::new(HistoryBudget::default());
    let n = store
        .open_utf8(
            RAW_N,
            Profile {
                default_eol: Eol::Cr,
                ..Profile::default()
            },
            false,
        )
        .unwrap();
    let capture = store
        .begin_save(
            n.doc,
            TargetKey("simulated-ack-counter-target-no-file".into()),
            "simulated guard".into(),
        )
        .unwrap();
    let end = n.body.len_bytes();
    let n_plus_one = store
        .submit(Transaction {
            doc: n.doc,
            base: n.revision,
            edits: vec![Edit::input(&n, end..end, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    store
        .finish_save_failure(
            &capture,
            SaveFailure {
                outcome: SaveFailureOutcome::BeforeReplace,
                conflict: false,
                os_error: Some(112),
                message: "simulated pre-replace counter fixture".into(),
                recovery_paths: vec![std::path::PathBuf::from(
                    "simulated-ack-counter-recovery-no-file",
                )],
            },
        )
        .unwrap();
    assert_eq!(n_plus_one.revision.0, n.revision.0 + 1);
    assert_ne!(n_plus_one.token, n.token);
    assert!(!Arc::ptr_eq(&n_plus_one.body, &n.body));
    (store, n, n_plus_one, capture)
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

struct Checkpoint {
    observed: ObserverSnapshot,
    handle: SubscriptionHandle,
    log: VecDeque<Arc<Record>>,
    queue: VecDeque<Arc<Record>>,
    recovery: Snapshot,
    history_depth: (usize, usize),
    history_charge: usize,
    next_subscription: u64,
}

fn checkpoint(store: &mut Store, doc: DocId, operation: u64) -> Checkpoint {
    // Registration reads the injected head and creates an empty queue. It
    // neither advances EventSeq nor attempts to replay the artificial gap.
    let (observed, handle) = store
        .observe_and_subscribe(doc, ObserverScope::new(vec![]), 16)
        .unwrap();
    assert_eq!(store.full_observer_state(doc), observed.state);
    assert!(observed.state.global.metadata.save.active.is_none());
    assert!(store.observer.active_doc.is_none());
    assert!(store.observer.pending_content.is_none());
    let journal = &store.observer.journals[&doc];
    let subscription = &store.observer.subscriptions[&handle.id];
    assert!(!journal.log.is_empty());
    assert!(subscription.queue.is_empty());
    assert!(subscription.lagged.is_none());
    Checkpoint {
        observed,
        handle,
        log: journal.log.clone(),
        queue: subscription.queue.clone(),
        recovery: store.recovery_snapshot(doc, operation).unwrap(),
        history_depth: store.history_depth(doc).unwrap(),
        history_charge: store.history_charge(),
        next_subscription: store.observer.next_subscription,
    }
}

fn assert_same_records(actual: &VecDeque<Arc<Record>>, expected: &VecDeque<Arc<Record>>) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.seq, expected.seq);
        assert!(Arc::ptr_eq(actual, expected));
    }
}

fn assert_unchanged(store: &mut Store, before: &Checkpoint, operation: u64) {
    let doc = before.observed.cursor.doc;
    let journal = &store.observer.journals[&doc];
    assert_eq!(journal.head, before.observed.cursor.seq);
    assert_eq!(journal.last, before.observed.state);
    assert_same_records(&journal.log, &before.log);
    assert_eq!(store.full_observer_state(doc), before.observed.state);
    let subscription = &store.observer.subscriptions[&before.handle.id];
    assert_eq!(subscription.handle.id, before.handle.id);
    assert_eq!(subscription.handle.baseline, before.handle.baseline);
    assert_eq!(subscription.handle.capacity, before.handle.capacity);
    assert!(subscription.lagged.is_none());
    assert_same_records(&subscription.queue, &before.queue);
    assert_eq!(store.observer.next_subscription, before.next_subscription);
    assert!(store.observer.active_doc.is_none());
    assert!(store.observer.pending_content.is_none());
    assert_snapshot(
        &store.snapshot(doc).unwrap(),
        &before.observed.state.global.document,
    );
    assert_snapshot(
        &store.recovery_snapshot(doc, operation).unwrap(),
        &before.recovery,
    );
    assert_eq!(store.history_depth(doc).unwrap(), before.history_depth);
    assert_eq!(store.history_charge(), before.history_charge);
    assert_eq!(
        store.poll_observer(&before.handle).unwrap(),
        ObserverPoll::Pending
    );

    // A fresh public observation also sees the same cursor, baseline,
    // history, failure flag and document state at either injected boundary.
    let (after, temporary) = store
        .observe_and_subscribe(doc, before.observed.cursor.scope.clone(), 16)
        .unwrap();
    assert_eq!(after.cursor, before.observed.cursor);
    assert_eq!(after.state, before.observed.state);
    store.unsubscribe(&temporary).unwrap();
}

#[test]
fn first_pre_replace_ack_at_exhausted_event_counter_changes_nothing() {
    for head in [u64::MAX - 1, u64::MAX] {
        let (mut store, n, n_plus_one, capture) = fixture();
        // Test-only injection after all ordinary fixture writes. No public
        // or production counter setter, real overflow or huge replay is used.
        store.observer.journals.get_mut(&n.doc).unwrap().head = EventSeq(head);
        let before = checkpoint(&mut store, n.doc, capture.operation);
        let failure = &before.observed.state.global.metadata.attention.recoveries[0];
        assert_eq!(failure.operation, capture.operation);
        assert_eq!(failure.stage, SaveStage::Encoding);
        assert!(!failure.acknowledged_before_replace);
        assert_eq!(
            before.observed.state.global.metadata.baseline.token,
            n.token
        );
        assert!(before.observed.state.global.metadata.dirty);
        assert_snapshot(&before.observed.state.global.document, &n_plus_one);
        assert_snapshot(&before.recovery, &n);

        assert!(matches!(
            store.acknowledge_pre_replace_failure(n.doc, capture.operation),
            Err(Error::CounterExhausted)
        ));

        assert_unchanged(&mut store, &before, capture.operation);
        assert_snapshot(&store.snapshot(n.doc).unwrap(), &n_plus_one);
        assert_snapshot(
            &store.recovery_snapshot(n.doc, capture.operation).unwrap(),
            &n,
        );
        assert_eq!(n.body.to_utf8(), RAW_N.as_bytes());
    }
}

#[test]
fn acknowledged_pre_replace_repeat_at_exhausted_event_counter_returns_owned_n_without_events() {
    for head in [u64::MAX - 1, u64::MAX] {
        let (mut store, n, n_plus_one, capture) = fixture();
        let original_state = store.full_observer_state(n.doc);
        let original_failure = original_state.global.metadata.attention.recoveries[0].clone();
        let first = store
            .acknowledge_pre_replace_failure(n.doc, capture.operation)
            .unwrap();
        assert!(first.newly_acknowledged);
        let mut expected_failure = original_failure;
        expected_failure.acknowledged_before_replace = true;
        assert_eq!(first.failure, expected_failure);
        assert_snapshot(&first.snapshot, &n);

        // The first acknowledgement succeeded at normal counters. Only now
        // is the private journal head moved to an artificial exhausted value.
        store.observer.journals.get_mut(&n.doc).unwrap().head = EventSeq(head);
        let before = checkpoint(&mut store, n.doc, capture.operation);
        assert_eq!(
            before.observed.state.global.metadata.attention.recoveries[0],
            expected_failure
        );
        assert_eq!(
            before.observed.state.global.metadata.baseline.token,
            n.token
        );
        assert!(before.observed.state.global.metadata.dirty);
        assert_snapshot(&before.observed.state.global.document, &n_plus_one);
        assert_snapshot(&before.recovery, &n);

        let repeated = store
            .acknowledge_pre_replace_failure(n.doc, capture.operation)
            .unwrap();
        assert!(!repeated.newly_acknowledged);
        assert_eq!(repeated.failure, first.failure);
        assert_eq!(repeated.failure, expected_failure);
        assert_snapshot(&repeated.snapshot, &n);
        assert_unchanged(&mut store, &before, capture.operation);
        assert_snapshot(&store.snapshot(n.doc).unwrap(), &n_plus_one);
        assert_snapshot(
            &store.recovery_snapshot(n.doc, capture.operation).unwrap(),
            &n,
        );

        drop(store);
        assert_eq!(repeated.failure, expected_failure);
        assert_snapshot(&repeated.snapshot, &n);
        assert_eq!(repeated.snapshot.body.to_utf8(), RAW_N.as_bytes());
    }
}
