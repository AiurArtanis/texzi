use texzi_document::*;
fn setup(raw: &str) -> (Store, Snapshot, ViewState, ViewState) {
    let mut st = Store::new(HistoryBudget::default());
    let s = st.open_utf8(raw, Profile::default(), false).unwrap();
    let a = st
        .open_view(
            s.doc,
            ModeTag::Source,
            SelectionSet::caret(&s, 0),
            Anchor {
                position: SourceByte::at(&s, 0),
                affinity: Affinity::Before,
            },
            0,
        )
        .unwrap();
    let b = st
        .open_view(
            s.doc,
            ModeTag::Rich,
            SelectionSet::caret(&s, s.body.len_bytes()),
            Anchor {
                position: SourceByte::at(&s, 0),
                affinity: Affinity::Before,
            },
            24,
        )
        .unwrap();
    (st, s, a, b)
}
fn type_text(st: &mut Store, v: ViewId, text: &str, tick: u64) -> Receipt {
    let p = st.plan_replace(v, text, InputKind::Typing, tick).unwrap();
    st.submit_view(p).unwrap()
}
fn complete(st: &mut Store, c: &SaveCapture) {
    st.advance_save(c, SaveStage::WritingTemp).unwrap();
    st.advance_save(c, SaveStage::ReadyToReplace).unwrap();
    st.commit_permit(c).unwrap();
    st.advance_save(c, SaveStage::Verifying).unwrap();
    st.accept_verified_save(c).unwrap();
}

