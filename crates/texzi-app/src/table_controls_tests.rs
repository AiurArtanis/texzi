//! Component fixtures use the actual ModeGate -> RichOwner -> NativeView ->
//! Store route, including source-side history. They do not claim GUI coverage.
use crate::{
    editor_owner::NativeView,
    rich_owner::ModeGate,
    rich_projection::{Projection, manager},
    table_controls::{META, TableAction, TableControls, action_transaction, cell_position},
};
use nsh_base::{
    editor::{ExternalDocument, ExternalEditKind, ExternalSelections},
    rich_editor::state::TextSelection,
};
use nsh_core::prelude::*;
use serde_json::json;
use std::rc::Rc;

fn runtime(f: impl FnOnce()) {
    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    f();
}
fn bytes(view: &NativeView) -> Vec<u8> {
    view.inspect(|s, id| s.snapshot(s.view(id).unwrap().doc).unwrap().body.to_utf8())
}
fn history(view: &NativeView) -> (usize, usize) {
    view.inspect(|s, id| s.history_depth(s.view(id).unwrap().doc).unwrap())
}
fn audit(view: &NativeView) -> String {
    view.inspect(|s, id| {
        let v = s.view(id).unwrap();
        let p = s.snapshot(v.doc).unwrap();
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            v,
            p.token,
            p.body.to_utf8(),
            p.body.origins(),
            p.body.eols(),
            s.history_depth(v.doc),
            s.dirty(v.doc)
        )
    })
}
fn setup(
    raw: &str,
    block: usize,
    row: usize,
    col: usize,
) -> (Rc<NativeView>, Rc<ModeGate>, Rc<TableControls>) {
    let source = NativeView::fixture(raw);
    let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
    gate.to_rich();
    assert!(gate.rich_mode.get(), "{}", gate.status.get());
    select(&gate, block, row, col);
    let controls = TableControls::new(gate.clone(), source.clone());
    (source, gate, controls)
}
fn select(gate: &ModeGate, block: usize, row: usize, col: usize) {
    let session = gate.session();
    let state = session.state();
    let at = cell_position(state.doc(), block, row, col).unwrap();
    let mut tr = state.tr();
    tr.set_selection(TextSelection::create(state.doc(), at, None));
    session.dispatch(tr).unwrap();
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
}
fn apply(controls: &TableControls, action: TableAction) {
    controls.begin().unwrap();
    controls
        .apply(action)
        .unwrap_or_else(|error| panic!("{action:?}: {error}"));
}

#[test]
fn table_actions_preserve_untouched_cell_spelling_padding_markers_and_mixed_eols() {
    runtime(|| {
        let header = r"| A\|B `c`   |  label  | &amp; |";
        let separator = "| :-------  |  -------: | :---------: |";
        let body = r"| x\*y `e`   |  plain  | &#38; |";
        let tail = "| tail  |  t  | z |";
        let fixture = |h: &str, s: &str, b: &str, t: &str| {
            format!("outside  \r\n\r\n{h}\r\n{s}\n{b}\n{t}\r\n\r\n<keep data-a='1'>raw</keep>\r\n")
        };
        let raw = fixture(header, separator, body, tail);
        let cases = [
            (
                TableAction::RowBefore,
                raw.replacen(body, &format!("|  |  |  |\r\n{body}"), 1),
            ),
            (
                TableAction::RowAfter,
                raw.replacen(tail, &format!("|  |  |  |\r\n{tail}"), 1),
            ),
            (
                TableAction::DeleteRow,
                raw.replacen(&format!("{body}\n"), "", 1),
            ),
            (
                TableAction::ColumnBefore,
                fixture(
                    r"| A\|B `c`   |  |  label  | &amp; |",
                    "| :-------  | --- |  -------: | :---------: |",
                    r"| x\*y `e`   |  |  plain  | &#38; |",
                    "| tail  |  |  t  | z |",
                ),
            ),
            (
                TableAction::DeleteColumn,
                fixture(
                    r"| A\|B `c`   | &amp; |",
                    "| :-------  | :---------: |",
                    r"| x\*y `e`   | &#38; |",
                    "| tail  | z |",
                ),
            ),
            (
                TableAction::AlignCenter,
                fixture(
                    header,
                    "| :-------  |  :-------: | :---------: |",
                    body,
                    tail,
                ),
            ),
            (TableAction::Header, fixture(body, separator, header, tail)),
        ];
        for (action, expected) in cases {
            let (source, gate, controls) = setup(&raw, 1, 1, 1);
            apply(&controls, action);
            assert_eq!(
                bytes(&source),
                expected.as_bytes(),
                "actual Owner bytes for {action:?}"
            );
            assert_eq!(
                history(&source),
                (1, 0),
                "single Store entry for {action:?}"
            );
            gate.to_source();
            gate.shell_history(true).unwrap();
            assert_eq!(
                bytes(&source),
                raw.as_bytes(),
                "source Undo restores exact bytes for {action:?}"
            );
            gate.shell_history(false).unwrap();
            assert_eq!(
                bytes(&source),
                expected.as_bytes(),
                "source Redo restores exact bytes for {action:?}"
            );
        }
    });
}

