use texzi_document::*;
fn failure(outcome: SaveFailureOutcome, conflict: bool) -> SaveFailure {
    SaveFailure {
        outcome,
        conflict,
        os_error: Some(112),
        message: "generated fixture failure".into(),
        recovery_paths: vec![std::path::PathBuf::from("generated-temp-not-real-io")],
    }
}
#[test]
fn failure_retains_snapshot_and_unknown_blocks_retry_after_reservation_release() {
    for stage in [
        SaveStage::Encoding,
        SaveStage::WritingTemp,
        SaveStage::ReadyToReplace,
        SaveStage::Replacing,
        SaveStage::Verifying,
    ] {
        let mut store = Store::new(HistoryBudget::default());
        let s = store.open_utf8("A", Profile::default(), false).unwrap();
        let target = TargetKey("memory-target".into());
        let c = store
            .begin_save(s.doc, target.clone(), "guard".into())
            .unwrap();
        if stage != SaveStage::Encoding {
            store.advance_save(&c, SaveStage::WritingTemp).unwrap();
        }
        if matches!(
            stage,
            SaveStage::ReadyToReplace | SaveStage::Replacing | SaveStage::Verifying
        ) {
            store.advance_save(&c, SaveStage::ReadyToReplace).unwrap();
        }
        if matches!(stage, SaveStage::Replacing | SaveStage::Verifying) {
            store.commit_permit(&c).unwrap();
        }
        if stage == SaveStage::Verifying {
            store.advance_save(&c, stage).unwrap();
        }
        let current = store
            .submit(Transaction {
                doc: s.doc,
                base: s.revision,
                edits: vec![Edit::input(&s, 1..1, "B")],
                boundary: BoundaryPolicy::Grapheme,
                cancelled: false,
            })
            .unwrap()
            .snapshot;
        let before = store.history_depth(s.doc).unwrap();
        let outcome = match stage {
            SaveStage::Replacing => SaveFailureOutcome::ReplaceFailed,
            SaveStage::Verifying => SaveFailureOutcome::OutcomeUnknown,
            _ => SaveFailureOutcome::BeforeReplace,
        };
        let conflict = stage == SaveStage::ReadyToReplace;
        let (_, handle) = store
            .observe_and_subscribe(s.doc, ObserverScope::new(vec![]), 8)
            .unwrap();
        store
            .finish_save_failure(&c, failure(outcome, conflict))
            .unwrap();
        assert!(matches!(
            store.poll_observer(&handle).unwrap(),
            ObserverPoll::Batch(_)
        ));
        assert!(matches!(
            store.poll_observer(&handle).unwrap(),
            ObserverPoll::Pending
        ));
        let (observed, _) = store
            .observe_and_subscribe(s.doc, ObserverScope::new(vec![]), 8)
            .unwrap();
        assert_eq!(observed.state.global.metadata.attention.recoveries.len(), 1);
        assert!(observed.state.global.metadata.save.active.is_none());
        assert_eq!(store.snapshot(s.doc).unwrap().token, current.token);
        assert_eq!(store.history_depth(s.doc).unwrap(), before);
        assert!(store.dirty(s.doc).unwrap());
        let recovered = store.recovery_snapshot(s.doc, c.operation).unwrap();
        assert_eq!(recovered.token, s.token);
        assert_eq!(recovered.body.to_utf8(), b"A");
        assert_eq!(
            store.finish_save_failure(&c, failure(outcome, conflict)),
            Err(Error::InvalidSave)
        );
        let other = store.open_utf8("other", Profile::default(), false).unwrap();
        if conflict || outcome != SaveFailureOutcome::BeforeReplace {
            assert!(matches!(
                observed.state.global.metadata.attention.conflict,
                Unimplemented::NeedsVerification { .. }
            ));
            if outcome == SaveFailureOutcome::OutcomeUnknown {
                assert!(matches!(
                    observed
                        .state
                        .global
                        .metadata
                        .attention
                        .save_outcome_unknown,
                    Unimplemented::NeedsVerification { .. }
                ));
            }
            assert!(matches!(
                store.begin_save(s.doc, TargetKey("different".into()), String::new()),
                Err(Error::UnresolvedConflict)
            ));
            assert!(matches!(
                store.begin_save(other.doc, target, String::new()),
                Err(Error::UnresolvedConflict)
            ));
        } else {
            let next = store.begin_save(s.doc, target, String::new()).unwrap();
            assert_eq!(
                store.finish_save_failure(&c, failure(outcome, false)),
                Err(Error::InvalidSave)
            );
            assert!(matches!(
                store.begin_save(s.doc, TargetKey("other".into()), String::new()),
                Err(Error::Busy)
            ));
            store.cancel_save(&next).unwrap();
        }
    }
}
#[test]
fn forged_capture_and_wrong_failure_phase_do_not_release_active_operation() {
    let mut store = Store::new(HistoryBudget::default());
    let s = store.open_utf8("A", Profile::default(), false).unwrap();
    let other = store.open_utf8("other", Profile::default(), false).unwrap();
    let c = store
        .begin_save(s.doc, TargetKey("target".into()), "guard".into())
        .unwrap();
    for field in 0..4 {
        let mut bad = c.clone();
        match field {
            0 => bad.operation += 1,
            1 => bad.binding_epoch += 1,
            2 => bad.target = TargetKey("wrong".into()),
            _ => bad.snapshot.body = other.body.clone(),
        };
        assert_eq!(
            store.finish_save_failure(&bad, failure(SaveFailureOutcome::BeforeReplace, false)),
            Err(Error::InvalidSave)
        );
    }
    assert_eq!(
        store.finish_save_failure(&c, failure(SaveFailureOutcome::OutcomeUnknown, false)),
        Err(Error::InvalidSave)
    );
    assert!(matches!(
        store.begin_save(s.doc, TargetKey("new".into()), String::new()),
        Err(Error::Busy)
    ));
    store.cancel_save(&c).unwrap();
}
