//! Installed Enter bindings through ModeGate -> RichOwner -> NativeView ->
//! Store. Selection moves; body, revision, token, dirty and history do not.
use crate::{
    editor_owner::NativeView,
    rich_owner::ModeGate,
    rich_projection::Projection,
    table_controls::cell_position,
    table_navigation::{self, EnterTarget},
};
use nsh_base::{
    editor::{ExternalDocument, ExternalEditKind, ExternalSelections},
    rich_editor::state::{KeyEvent, TextSelection},
};
use nsh_core::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn runtime(f: impl FnOnce()) {
    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    f();
}
fn body_audit(view: &NativeView) -> String {
    view.inspect(|store, id| {
        let doc = store.view(id).unwrap().doc;
        let snapshot = store.snapshot(doc).unwrap();
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            snapshot.revision,
            snapshot.token,
            snapshot.body.to_utf8(),
            snapshot.body.eols(),
            snapshot.body.origins(),
            store.dirty(doc),
            store.history_depth(doc)
        )
    })
}
fn bytes(view: &NativeView) -> Vec<u8> {
    view.inspect(|store, id| {
        store
            .snapshot(store.view(id).unwrap().doc)
            .unwrap()
            .body
            .to_utf8()
    })
}
fn select(gate: &ModeGate, block: usize, row: usize, column: usize) {
    let session = gate.session();
    let state = session.state();
    let position = cell_position(state.doc(), block, row, column).unwrap();
    let mut tr = state.tr();
    tr.set_selection(TextSelection::create(state.doc(), position, None));
    session.dispatch(tr).unwrap();
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
}
fn installed_enter(gate: &ModeGate) {
    let session = gate.session();
    let state = session.state();
    let event = KeyEvent::new("Enter");
    let handled = state.plugins().iter().any(|plugin| {
        plugin.handle_key_down(&state, &event, &mut |tr| session.dispatch(tr).unwrap())
    });
    assert!(
        handled,
        "installed Enter binding must handle this table cell"
    );
}

#[test]
fn installed_table_enter_moves_same_column_and_preserves_existing_source_undo() {
    runtime(|| {
        let raw = "| head | right |\r\n| --- | --- |\r\n| one | two |\r\n| last |  |\r\n";
        let source = NativeView::fixture(raw);
        let snapshot = source.snapshot().unwrap();
        let start = snapshot.rope.to_string().find("right").unwrap();
        source
            .edit(
                snapshot.revision,
                snapshot.view_epoch,
                vec![(start..start + 5, "RIGHT".into())],
                ExternalSelections {
                    primary: 0,
                    ranges: vec![(start + 5, start + 5)],
                },
                ExternalEditKind::Other,
            )
            .unwrap();
        let edited = bytes(&source);
        let rich = source.sibling().unwrap();
        let gate = ModeGate::new(source.clone(), rich.clone());
        gate.to_rich();
        assert!(gate.rich_mode.get(), "{}", gate.status.get());
        select(&gate, 0, 0, 1);
        let before = body_audit(&source);
        let old = gate.session().state();
        let target = cell_position(old.doc(), 0, 1, 1).unwrap();
        assert_eq!(
            table_navigation::target(&old),
            EnterTarget::NextRow { position: target }
        );
        installed_enter(&gate);
        assert!(gate.status.get().is_empty(), "{}", gate.status.get());
        let state = gate.session().state();
        assert_eq!(state.doc(), old.doc());
        assert_eq!(
            (state.selection().anchor(), state.selection().head()),
            (target, target)
        );
        assert_ne!(state.selection().head(), old.selection().head());
        assert_eq!(
            body_audit(&source),
            before,
            "Enter must have zero body/token/dirty/history delta"
        );
        let projection =
            Projection::parse(state.schema(), source.snapshot().unwrap().rope.to_string()).unwrap();
        let source_at = projection.source_position(target).unwrap();
        assert_eq!(source_at, projection.source.find("two").unwrap());
        assert_eq!(
            rich.snapshot().unwrap().selections.ranges,
            vec![(source_at, source_at)]
        );
        gate.to_source();
        assert!(!gate.rich_mode.get());
        assert_eq!(
            source.snapshot().unwrap().selections.ranges,
            vec![(source_at, source_at)]
        );
        assert_eq!(body_audit(&source), before);
        gate.shell_history(true).unwrap();
        assert_eq!(
            bytes(&source),
            raw.as_bytes(),
            "Source Undo must still reach the earlier source edit"
        );
        gate.shell_history(false).unwrap();
        assert_eq!(bytes(&source), edited);
    });
}

#[test]
fn installed_table_enter_at_last_row_reports_boundary_without_splitting_or_history() {
    runtime(|| {
        let source =
            NativeView::fixture("outside\r\n\r\n| h | v |\r\n| --- | --- |\r\n| last |  |\r\n");
        let rich = source.sibling().unwrap();
        let gate = ModeGate::new(source.clone(), rich.clone());
        gate.to_rich();
        assert!(gate.rich_mode.get(), "{}", gate.status.get());
        select(&gate, 1, 1, 1);
        let before = body_audit(&source);
        let old = gate.session().state();
        assert_eq!(table_navigation::target(&old), EnterTarget::LastRow);
        let queried = Arc::new(AtomicUsize::new(0));
        let queried_callback = queried.clone();
        let command = table_navigation::enter(move || {
            queried_callback.fetch_add(1, Ordering::Relaxed);
        });
        assert!(command(&old, None));
        assert_eq!(
            queried.load(Ordering::Relaxed),
            0,
            "availability queries must not publish boundary feedback"
        );
        installed_enter(&gate);
        assert!(
            !gate.status.get().is_empty(),
            "the actual installed binding must explicitly report the last row"
        );
        let state = gate.session().state();
        assert_eq!(state.doc(), old.doc());
        assert_eq!(state.selection().to_json(), old.selection().to_json());
        assert_eq!(body_audit(&source), before);
        let projection =
            Projection::parse(state.schema(), source.snapshot().unwrap().rope.to_string()).unwrap();
        let source_at = projection
            .source_position(state.selection().head())
            .unwrap();
        assert_eq!(
            rich.snapshot().unwrap().selections.ranges,
            vec![(source_at, source_at)]
        );
        gate.to_source();
        assert!(!gate.rich_mode.get());
        assert_eq!(
            source.snapshot().unwrap().selections.ranges,
            vec![(source_at, source_at)]
        );
        assert_eq!(body_audit(&source), before);
        gate.to_rich();
        assert!(gate.rich_mode.get());
        let session = gate.session();
        let state = session.state();
        let mut tr = state.tr();
        tr.set_selection(TextSelection::create(state.doc(), 1, None));
        session.dispatch(tr).unwrap();
        let outside = session.state();
        assert_eq!(
            table_navigation::target(&outside),
            EnterTarget::OutsideTable
        );
        assert!(
            !command(&outside, None),
            "outside-table Enter must fall through to existing editor rules"
        );
        assert_eq!(queried.load(Ordering::Relaxed), 0);
        assert_eq!(body_audit(&source), before);
    });
}