#[test]
fn table_rows_columns_and_headers_round_trip_through_store_source_history() {
    runtime(|| {
        let raw = "前段  \r\n\r\n| head | value |\r\n| :--- | ---: |\r\n| one | two |\r\n| three | four |\r\n\n<keep data-x='1'>raw</keep>\n";
        for action in [
            TableAction::RowBefore,
            TableAction::RowAfter,
            TableAction::DeleteRow,
            TableAction::ColumnBefore,
            TableAction::ColumnAfter,
            TableAction::DeleteColumn,
            TableAction::Header,
        ] {
            let (source, gate, controls) = setup(raw, 1, 1, 0);
            let before = gate.session().state().doc().clone();
            apply(&controls, action);
            let after = bytes(&source);
            assert_ne!(after, raw.as_bytes(), "{action:?}");
            assert!(
                after.starts_with(b"\xe5\x89\x8d\xe6\xae\xb5  \r\n\r\n"),
                "prefix {action:?}"
            );
            assert!(
                after.ends_with(b"\r\n\n<keep data-x='1'>raw</keep>\n"),
                "suffix {action:?}: {after:?}"
            );
            assert_eq!(history(&source), (1, 0), "{action:?}");
            let state = gate.session().state();
            let table = state.doc().child(1);
            assert_eq!(table.child(0).child(0).ty().name(), "tableHeader");
            match action {
                TableAction::RowBefore | TableAction::RowAfter => {
                    assert_eq!(table.child_count(), 4)
                }
                TableAction::DeleteRow => assert_eq!(table.child_count(), 2),
                TableAction::ColumnBefore | TableAction::ColumnAfter => {
                    assert_eq!(table.child(0).child_count(), 3)
                }
                TableAction::DeleteColumn => assert_eq!(table.child(0).child_count(), 1),
                TableAction::Header => assert_eq!(table.child(0).text_content(), "onetwo"),
                _ => unreachable!(),
            }
            // Switch to source and undo the same Store entry, then redo and
            // return to rich. No second rich body/history participates.
            gate.to_source();
            assert!(!gate.rich_mode.get());
            gate.shell_history(true).unwrap();
            assert_eq!(bytes(&source), raw.as_bytes(), "undo {action:?}");
            assert_eq!(history(&source), (0, 1));
            gate.shell_history(false).unwrap();
            assert_eq!(bytes(&source), after);
            gate.to_rich();
            assert!(gate.rich_mode.get());
            assert_ne!(gate.session().state().doc(), &before);
        }
    });
}

#[test]
fn table_alignment_changes_the_whole_column_and_remains_editable() {
    runtime(|| {
        for (action, expected) in [
            (TableAction::AlignLeft, json!("left")),
            (TableAction::AlignCenter, json!("center")),
            (TableAction::AlignRight, json!("right")),
            (TableAction::AlignDefault, json!(null)),
        ] {
            let raw = if action == TableAction::AlignDefault {
                "| h | v |\r\n| --- | :---: |\r\n| a | b |\r\n"
            } else {
                "| h | v |\r\n| --- | --- |\r\n| a | b |\r\n"
            };
            let (source, gate, controls) = setup(raw, 0, 1, 1);
            apply(&controls, action);
            let state = gate.session().state();
            let table = state.doc().child(0);
            for row in 0..table.child_count() {
                assert_eq!(
                    table.child(row).child(1).child(0).attrs()["textAlign"],
                    expected
                );
            }
            let mut tr = state.tr();
            tr.insert_text("中", None, None);
            gate.session().dispatch(tr).unwrap();
            assert!(gate.status.get().is_empty(), "{}", gate.status.get());
            assert!(source.snapshot().unwrap().rope.to_string().contains("中b"));
            gate.shell_history(true).unwrap();
            gate.shell_history(true).unwrap();
            assert_eq!(bytes(&source), raw.as_bytes());
        }
    });
}

#[test]
fn deleting_the_last_row_or_column_leaves_a_typeable_empty_paragraph() {
    runtime(|| {
        for raw in [
            "| h |\r\n| --- |",
            "before\r\n\r\n| h |\r\n| --- |\r\n\n<raw>x</raw>\n",
        ] {
            for action in [
                TableAction::DeleteRow,
                TableAction::DeleteColumn,
                TableAction::DeleteTable,
            ] {
                let block = usize::from(raw.starts_with("before"));
                let (source, gate, controls) = setup(raw, block, 0, 0);
                apply(&controls, action);
                let state = gate.session().state();
                assert_eq!(state.doc().child(block).ty().name(), "paragraph");
                assert_eq!(state.doc().child(block).content().size(), 0);
                let mut tr = state.tr();
                tr.insert_text("可继续输入", None, None);
                gate.session().dispatch(tr).unwrap();
                assert!(gate.status.get().is_empty(), "{}", gate.status.get());
                assert!(
                    source
                        .snapshot()
                        .unwrap()
                        .rope
                        .to_string()
                        .contains("可继续输入")
                );
                gate.to_source();
                gate.shell_history(true).unwrap();
                gate.shell_history(true).unwrap();
                assert_eq!(bytes(&source), raw.as_bytes());
            }
        }
    });
}