#[test]
fn ex01_independent_views_atomic_body_and_positions() {
    // EX01's byte16 is the source EOF, not a visual column.
    let (mut st, s, a, b) = setup("cat\nsecond\nthird");
    let b = st
        .set_scroll(
            b.id,
            b.epoch,
            Anchor {
                position: SourceByte::at(&s, 4),
                affinity: Affinity::After,
            },
            24,
        )
        .unwrap();
    let r = type_text(&mut st, a.id, "X", 0);
    let observed = st.read_document(s.doc).unwrap();
    assert_eq!(
        observed.document.body.source().to_string(),
        "Xcat\nsecond\nthird"
    );
    let av = st.view(a.id).unwrap();
    let bv = st.view(b.id).unwrap();
    assert_eq!(av.selection.selections[0].head.position.offset, 1);
    assert_eq!(av.scroll.position.offset, 0);
    assert_eq!(bv.selection.selections[0].head.position.offset, 17);
    assert_eq!(bv.scroll.position.offset, 5);
    assert_eq!(bv.pixel_bias, 24);
    for v in observed.views {
        assert_eq!(
            v.selection.selections[0].head.position.revision,
            r.snapshot.revision
        );
        assert_eq!(v.scroll.position.revision, r.snapshot.revision);
    }
    assert_eq!(st.history_depth(s.doc).unwrap(), (1, 0));
    assert!(st.dirty(s.doc).unwrap());
}
#[test]
fn ex02_cross_view_and_logical_mode_shared_history() {
    let (mut st, s, a, b) = setup("hi");
    let a = st
        .set_selection(a.id, a.epoch, SelectionSet::caret(&s, 2))
        .unwrap();
    type_text(&mut st, a.id, "!", 10);
    let n = type_text(&mut st, b.id, "?", 20).snapshot;
    assert_eq!(n.body.source().to_string(), "hi!?");
    assert_eq!(st.history_depth(s.doc).unwrap(), (2, 0));
    let a = st.view(a.id).unwrap();
    st.undo_view(a.id, a.epoch).unwrap();
    let now = st.snapshot(s.doc).unwrap();
    assert_eq!(now.body.source().to_string(), "hi!");
    let a = st.view(a.id).unwrap();
    st.set_mode_tag(a.id, a.epoch, ModeTag::Rich).unwrap();
    assert_eq!(st.snapshot(s.doc).unwrap().token, now.token);
    assert_eq!(st.history_depth(s.doc).unwrap(), (1, 1));
    let a = st.view(a.id).unwrap();
    let u = st.undo_view(a.id, a.epoch).unwrap();
    assert_eq!(u.snapshot.body.source().to_string(), "hi");
    assert_eq!(u.snapshot.token, s.token);
    assert!(!st.dirty(s.doc).unwrap());
    let b = st.view(b.id).unwrap();
    let r = st.redo_view(b.id, b.epoch).unwrap();
    assert_eq!(r.snapshot.body.source().to_string(), "hi!");
    assert_eq!(
        st.view(b.id).unwrap().selection.selections[0]
            .head
            .position
            .offset,
        3
    );
}
#[test]
fn merged_typing_save_fence_keeps_intermediate_token_reachable() {
    let (mut st, s, a, _) = setup("");
    let one = type_text(&mut st, a.id, "a", 0).snapshot;
    let saved = type_text(&mut st, a.id, "b", 749).snapshot;
    assert_eq!(st.history_depth(s.doc).unwrap(), (1, 0));
    assert_ne!(one.token, saved.token);
    let capture = st
        .begin_save(s.doc, TargetKey("logical-fixture".into()), "guard".into())
        .unwrap();
    type_text(&mut st, a.id, "c", 750);
    type_text(&mut st, a.id, "d", 751);
    assert_eq!(st.history_depth(s.doc).unwrap(), (2, 0));
    complete(&mut st, &capture);
    assert!(st.dirty(s.doc).unwrap());
    let a = st.view(a.id).unwrap();
    let u = st.undo_view(a.id, a.epoch).unwrap();
    assert_eq!(u.snapshot.body.source().to_string(), "ab");
    assert_eq!(u.snapshot.token, saved.token);
    assert!(!st.dirty(s.doc).unwrap());
    let a = st.view(a.id).unwrap();
    st.undo_view(a.id, a.epoch).unwrap();
    assert_eq!(st.snapshot(s.doc).unwrap().token, s.token);
    let a = st.view(a.id).unwrap();
    st.redo_view(a.id, a.epoch).unwrap();
    let a = st.view(a.id).unwrap();
    st.redo_view(a.id, a.epoch).unwrap();
    assert_eq!(
        st.snapshot(s.doc).unwrap().body.source().to_string(),
        "abcd"
    );
}
#[test]
fn navigation_cross_view_paste_ime_and_time_fences() {
    let (mut st, s, a, b) = setup("");
    type_text(&mut st, a.id, "a", 0);
    type_text(&mut st, a.id, "b", 750);
    assert_eq!(st.history_depth(s.doc).unwrap().0, 1);
    type_text(&mut st, a.id, "c", 1501);
    assert_eq!(st.history_depth(s.doc).unwrap().0, 2);
    type_text(&mut st, a.id, "d", 100);
    assert_eq!(st.history_depth(s.doc).unwrap().0, 3); // backwards clock cannot merge
    let a = st.view(a.id).unwrap();
    let cur = st.snapshot(s.doc).unwrap();
    st.set_selection(a.id, a.epoch, SelectionSet::caret(&cur, 4))
        .unwrap();
    type_text(&mut st, a.id, "e", 101);
    type_text(&mut st, b.id, "f", 102);
    type_text(&mut st, a.id, "g", 103);
    assert_eq!(st.history_depth(s.doc).unwrap().0, 6);
    for kind in [
        InputKind::Paste,
        InputKind::ImeCommit,
        InputKind::Structural,
    ] {
        let p = st.plan_replace(a.id, "x", kind, 104).unwrap();
        st.submit_view(p).unwrap();
    }
    type_text(&mut st, a.id, "y", 105);
    assert_eq!(st.history_depth(s.doc).unwrap().0, 10);
}
#[test]
fn illegal_selection_after_origin_or_stale_epoch_does_not_publish() {
    let (mut st, s, a, b) = setup("猫😀");
    let before = st.read_document(s.doc).unwrap();
    let mut p = st.plan_replace(a.id, "狗", InputKind::Typing, 0).unwrap();
    p.selection_after.selections[0].head.position.offset = 4;
    assert_eq!(st.submit_view(p).unwrap_err(), Error::InvalidBoundary);
    assert_eq!(st.snapshot(s.doc).unwrap().token, s.token);
    assert_eq!(st.read_document(s.doc).unwrap().views, before.views);
    assert_eq!(st.history_depth(s.doc).unwrap(), (0, 0));
    let p = st.plan_replace(a.id, "x", InputKind::Typing, 0).unwrap();
    st.set_selection(a.id, a.epoch, SelectionSet::caret(&s, 0))
        .unwrap();
    assert_eq!(st.submit_view(p).unwrap_err(), Error::StaleView);
    let other = st.open_utf8("other", Profile::default(), false).unwrap();
    let mut p = st.plan_replace(b.id, "x", InputKind::Typing, 0).unwrap();
    p.transaction.doc = other.doc;
    assert_eq!(st.submit_view(p).unwrap_err(), Error::InvalidView);
    assert_eq!(st.snapshot(s.doc).unwrap().token, s.token);
    let a = st.view(a.id).unwrap();
    st.close_view(a.id, a.epoch).unwrap();
    assert_eq!(st.view(a.id).unwrap_err(), Error::InvalidView);
}
#[test]
fn normalize_overlap_duplicates_primary_and_multiselect_inverse() {
    let (mut st, s, a, _) = setup("abcdef");
    let sel = |a, b| Selection {
        anchor: Anchor {
            position: SourceByte::at(&s, a),
            affinity: Affinity::Before,
        },
        head: Anchor {
            position: SourceByte::at(&s, b),
            affinity: Affinity::After,
        },
    };
    let intent = SelectionSet {
        selections: vec![sel(4, 6), sel(0, 2), sel(1, 3), Selection::caret(&s, 5)],
        primary: 0,
    };
    let normalized = normalize_selections(&s, intent).unwrap();
    assert_eq!(normalized.primary, 1);
    assert_eq!(
        normalized
            .selections
            .iter()
            .map(Selection::range)
            .collect::<Vec<_>>(),
        vec![0..3, 4..6]
    );
    let a = st.set_selection(a.id, a.epoch, normalized).unwrap();
    let r = type_text(&mut st, a.id, "X", 0);
    assert_eq!(r.snapshot.body.source().to_string(), "XdX");
    assert_eq!(st.view(a.id).unwrap().selection.primary, 1);
    assert_eq!(st.history_depth(s.doc).unwrap(), (1, 0));
    let a = st.view(a.id).unwrap();
    let u = st.undo_view(a.id, a.epoch).unwrap();
    assert_eq!(u.snapshot.body.source().to_string(), "abcdef");
    assert_eq!(
        st.view(a.id)
            .unwrap()
            .selection
            .selections
            .iter()
            .map(Selection::range)
            .collect::<Vec<_>>(),
        vec![0..3, 4..6]
    );
    let n = normalize_selections(
        &s,
        SelectionSet {
            selections: vec![Selection::caret(&s, 1), Selection::caret(&s, 1)],
            primary: 1,
        },
    )
    .unwrap();
    assert_eq!(n.selections.len(), 1);
    assert_eq!(n.primary, 0);
}
#[test]
fn ex06_invalid_multiedit_preserves_all_views_and_open_group() {
    let (mut st, s, a, _) = setup("猫😀");
    let mut p = st.plan_replace(a.id, "a", InputKind::Typing, 0).unwrap();
    p.transaction.edits = vec![Edit::input(&s, 0..3, "狗"), Edit::input(&s, 4..5, "x")];
    let before = st.read_document(s.doc).unwrap();
    assert_eq!(st.submit_view(p).unwrap_err(), Error::InvalidBoundary);
    assert_eq!(st.read_document(s.doc).unwrap().views, before.views);
    assert_eq!(st.history_depth(s.doc).unwrap(), (0, 0));
    let (mut st, s, a, _) = setup("");
    type_text(&mut st, a.id, "a", 0);
    let mut p = st.plan_replace(a.id, "b", InputKind::Typing, 1).unwrap();
    p.selection_after.primary = 9;
    assert_eq!(st.submit_view(p).unwrap_err(), Error::InvalidSelection);
    type_text(&mut st, a.id, "c", 2);
    assert_eq!(st.history_depth(s.doc).unwrap().0, 1);
}
#[test]
fn passive_selection_snap_is_explicit_and_noop_mode_is_not_content() {
    let (mut st, s, a, b) = setup("ab");
    let mut passive = SelectionSet::caret(&s, 1);
    passive.selections[0].anchor.affinity = Affinity::Before;
    passive.selections[0].head.affinity = Affinity::Before;
    let b = st.set_selection(b.id, b.epoch, passive).unwrap();
    let a = st
        .set_selection(a.id, a.epoch, SelectionSet::caret(&s, 1))
        .unwrap();
    type_text(&mut st, a.id, "\u{301}", 0);
    let b = st.view(b.id).unwrap();
    assert_eq!(b.selection.selections[0].head.position.offset, 0);
    assert_eq!(b.mapping_snaps.len(), 2);
    assert_eq!(
        (b.mapping_snaps[0].requested, b.mapping_snaps[0].actual),
        (1, 0)
    );
    let token = st.snapshot(s.doc).unwrap().token;
    let revision = st.snapshot(s.doc).unwrap().revision;
    let depth = st.history_depth(s.doc).unwrap();
    st.set_mode_tag(b.id, b.epoch, ModeTag::Source).unwrap();
    assert_eq!(st.snapshot(s.doc).unwrap().token, token);
    assert_eq!(st.snapshot(s.doc).unwrap().revision, revision);
    assert_eq!(st.history_depth(s.doc).unwrap(), depth);
}
