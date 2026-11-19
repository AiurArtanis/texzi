//! Resolver scopes and terminal receipts are simulated; no filesystem IO.
use texzi_document::*;
fn key(s: &str) -> TargetKey {
    TargetKey(s.into())
}
fn scope(parent: &str, file: Option<&str>) -> SaveScope {
    SaveScope {
        parent: key(parent),
        object: file.map(key),
    }
}
fn doc(store: &mut Store) -> DocId {
    store.open_utf8("A", Profile::default(), false).unwrap().doc
}
#[test]
fn separate_directory_and_file_domains_conflict_and_cancel_releases_only_its_operation() {
    let mut store = Store::new(HistoryBudget::default());
    let a = doc(&mut store);
    let b = doc(&mut store);
    let c = doc(&mut store);
    let first = store
        .begin_scoped_save(
            a,
            key("absent-target"),
            String::new(),
            scope("parent", None),
        )
        .unwrap();
    assert!(matches!(
        store.begin_scoped_save(
            b,
            key("new-file-id"),
            String::new(),
            scope("parent", Some("new-file-id"))
        ),
        Err(Error::Busy)
    ));
    store.cancel_save(&first).unwrap();
    let replacement = store
        .begin_scoped_save(
            b,
            key("new-file-id"),
            String::new(),
            scope("parent", Some("new-file-id")),
        )
        .unwrap();
    assert_eq!(store.cancel_save(&first), Err(Error::InvalidSave));
    assert!(matches!(
        store.begin_scoped_save(
            a,
            key("another-target"),
            String::new(),
            scope("parent", None)
        ),
        Err(Error::Busy)
    ));
    // Another directory's hardlink is excluded by the object domain.
    assert!(matches!(
        store.begin_scoped_save(
            c,
            key("new-file-id"),
            String::new(),
            scope("other-parent", Some("new-file-id"))
        ),
        Err(Error::Busy)
    ));
    let unrelated = store
        .begin_scoped_save(
            c,
            key("unrelated-id"),
            String::new(),
            scope("other-parent", Some("unrelated-id")),
        )
        .unwrap();
    store.cancel_save(&unrelated).unwrap();
    store.cancel_save(&replacement).unwrap();
    let wrong = store.begin_scoped_save(
        a,
        key("wrong-object"),
        String::new(),
        scope("parent", Some("actual-object")),
    );
    assert!(matches!(wrong, Err(Error::InvalidSave)));
    let free = store
        .begin_scoped_save(
            a,
            key("actual-object"),
            String::new(),
            scope("parent", Some("actual-object")),
        )
        .unwrap();
    store.cancel_save(&free).unwrap();
}
#[test]
fn failures_release_or_hold_both_domains_and_recovery_snapshot_records_the_scope() {
    for (unknown, conflict) in [(false, false), (false, true), (true, false)] {
        let mut store = Store::new(HistoryBudget::default());
        let a = doc(&mut store);
        let b = doc(&mut store);
        let resolver = scope("parent", Some("file-id"));
        let c = store
            .begin_scoped_save(a, key("file-id"), String::new(), resolver.clone())
            .unwrap();
        if unknown {
            store.advance_save(&c, SaveStage::WritingTemp).unwrap();
            store.advance_save(&c, SaveStage::ReadyToReplace).unwrap();
            store.commit_permit(&c).unwrap();
            store.advance_save(&c, SaveStage::Verifying).unwrap();
        }
        store
            .finish_save_failure(
                &c,
                SaveFailure {
                    outcome: if unknown {
                        SaveFailureOutcome::OutcomeUnknown
                    } else {
                        SaveFailureOutcome::BeforeReplace
                    },
                    conflict,
                    os_error: None,
                    message: "memory-only failure receipt".into(),
                    recovery_paths: vec![],
                },
            )
            .unwrap();
        let (snapshot, _) = store
            .observe_and_subscribe(a, ObserverScope::new(vec![]), 8)
            .unwrap();
        assert_eq!(
            snapshot.state.global.metadata.attention.recoveries[0].scope,
            Some(resolver)
        );
        assert!(snapshot.state.global.metadata.save.active.is_none());
        let same_parent = store.begin_scoped_save(
            b,
            key("different-object"),
            String::new(),
            scope("parent", Some("different-object")),
        );
        if unknown || conflict {
            assert!(matches!(same_parent, Err(Error::UnresolvedConflict)));
            assert!(matches!(
                store.begin_scoped_save(
                    b,
                    key("file-id"),
                    String::new(),
                    scope("other-parent", Some("file-id"))
                ),
                Err(Error::UnresolvedConflict)
            ));
        } else {
            store.cancel_save(&same_parent.unwrap()).unwrap();
            let free = store
                .begin_scoped_save(
                    b,
                    key("file-id"),
                    String::new(),
                    scope("other-parent", Some("file-id")),
                )
                .unwrap();
            store.cancel_save(&free).unwrap();
        }
    }
}