#[test]
fn table_cancel_ime_and_stale_revision_never_publish() {
    runtime(|| {
        let (source, gate, controls) =
            setup("| h | v |\r\n| --- | --- |\r\n| a | b |\r\n", 0, 1, 0);
        let before = audit(&source);
        controls.begin().unwrap();
        controls.cancel();
        assert!(controls.apply(TableAction::RowAfter).is_err());
        assert_eq!(audit(&source), before);
        source.set_composing(true);
        assert!(controls.begin().is_err());
        assert_eq!(audit(&source), before);
        source.set_composing(false);
        controls.begin().unwrap();
        source.set_composing(true);
        assert!(controls.apply(TableAction::ColumnAfter).is_err());
        assert_eq!(audit(&source), before);
        source.set_composing(false);
        let other = source.sibling_source().unwrap();
        let snapshot = other.snapshot().unwrap();
        other
            .edit(
                snapshot.revision,
                snapshot.view_epoch,
                vec![(2..3, "new".into())],
                ExternalSelections {
                    primary: 0,
                    ranges: vec![(5, 5)],
                },
                ExternalEditKind::Other,
            )
            .unwrap();
        let after_external = audit(&source);
        assert!(
            controls
                .apply(TableAction::RowAfter)
                .unwrap_err()
                .contains("过期")
        );
        assert_eq!(audit(&source), after_external);
    });
}

#[test]
fn table_selection_revision_invalidates_the_open_anchor_without_writing() {
    runtime(|| {
        let (source, gate, controls) = setup("| h | v |\n| --- | --- |\n| a | b |\n", 0, 1, 0);
        controls.begin().unwrap();
        let anchor = controls.anchor().unwrap();
        let state = gate.session().state();
        assert_eq!(anchor.revision, state.revision());
        assert_eq!(
            state.doc().node_at(anchor.table).unwrap().ty().name(),
            "table"
        );
        assert_eq!(
            state.doc().node_at(anchor.cell).unwrap().ty().name(),
            "tableCell"
        );
        select(&gate, 0, 1, 1);
        let before_action = audit(&source);
        assert!(controls.apply(TableAction::RowAfter).is_err());
        assert_eq!(audit(&source), before_action);
    });
}

#[test]
fn table_metadata_cannot_authorize_changed_text_or_another_block() {
    runtime(|| {
        let (source, gate, _) = setup("| h | v |\n| --- | --- |\n| a | b |\n\nkeep\n", 0, 1, 0);
        let before = audit(&source);
        let session = gate.session();
        let state = session.state();
        let mut tr = action_transaction(&state, 0, 1, 0, TableAction::RowAfter).unwrap();
        let at = cell_position(tr.doc(), 0, 1, 0).unwrap();
        tr.insert_text("FORGED", Some(at), Some(at));
        session.dispatch(tr).unwrap();
        assert!(!gate.status.get().is_empty());
        assert_eq!(audit(&source), before);
        gate.status.set(String::new());
        let mut tr = state.tr();
        tr.insert_text("FORGED", None, None);
        tr.set_meta(
            META,
            json!({"block":0,"row":1,"column":0,"action":"rowAfter"}),
        );
        session.dispatch(tr).unwrap();
        assert!(!gate.status.get().is_empty());
        assert_eq!(audit(&source), before);
    });
}

#[test]
fn empty_cell_mapping_handles_escaped_pipes_and_code_in_neighbor_cells() {
    runtime(|| {
        for raw in [
            "| h | v |\n| --- | --- |\n| a\\|b |  |\n",
            "| h | v |\n| --- | --- |\n| `a\\|b` |  |\n",
        ] {
            let projection = Projection::parse(manager().schema(), raw.into()).unwrap();
            let pos = cell_position(&projection.doc, 0, 1, 1).unwrap();
            let source_at = projection
                .source_position(pos)
                .expect("actual empty-cell source seam");
            assert_eq!(projection.rich_position(source_at), Some(pos));
            let (source, gate, controls) = setup(raw, 0, 1, 0);
            apply(&controls, TableAction::ColumnBefore);
            let state = gate.session().state();
            let mut tr = state.tr();
            tr.insert_text("空格可写", None, None);
            gate.session().dispatch(tr).unwrap();
            assert!(gate.status.get().is_empty(), "{}", gate.status.get());
            assert!(
                source
                    .snapshot()
                    .unwrap()
                    .rope
                    .to_string()
                    .contains("空格可写")
            );
        }
    });
}
