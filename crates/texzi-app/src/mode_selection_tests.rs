//! FR-MD-10 through actual Source/Rich logical views and their single Store.
use super::*;

const RAW: &str = "zero😀\r\n\r\nsecond &amp; end\n\n<custom>keep</custom>\r\n";

#[derive(Debug, PartialEq, Eq)]
struct BodyAudit {
    bytes: Vec<u8>,
    revision: u64,
    token: texzi_document::StateToken,
    profile: texzi_document::Profile,
    eols: Vec<texzi_document::Eol>,
    origins: String,
    history: (usize, usize),
    dirty: bool,
}
fn body(view: &NativeView) -> BodyAudit {
    view.inspect(|store, id| {
        let s = store.snapshot(store.view(id).unwrap().doc).unwrap();
        BodyAudit {
            bytes: s.body.to_utf8(),
            revision: s.revision.0,
            token: s.token,
            profile: s.body.profile,
            eols: s.body.eols(),
            origins: format!("{:?}", s.body.origins()),
            history: store.history_depth(s.doc).unwrap(),
            dirty: store.dirty(s.doc).unwrap(),
        }
    })
}
fn view_state(view: &NativeView) -> String {
    view.inspect(|store, id| format!("{:?}", store.view(id).unwrap()))
}
fn runtime(f: impl FnOnce()) {
    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    f();
}
fn fixture() -> (
    Rc<NativeView>,
    Rc<NativeView>,
    Rc<ModeGate>,
    ExternalSelections,
) {
    let source = NativeView::fixture(RAW);
    let rich = source.sibling().unwrap();
    let gate = ModeGate::new(source.clone(), rich.clone());
    let s = source.snapshot().unwrap();
    let at = s.rope.to_string().find("second").unwrap();
    let selected = source
        .select(
            s.revision,
            s.view_epoch,
            ExternalSelections {
                primary: 1,
                ranges: vec![(0, 4), (at + 6, at)],
            },
        )
        .unwrap()
        .selections;
    assert_eq!(selected.primary, 1);
    assert_eq!(selected.ranges.len(), 2);
    assert!(
        selected.ranges[1].0 > selected.ranges[1].1,
        "primary direction is intentionally reversed"
    );
    (source, rich, gate, selected)
}
fn enter(gate: &ModeGate) {
    gate.to_rich();
    assert!(gate.rich_mode.get(), "{}", gate.status.get());
    assert_eq!(gate.status.get(), "排版视图使用主选区");
}
fn select_rich(gate: &ModeGate, a: usize, h: usize) {
    let session = gate.session();
    let state = session.state();
    let p = Projection::parse(
        state.schema(),
        gate.rich.snapshot().unwrap().rope.to_string(),
    )
    .unwrap();
    let mut tr = state.tr();
    tr.set_selection(TextSelection::create(
        state.doc(),
        p.rich_position(a).unwrap(),
        Some(p.rich_position(h).unwrap()),
    ));
    session.dispatch(tr).unwrap();
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
}
fn assert_primary(source: &NativeView, original: &ExternalSelections) {
    assert_eq!(
        source.snapshot().unwrap().selections,
        ExternalSelections {
            primary: 0,
            ranges: vec![original.ranges[original.primary]],
        }
    );
}

#[test]
fn multi_source_roundtrip_keeps_primary_direction_original_view_and_body_with_one_notice() {
    runtime(|| {
        let (source, rich, gate, multi) = fixture();
        let before = body(&source);
        let source_view = view_state(&source);
        enter(&gate);
        assert_eq!(source.snapshot().unwrap().selections, multi);
        assert_eq!(
            rich.snapshot().unwrap().selections,
            ExternalSelections {
                primary: 0,
                ranges: vec![multi.ranges[multi.primary]],
            }
        );
        let rich_snapshot = rich.snapshot().unwrap();
        let state = gate.session().state();
        let p = Projection::parse(state.schema(), rich_snapshot.rope.to_string()).unwrap();
        assert_eq!(
            p.source_position(state.selection().anchor()),
            Some(multi.ranges[1].0)
        );
        assert_eq!(
            p.source_position(state.selection().head()),
            Some(multi.ranges[1].1)
        );
        // Repeated requests/render reads do not re-emit the transition notice.
        gate.status.set("已读".into());
        gate.to_rich();
        assert_eq!(gate.status.get(), "已读");
        select_rich(&gate, multi.ranges[1].0, multi.ranges[1].1); // exact no-op
        assert_eq!(
            rich.snapshot().unwrap().view_epoch,
            rich_snapshot.view_epoch
        );
        gate.to_source();
        assert!(!gate.rich_mode.get(), "{}", gate.status.get());
        assert!(gate.status.get().is_empty());
        assert_eq!(source.snapshot().unwrap().selections, multi);
        assert_eq!(
            view_state(&source),
            source_view,
            "reuse the same Source view and its original anchors"
        );
        gate.source_mounted();
        assert_eq!(body(&source), before);
        assert_eq!(body(&rich), before);
    });
}

