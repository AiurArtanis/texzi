use texzi_document::*;
fn setup(limits: ObserverLimits) -> (Store, Snapshot, ViewState, ViewState) {
    let mut s = Store::new(HistoryBudget::default());
    s.set_observer_limits(limits).unwrap();
    let d = s.open_utf8("abc", Profile::default(), false).unwrap();
    let open = |s: &mut Store, at| {
        s.open_view(
            d.doc,
            ModeTag::Source,
            SelectionSet::caret(&d, at),
            Anchor {
                position: SourceByte::at(&d, 0),
                affinity: Affinity::Before,
            },
            0,
        )
        .unwrap()
    };
    let a = open(&mut s, 0);
    let b = open(&mut s, 3);
    (s, d, a, b)
}
fn edit(s: &mut Store, doc: DocId, text: &str) -> Receipt {
    let d = s.snapshot(doc).unwrap();
    s.submit(Transaction {
        doc,
        base: d.revision,
        edits: vec![Edit::input(
            &d,
            d.body.len_bytes()..d.body.len_bytes(),
            text,
        )],
        boundary: BoundaryPolicy::Grapheme,
        cancelled: false,
    })
    .unwrap()
}
fn batch(s: &mut Store, h: &SubscriptionHandle) -> EventBatch {
    match s.poll_observer(h).unwrap() {
        ObserverPoll::Batch(b) => b,
        p => panic!("expected batch, got {p:?}"),
    }
}
fn apply(c: &mut ObserverCache, b: EventBatch) {
    for f in b.frames() {
        c.receive(f).unwrap();
    }
}
fn observe(
    s: &mut Store,
    doc: DocId,
    views: Vec<ViewId>,
    capacity: usize,
) -> (ObserverSnapshot, SubscriptionHandle) {
    s.observe_and_subscribe(doc, ObserverScope::new(views), capacity)
        .unwrap()
}
fn installed(snapshot: ObserverSnapshot) -> ObserverCache {
    let mut c = ObserverCache::prepare(snapshot, 4096).unwrap();
    c.install().unwrap();
    c
}
fn navigate(s: &mut Store, id: ViewId, at: usize) {
    let v = s.view(id).unwrap();
    let d = s.snapshot(v.doc).unwrap();
    s.set_selection(id, v.epoch, SelectionSet::caret(&d, at))
        .unwrap();
}

