use texzi_document::*;
#[test]
fn native_multiselection_plan_is_atomic_and_undo_restores_owner() {
    let mut store = Store::new(HistoryBudget::default());
    let s = store
        .open_utf8("甲乙\nabc", Profile::default(), false)
        .unwrap();
    let set = SelectionSet::caret(&s, 0);
    let v = store
        .open_view(
            s.doc,
            ModeTag::Source,
            set.clone(),
            set.selections[0].head,
            0,
        )
        .unwrap();
    let plan = store
        .plan_view_edits(
            v.id,
            s.revision,
            v.epoch,
            vec![(0..3, "中".into()), (7..10, "xy".into())],
            vec![(3, 3), (9, 9)],
            1,
            InputKind::Structural,
            1,
        )
        .unwrap();
    let r = store.submit_view(plan).unwrap();
    assert_eq!(r.snapshot.body.source().to_string(), "中乙\nxy");
    assert_eq!(store.view(v.id).unwrap().selection.primary, 1);
    let current = store.view(v.id).unwrap();
    let before = store.snapshot(s.doc).unwrap();
    assert!(
        store
            .plan_view_edits(
                v.id,
                s.revision,
                v.epoch,
                vec![],
                vec![(0, 0)],
                0,
                InputKind::Typing,
                2
            )
            .is_err()
    );
    assert!(
        store
            .plan_view_edits(
                v.id,
                before.revision,
                current.epoch,
                vec![(0..0, "z".into())],
                vec![(999, 999)],
                0,
                InputKind::Typing,
                2
            )
            .is_err()
    );
    assert_eq!(store.snapshot(s.doc).unwrap().token, before.token);
    store.undo_view(v.id, current.epoch).unwrap();
    assert_eq!(
        store.snapshot(s.doc).unwrap().body.source().to_string(),
        "甲乙\nabc"
    );
}