#[test]
fn rich_navigation_away_and_back_invalidates_old_multi_even_without_body_change() {
    runtime(|| {
        let (source, rich, gate, multi) = fixture();
        let before = body(&source);
        enter(&gate);
        let epoch = rich.snapshot().unwrap().view_epoch;
        select_rich(&gate, 1, 1);
        select_rich(&gate, multi.ranges[1].0, multi.ranges[1].1);
        assert!(rich.snapshot().unwrap().view_epoch > epoch);
        gate.to_source();
        assert!(!gate.rich_mode.get());
        assert_primary(&source, &multi);
        assert!(gate.status.get().contains("原多选已因正文或光标变化失效"));
        gate.status.set("已读".into());
        gate.to_source();
        assert_eq!(gate.status.get(), "已读", "no repeated transition notice");
        assert_eq!(
            body(&source),
            before,
            "navigation does not touch revision/token/history/dirty"
        );
    });
}

#[test]
fn source_selection_generation_change_also_invalidates_old_multi_after_return_to_same_ranges() {
    runtime(|| {
        let (source, _, gate, multi) = fixture();
        let before = body(&source);
        enter(&gate);
        let s = source.snapshot().unwrap();
        source
            .select(
                s.revision,
                s.view_epoch,
                ExternalSelections {
                    primary: 0,
                    ranges: vec![(1, 1)],
                },
            )
            .unwrap();
        let s = source.snapshot().unwrap();
        source
            .select(s.revision, s.view_epoch, multi.clone())
            .unwrap();
        gate.to_source();
        assert!(!gate.rich_mode.get());
        assert_primary(&source, &multi);
        assert!(gate.status.get().contains("原多选已因正文或光标变化失效"));
        assert_eq!(body(&source), before);
    });
}

#[test]
fn body_edit_then_undo_does_not_restore_old_multi_or_consume_remaining_redo() {
    runtime(|| {
        let (source, _, gate, multi) = fixture();
        let original = body(&source);
        enter(&gate);
        let session = gate.session();
        let mut tr = session.state().tr();
        tr.insert_text("changed", None, None);
        session.dispatch(tr).unwrap();
        assert!(gate.status.get().is_empty(), "{}", gate.status.get());
        gate.shell_history(true).unwrap();
        let undone = body(&source);
        assert_eq!(undone.bytes, original.bytes);
        assert_eq!(undone.token, original.token);
        assert!(undone.revision > original.revision);
        assert_eq!(undone.history, (0, 1));
        gate.to_source();
        assert!(!gate.rich_mode.get());
        assert_primary(&source, &multi);
        assert!(gate.status.get().contains("原多选已因正文或光标变化失效"));
        assert_eq!(
            body(&source),
            undone,
            "mode switch must preserve the redo and dirty state"
        );
    });
}

#[test]
fn blocked_switches_keep_multi_atomic_and_raw_source_entry_never_revives_it() {
    runtime(|| {
        let (source, rich, gate, multi) = fixture();
        let before = body(&source);
        let views = (view_state(&source), view_state(&rich));
        source.set_composing(true);
        gate.to_rich();
        assert!(!gate.rich_mode.get());
        assert_eq!((view_state(&source), view_state(&rich)), views);
        source.set_composing(false);
        assert!(
            gate.prepare_and_switch(|| Err("controlled prepare failure".into()))
                .is_err()
        );
        assert!(!gate.rich_mode.get());
        assert_eq!((view_state(&source), view_state(&rich)), views);
        enter(&gate);
        let views = (view_state(&source), view_state(&rich));
        rich.set_composing(true);
        gate.to_source();
        assert!(gate.rich_mode.get());
        assert_eq!((view_state(&source), view_state(&rich)), views);
        rich.set_composing(false);
        gate.to_source();
        assert!(!gate.rich_mode.get());
        assert_eq!(source.snapshot().unwrap().selections, multi);
        gate.source_mounted();
        enter(&gate);
        let action = gate.raw_blocks().into_iter().next().unwrap();
        let mut stale = action.clone();
        stale.revision += 1;
        let views = (view_state(&source), view_state(&rich));
        assert!(gate.edit_raw_block(&stale).is_err());
        assert!(gate.rich_mode.get());
        assert_eq!((view_state(&source), view_state(&rich)), views);
        gate.edit_raw_block(&action).unwrap();
        assert!(!gate.rich_mode.get());
        let local = ExternalSelections {
            primary: 0,
            ranges: vec![(action.source.start, action.source.end)],
        };
        assert_eq!(source.snapshot().unwrap().selections, local);
        gate.source_mounted();
        gate.to_source();
        assert_eq!(source.snapshot().unwrap().selections, local);
        assert_eq!(body(&source), before);
        assert_eq!(body(&rich), before);
    });
}