#[test]
fn ex13_atomic_observe_before_after_content_and_install_buffer() {
    let (mut s, d, a, _) = setup(ObserverLimits::default());
    edit(&mut s, d.doc, "!");
    let (snapshot, h) = observe(&mut s, d.doc, vec![a.id], 8);
    let w = snapshot.cursor.seq;
    assert_eq!(
        snapshot.state.global.document.body.source().to_string(),
        "abc!"
    );
    let mut c = ObserverCache::prepare(snapshot, 16).unwrap();
    edit(&mut s, d.doc, "?");
    let b = batch(&mut s, &h);
    assert_eq!(b.cursor.seq.0, w.0 + 1);
    for f in b.frames() {
        assert_eq!(c.receive(f).unwrap(), CacheOutcome::Buffered);
    }
    assert!(c.state().is_none());
    assert_eq!(c.cursor().seq, w);
    assert!(c.cursor_for_resume().is_err());
    c.install().unwrap();
    assert_eq!(
        c.state().unwrap().global.document.body.source().to_string(),
        "abc!?"
    );
    assert_eq!(c.cursor().seq.0, w.0 + 1);
    let (next, handle) = observe(&mut s, d.doc, vec![a.id], 8);
    assert_eq!(next.cursor, c.cursor().clone());
    assert_eq!(s.poll_observer(&handle).unwrap(), ObserverPoll::Pending);
}
#[test]
fn ex14_same_revision_save_history_and_contiguous_resume_to_live() {
    let (mut s, d, a, _) = setup(ObserverLimits::default());
    let capture = s
        .begin_save(d.doc, TargetKey("memory-fixture".into()), "guard".into())
        .unwrap();
    s.advance_save(&capture, SaveStage::WritingTemp).unwrap();
    s.advance_save(&capture, SaveStage::ReadyToReplace).unwrap();
    s.commit_permit(&capture).unwrap();
    s.advance_save(&capture, SaveStage::Verifying).unwrap();
    let (snapshot, old) = observe(&mut s, d.doc, vec![a.id], 8);
    let mut c = installed(snapshot);
    let cached = c.cursor_for_resume().unwrap();
    s.unsubscribe(&old).unwrap();
    s.accept_verified_save(&capture).unwrap();
    s.fence(d.doc, FenceReason::Explicit).unwrap();
    let h = s.resume(d.doc, cached.scope.clone(), &cached, 8).unwrap();
    c.rebind_resume(&h).unwrap();
    navigate(&mut s, a.id, 1);
    for _ in 0..3 {
        apply(&mut c, batch(&mut s, &h));
    }
    let state = c.state().unwrap();
    assert_eq!(state.global.document.revision, d.revision);
    assert_eq!(c.cursor().seq.0, cached.seq.0 + 3);
    assert!(state.global.metadata.save.active.is_none());
    assert_eq!(
        state.global.metadata.baseline.kind,
        BaselineKind::SavedByCaller
    );
    assert_eq!(
        state
            .global
            .metadata
            .save
            .last_terminal
            .as_ref()
            .unwrap()
            .result,
        SaveResult::SucceededByCaller
    );
    assert!(!state.global.metadata.dirty);
    assert_eq!(
        state.global.metadata.history.last_fence,
        FenceReason::Navigation
    );
    assert!(
        matches!(&state.views[&a.id],ObservedView::Open{state,..} if state.selection.selections[0].head.position.offset==1)
    );
}
#[test]
fn ex15_whole_batch_duplicates_partial_and_gap_detection() {
    let (mut s, d, a, _) = setup(ObserverLimits::default());
    let (snapshot, h) = observe(&mut s, d.doc, vec![a.id], 8);
    let mut c = installed(snapshot.clone());
    let old = c.state().unwrap().clone();
    edit(&mut s, d.doc, "X");
    let b = batch(&mut s, &h);
    let frames = b.frames();
    c.receive(frames[0].clone()).unwrap();
    c.receive(frames[1].clone()).unwrap();
    assert_eq!(
        c.receive(frames[1].clone()).unwrap(),
        CacheOutcome::IgnoredDuplicate
    );
    assert_eq!(c.state().unwrap(), &old);
    assert_eq!(c.cursor().seq, snapshot.cursor.seq);
    assert!(c.cursor_for_resume().is_err());
    for f in frames.iter().skip(2) {
        c.receive(f.clone()).unwrap();
    }
    let complete = c.state().unwrap().clone();
    for f in &frames {
        assert_eq!(
            c.receive(f.clone()).unwrap(),
            CacheOutcome::IgnoredDuplicate
        );
    }
    assert_eq!(c.state().unwrap(), &complete);
    let mut corrupted = frames.last().unwrap().clone();
    if let ObserverFrame::End { parts, .. } = &mut corrupted {
        *parts += 1;
    }
    assert_eq!(c.receive(corrupted), Err(ObserverError::Protocol));
    assert!(c.is_stale());
    assert_eq!(c.state().unwrap(), &complete);
    let mut partial = installed(snapshot);
    partial.receive(frames[0].clone()).unwrap();
    assert_eq!(
        partial.receive(frames.last().unwrap().clone()),
        Err(ObserverError::Protocol)
    );
    assert_eq!(partial.cursor().seq, h.baseline.seq);
    let (snapshot, h2) = observe(&mut s, d.doc, vec![a.id], 8);
    let mut gap = installed(snapshot);
    edit(&mut s, d.doc, "1");
    edit(&mut s, d.doc, "2");
    let _ = batch(&mut s, &h2);
    let newer = batch(&mut s, &h2);
    assert_eq!(
        gap.receive(newer.frames()[0].clone()),
        Err(ObserverError::Protocol)
    );
    assert!(gap.is_stale());
}
#[test]
fn ex15_empty_projection_and_old_handle_identity_before_sequence() {
    let (mut s, d, a, b) = setup(ObserverLimits::default());
    navigate(&mut s, b.id, 1); // fence already Navigation
    let (snapshot, h) = observe(&mut s, d.doc, vec![a.id], 8);
    let mut c = installed(snapshot.clone());
    let before = c.state().unwrap().clone();
    navigate(&mut s, b.id, 2);
    let empty = batch(&mut s, &h);
    assert!(empty.events.is_empty());
    apply(&mut c, empty.clone());
    assert_eq!(c.state().unwrap(), &before);
    assert_eq!(c.cursor().seq.0, snapshot.cursor.seq.0 + 1);
    let (next, h2) = observe(&mut s, d.doc, vec![a.id, b.id], 8);
    let mut next = installed(next);
    let mut old = empty.frames()[0].clone();
    if let ObserverFrame::Begin { cursor, .. } = &mut old {
        cursor.seq = EventSeq(9999);
    }
    assert_eq!(next.receive(old).unwrap(), CacheOutcome::IgnoredOldHandle);
    assert!(!next.is_stale());
    let mut bad = ObserverFrame::Begin {
        cursor: h2.baseline.clone(),
        subscription: h2.id,
        parts: 0,
    };
    if let ObserverFrame::Begin { cursor, .. } = &mut bad {
        cursor.scope.schema = 99;
        cursor.seq = EventSeq(0);
    }
    assert_eq!(next.receive(bad), Err(ObserverError::Protocol));
}
#[test]
fn resume_rejects_wrong_store_doc_scope_schema_and_future_cursor() {
    let (mut s, d, a, b) = setup(ObserverLimits::default());
    let (snapshot, h) = observe(&mut s, d.doc, vec![a.id], 8);
    s.unsubscribe(&h).unwrap();
    let cursor = snapshot.cursor;
    let mut bad = cursor.clone();
    bad.store.instance += 1;
    bad.seq = EventSeq(9999);
    assert_eq!(
        s.resume(d.doc, cursor.scope.clone(), &bad, 8).unwrap_err(),
        ObserverError::WrongStore
    );
    let other = s.open_utf8("different", Profile::default(), false).unwrap();
    let mut bad = cursor.clone();
    bad.doc = other.doc;
    assert_eq!(
        s.resume(d.doc, cursor.scope.clone(), &bad, 8).unwrap_err(),
        ObserverError::WrongDoc
    );
    assert_eq!(
        s.resume(d.doc, ObserverScope::new(vec![a.id, b.id]), &cursor, 8)
            .unwrap_err(),
        ObserverError::ScopeMismatch
    );
    let mut bad = cursor.clone();
    bad.scope.schema = 99;
    assert_eq!(
        s.resume(d.doc, bad.scope.clone(), &bad, 8).unwrap_err(),
        ObserverError::UnsupportedSchema
    );
    let mut bad = cursor.clone();
    bad.seq.0 += 1;
    assert_eq!(
        s.resume(d.doc, cursor.scope.clone(), &bad, 8).unwrap_err(),
        ObserverError::CursorAhead
    );
    assert_eq!(
        s.observe_and_subscribe(
            d.doc,
            ObserverScope {
                schema: 1,
                views: vec![a.id, a.id]
            },
            8
        )
        .unwrap_err(),
        ObserverError::InvalidScope
    );
}
#[test]
fn ex16_old_cursor_and_twice_lagged_resync_complete_save_domains() {
    let limits = ObserverLimits {
        journal_batches: 2,
        max_queue_batches: 2,
        max_subscriptions: 8,
    };
    let (mut s, d, a, _) = setup(limits);
    let (snapshot, h) = observe(&mut s, d.doc, vec![a.id], 1);
    let old_cursor = snapshot.cursor.clone();
    let mut first = ObserverCache::prepare(snapshot, 32).unwrap();
    navigate(&mut s, a.id, 1);
    navigate(&mut s, a.id, 2);
    let lag = match s.poll_observer(&h).unwrap() {
        ObserverPoll::Lagged(l) => l,
        p => panic!("{p:?}"),
    };
    first.receive_lagged(&lag).unwrap();
    assert!(first.state().is_none());
    assert_eq!(first.install(), Err(ObserverError::NeedsObserve));
    assert_eq!(first.cursor().seq, old_cursor.seq);
    assert_eq!(s.poll_observer(&h), Err(ObserverError::StaleSubscription));
    navigate(&mut s, a.id, 3);
    assert!(matches!(
        s.resume(d.doc, old_cursor.scope.clone(), &old_cursor, 2),
        Err(ObserverError::CursorTooOld { .. })
    ));
    let (snapshot, h2) = observe(&mut s, d.doc, vec![a.id], 1);
    let w = snapshot.cursor.seq;
    let mut second = ObserverCache::prepare(snapshot, 32).unwrap();
    let capture = s
        .begin_save(d.doc, TargetKey("fixture".into()), "guard".into())
        .unwrap();
    s.advance_save(&capture, SaveStage::WritingTemp).unwrap();
    let lag2 = match s.poll_observer(&h2).unwrap() {
        ObserverPoll::Lagged(l) => l,
        p => panic!("{p:?}"),
    };
    second.receive_lagged(&lag2).unwrap();
    assert_eq!(second.cursor().seq, w);
    assert_eq!(second.install(), Err(ObserverError::NeedsObserve));
    let (snapshot, h3) = observe(&mut s, d.doc, vec![a.id], 1);
    assert_eq!(snapshot.state.global.document.revision, d.revision);
    assert_eq!(
        snapshot
            .state
            .global
            .metadata
            .save
            .active
            .as_ref()
            .unwrap()
            .stage,
        SaveStage::WritingTemp
    );
    let mut third = installed(snapshot);
    navigate(&mut s, a.id, 0);
    apply(&mut third, batch(&mut s, &h3));
    assert_eq!(
        third.receive_lagged(&lag2).unwrap(),
        CacheOutcome::IgnoredOldHandle
    );
    assert!(!third.is_stale());
}
#[test]
fn empty_log_current_cursor_and_replay_capacity_are_explicit() {
    let mut s = Store::new(HistoryBudget::default());
    s.set_observer_limits(ObserverLimits {
        journal_batches: 0,
        max_queue_batches: 4,
        max_subscriptions: 1,
    })
    .unwrap();
    let d = s.open_utf8("a", Profile::default(), false).unwrap();
    let (snapshot, h) = observe(&mut s, d.doc, vec![], 4);
    s.unsubscribe(&h).unwrap();
    let resumed = s
        .resume(d.doc, snapshot.cursor.scope.clone(), &snapshot.cursor, 4)
        .unwrap();
    assert_eq!(s.poll_observer(&resumed).unwrap(), ObserverPoll::Pending);
    s.unsubscribe(&resumed).unwrap();
    edit(&mut s, d.doc, "x");
    assert!(matches!(
        s.resume(d.doc, snapshot.cursor.scope.clone(), &snapshot.cursor, 4),
        Err(ObserverError::CursorTooOld { .. })
    ));
    let (mut s, d, a, _) = setup(ObserverLimits {
        journal_batches: 4,
        max_queue_batches: 4,
        max_subscriptions: 1,
    });
    let (snapshot, h) = observe(&mut s, d.doc, vec![a.id], 4);
    s.unsubscribe(&h).unwrap();
    edit(&mut s, d.doc, "x");
    edit(&mut s, d.doc, "y");
    assert!(matches!(
        s.resume(d.doc, snapshot.cursor.scope.clone(), &snapshot.cursor, 1),
        Err(ObserverError::Lagged(_))
    ));
    let _ = observe(&mut s, d.doc, vec![a.id], 4); // failed resume did not consume a subscription
}
#[test]
fn resume_overflow_marks_cache_stale_without_advancing_watermark() {
    let (mut s, d, a, _) = setup(ObserverLimits {
        journal_batches: 4,
        max_queue_batches: 4,
        max_subscriptions: 4,
    });
    let (snapshot, h) = observe(&mut s, d.doc, vec![a.id], 4);
    let mut c = installed(snapshot.clone());
    let before = c.state().unwrap().clone();
    s.unsubscribe(&h).unwrap();
    edit(&mut s, d.doc, "x");
    edit(&mut s, d.doc, "y");
    let rejection = s
        .resume(d.doc, snapshot.cursor.scope.clone(), &snapshot.cursor, 1)
        .unwrap_err();
    assert!(matches!(rejection, ObserverError::Lagged(_)));
    c.resume_rejected(&rejection).unwrap();
    assert!(c.is_stale());
    assert_eq!(c.cursor(), &snapshot.cursor);
    assert_eq!(c.state().unwrap(), &before);
    assert_eq!(c.cursor_for_resume(), Err(ObserverError::NeedsObserve));
    let (fresh, _) = observe(&mut s, d.doc, vec![a.id], 4);
    let fresh = installed(fresh);
    assert_eq!(
        fresh
            .state()
            .unwrap()
            .global
            .document
            .body
            .source()
            .to_string(),
        "abcxy"
    );
    assert!(!fresh.is_stale());
}
#[test]
fn closed_view_tombstone_document_gone_and_new_store_epoch() {
    let (mut s, d, a, _) = setup(ObserverLimits::default());
    let (snapshot, h) = observe(&mut s, d.doc, vec![a.id], 8);
    let mut c = installed(snapshot);
    let a = s.view(a.id).unwrap();
    s.close_view(a.id, a.epoch).unwrap();
    apply(&mut c, batch(&mut s, &h));
    assert!(matches!(
        c.state().unwrap().views[&a.id],
        ObservedView::Closed(_)
    ));
    edit(&mut s, d.doc, "x");
    apply(&mut c, batch(&mut s, &h));
    let (snapshot, _) = observe(&mut s, d.doc, vec![a.id], 8);
    assert!(matches!(
        snapshot.state.views[&a.id],
        ObservedView::Closed(_)
    ));
    s.close_document(d.doc).unwrap();
    match s.poll_observer(&h).unwrap() {
        ObserverPoll::DocGone {
            cursor,
            subscription,
        } => {
            c.receive_doc_gone(&cursor, subscription).unwrap();
        }
        p => panic!("{p:?}"),
    };
    assert!(c.is_stale());
    assert_eq!(
        s.resume(d.doc, snapshot.cursor.scope.clone(), &snapshot.cursor, 8)
            .unwrap_err(),
        ObserverError::DocGone
    );
    let mut other = Store::new(HistoryBudget::default());
    let new = other.open_utf8("abc", Profile::default(), false).unwrap();
    let (next, _) = observe(&mut other, new.doc, vec![], 8);
    assert_ne!(snapshot.cursor.store, next.cursor.store);
}
#[test]
fn failed_plan_publishes_nothing_and_nested_save_fence_is_one_batch() {
    let (mut s, d, a, _) = setup(ObserverLimits::default());
    let (snapshot, h) = observe(&mut s, d.doc, vec![a.id], 8);
    let mut bad = s.plan_replace(a.id, "X", InputKind::Typing, 0).unwrap();
    bad.selection_after.primary = 9;
    assert_eq!(s.submit_view(bad).unwrap_err(), Error::InvalidSelection);
    assert_eq!(s.poll_observer(&h).unwrap(), ObserverPoll::Pending);
    let capture = s
        .begin_save(d.doc, TargetKey("generated".into()), "guard".into())
        .unwrap();
    let b = batch(&mut s, &h);
    assert_eq!(b.cursor.seq.0, snapshot.cursor.seq.0 + 1);
    let mut c = installed(snapshot);
    apply(&mut c, b);
    let g = &c.state().unwrap().global.metadata;
    assert_eq!(g.history.last_fence, FenceReason::Save);
    assert_eq!(g.save.active.as_ref().unwrap().operation, capture.operation);
    assert_eq!(s.poll_observer(&h).unwrap(), ObserverPoll::Pending);
}
