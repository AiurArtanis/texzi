//! Component fixtures use the actual ModeGate -> RichOwner -> NativeView ->
//! Store route, including source-side history. They do not claim GUI coverage.
use crate::{
    editor_owner::NativeView,
    rich_owner::ModeGate,
    rich_projection::{Projection, manager},
    table_controls::{
        META, TableAction, TableControls, action_transaction, active_header_cell, cell_position,
    },
};
use nsh_base::{
    editor::{ExternalDocument, ExternalEditKind, ExternalSelections},
    rich_editor::{
        formats::{DocFormat, MarkdownFormat},
        state::TextSelection,
        tables::{CellSelection, selected_rect},
    },
};
use nsh_core::prelude::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, rc::Rc, time::SystemTime};

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
    setup_native(NativeView::fixture(raw), block, row, col)
}

fn setup_native(
    source: Rc<NativeView>,
    block: usize,
    row: usize,
    col: usize,
) -> (Rc<NativeView>, Rc<ModeGate>, Rc<TableControls>) {
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
fn table_cancel_header_transaction_changes_only_the_current_rows_header_cells() {
    runtime(|| {
        let (_, gate, _) = setup(
            "| Head | Other |\n| :--- | ---: |\n| body | value |\n",
            0,
            0,
            0,
        );
        let state = gate.session().state();
        let first = cell_position(state.doc(), 0, 0, 0).unwrap() - 2;
        let second = cell_position(state.doc(), 0, 0, 1).unwrap() - 2;
        let mut mixed = state.tr();
        let mut attrs = state.doc().child(0).child(0).child(0).attrs().clone();
        attrs.insert("colwidth".into(), json!([172]));
        mixed
            .set_node_markup(first, None, Some(attrs), None)
            .unwrap();
        let same_attrs = state.doc().child(0).child(0).child(1).attrs().clone();
        mixed
            .set_node_markup(
                second,
                state.schema().node_type("tableCell"),
                Some(same_attrs),
                None,
            )
            .unwrap();
        mixed.set_selection(TextSelection::create(
            mixed.doc(),
            first + 3,
            Some(first + 4),
        ));
        let state = state.apply(mixed).unwrap();
        let old = state.doc().child(0);
        let action = action_transaction(&state, 0, 0, 0, TableAction::CancelHeader).unwrap();
        assert_eq!(action.steps.len(), 1);
        let result = action.doc().child(0);
        let old_cell = old.child(0).child(0);
        let cell = result.child(0).child(0);
        assert_eq!(cell.ty().name(), "tableCell");
        assert_eq!(cell.attrs(), old_cell.attrs());
        assert_eq!(cell.content(), old_cell.content());
        assert_eq!(result.child(0).child(1), old.child(0).child(1));
        assert_eq!(result.child(1), old.child(1));
        let (anchor, head) = (state.selection().anchor(), state.selection().head());
        let applied = state.apply(action).unwrap();
        assert_eq!(
            (applied.selection().anchor(), applied.selection().head()),
            (anchor, head)
        );
        assert!(action_transaction(&applied, 0, 0, 0, TableAction::CancelHeader).is_err());
        assert!(action_transaction(&state, 0, 1, 0, TableAction::CancelHeader).is_err());
    });
}

#[test]
fn table_cancel_header_source_representations_are_explicit() {
    let gfm = "| h | v |\n| --- | --- |\n| a | b |\n";
    let marker = format!("<!-- texzi:table-no-header -->\n{gfm}");
    let ast = markdown::to_mdast(&marker, &markdown::ParseOptions::gfm()).unwrap();
    assert!(matches!(
        ast.children().unwrap().as_slice(),
        [
            markdown::mdast::Node::Html(_),
            markdown::mdast::Node::Table(_)
        ]
    ));
    let manager = manager();
    let schema = manager.schema();
    let portable = Projection::parse(schema, gfm.to_owned()).unwrap();
    assert_eq!(
        portable.doc.child(0).child(0).child(0).ty().name(),
        "tableHeader"
    );
    let html = "<table>\n<tr><td>h</td><td>v</td></tr>\n<tr><td>a</td><td>b</td></tr>\n</table>\n";
    let opaque = Projection::parse(schema, html.to_owned()).unwrap();
    assert!(opaque.blocks[0].opaque);
    assert_eq!(opaque.source, html);
    let format = MarkdownFormat::default();
    let ordinary = format.parse(schema, html).unwrap();
    assert_eq!(ordinary.child(0).child(0).child(0).ty().name(), "tableCell");
    let serialized = format.serialize(&ordinary).unwrap();
    assert!(serialized.contains("<td>") && !serialized.contains("<th>"));
    assert_eq!(format.parse(schema, &serialized).unwrap(), ordinary);
}

const NO_HEADER_MARKER: &str = "<!-- texzi:table-no-header -->";

fn without_marker_line(marked: &str) -> String {
    let start = marked.find(NO_HEADER_MARKER).unwrap();
    let end = start + marked[start..].find('\n').unwrap() + 1;
    assert!(matches!(
        &marked[start + NO_HEADER_MARKER.len()..end],
        "\n" | "\r\n"
    ));
    assert_eq!(marked.matches(NO_HEADER_MARKER).count(), 1);
    format!("{}{}", &marked[..start], &marked[end..])
}

fn assert_headerless(gate: &ModeGate, block: usize) {
    let state = gate.session().state();
    let table = state.doc().child(block);
    assert_eq!(table.ty().name(), "table");
    for row in 0..table.child_count() {
        let row = table.child(row);
        for cell in 0..row.child_count() {
            assert_eq!(row.child(cell).ty().name(), "tableCell");
        }
    }
}

#[test]
fn table_cancel_header_marker_is_the_only_byte_change_and_survives_reopen_and_undo() {
    runtime(|| {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("texzi-table-marker-{}-{nonce}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let worker = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let table = "| Hé\u{1f600}\\|ad `q` | &amp; |  \r\n| :------ | ---: |\n| *one* | &#38; |\r\n| tail | z |";
        for (name, block, before, after) in [
            ("first", 0, "", "\r\n\n收尾一段\n"),
            (
                "middle",
                1,
                "# 前言\r\n\r\n",
                "\n\n<keep data-x='1'>raw</keep>\n",
            ),
            ("final", 1, "内容段  \n\n", ""),
        ] {
            let raw = format!("{before}{table}{after}");
            let path = dir.join(format!("{name}.md"));
            fs::write(&path, &raw).unwrap();
            let (source, gate, controls) =
                setup_native(NativeView::open_file(&path).unwrap(), block, 0, 1);
            let initial = gate.session().state();
            let selected_head = initial.selection().head();
            let original_table = initial.doc().child(block).clone();
            apply(&controls, TableAction::CancelHeader);
            assert_headerless(&gate, block);
            let changed = gate.session().state();
            assert_eq!(changed.selection().head(), selected_head);
            let result = changed.doc().child(block);
            assert_eq!(result.child_count(), original_table.child_count());
            assert_eq!(result.child(1), original_table.child(1));
            for col in 0..result.child(0).child_count() {
                assert_eq!(
                    result.child(0).child(col).content(),
                    original_table.child(0).child(col).content()
                );
                assert_eq!(
                    result.child(0).child(col).attrs(),
                    original_table.child(0).child(col).attrs()
                );
            }
            let marked = String::from_utf8(bytes(&source)).unwrap();
            assert_eq!(marked.find(NO_HEADER_MARKER), Some(before.len()));
            assert_eq!(without_marker_line(&marked), raw);
            assert_eq!(history(&source), (1, 0));

            // Verify the operation only reaches disk on Save. Reopen the
            // written file with a new document owner and Markdown projection.
            assert_eq!(fs::read(&path).unwrap(), raw.as_bytes());
            worker
                .block_on(source.save_file(path.clone(), true, false))
                .unwrap();
            let written = fs::read(&path).unwrap();
            assert_eq!(written, marked.as_bytes());
            let marked_digest = Sha256::digest(&written);
            let (reopened_source, reopened, _) =
                setup_native(NativeView::open_file(&path).unwrap(), block, 0, 1);
            assert_headerless(&reopened, block);
            assert_eq!(bytes(&reopened_source), marked.as_bytes());
            assert_eq!(
                reopened.session().state().doc().to_json(),
                changed.doc().to_json()
            );

            gate.to_source();
            gate.shell_history(true).unwrap();
            assert_eq!(bytes(&source), raw.as_bytes());
            assert_eq!(history(&source), (0, 1));
            worker
                .block_on(source.save_file(path.clone(), true, false))
                .unwrap();
            let undone = fs::read(&path).unwrap();
            assert_eq!(undone, raw.as_bytes());
            eprintln!(
                "table marker {name}: saved {marked_digest:x}, saved one-step undo {:x}",
                Sha256::digest(&undone)
            );
            gate.to_rich();
            assert_eq!(
                gate.session().state().doc().to_json(),
                initial.doc().to_json()
            );
            gate.to_source();
            gate.shell_history(false).unwrap();
            assert_eq!(bytes(&source), marked.as_bytes());
            gate.to_rich();
            assert_headerless(&gate, block);
            apply(&controls, TableAction::Header);
            assert_eq!(bytes(&source), raw.as_bytes());
            gate.shell_history(true).unwrap();
            assert_headerless(&gate, block);
            assert_eq!(bytes(&source), marked.as_bytes());
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir(dir).unwrap();
    });
}

#[test]
fn table_cancel_header_marker_scopes_only_the_exact_adjacent_top_level_table() {
    let pipe = "| H | V |\n| --- | --- |\n| A | B |\n";
    let manager = manager();
    let schema = manager.schema();
    for text in [
        format!("{NO_HEADER_MARKER}\n\n{pipe}"),
        format!("{NO_HEADER_MARKER} \n{pipe}"),
        format!("  {NO_HEADER_MARKER}\n{pipe}"),
        format!("{NO_HEADER_MARKER}\n普通正文\n\n{pipe}"),
        format!("> {NO_HEADER_MARKER}\n> | H | V |\n> | --- | --- |\n> | A | B |\n\n{pipe}"),
        format!("```md\n{NO_HEADER_MARKER}\n{pipe}```\n\n{pipe}"),
    ] {
        let projected = Projection::parse(schema, text.clone()).unwrap();
        let table = projected
            .blocks
            .iter()
            .find(|block| block.node.ty().name() == "table")
            .unwrap();
        assert_eq!(table.node.child(0).child(0).ty().name(), "tableHeader");
        assert!(!projected.source[table.source.clone()].contains(NO_HEADER_MARKER));
        assert_eq!(projected.source, text);
    }
    let adjacent = format!("{pipe}\n{NO_HEADER_MARKER}\r\n{pipe}\n{pipe}");
    let projected = Projection::parse(schema, adjacent.clone()).unwrap();
    let tables: Vec<_> = projected
        .blocks
        .iter()
        .filter(|block| block.node.ty().name() == "table")
        .collect();
    assert_eq!(tables.len(), 3);
    assert_eq!(tables[0].node.child(0).child(0).ty().name(), "tableHeader");
    assert_eq!(tables[1].node.child(0).child(0).ty().name(), "tableCell");
    assert_eq!(tables[2].node.child(0).child(0).ty().name(), "tableHeader");
    assert!(adjacent[tables[1].source.clone()].starts_with(NO_HEADER_MARKER));
}

#[test]
fn table_cancel_header_does_not_confuse_cell_text_with_the_marker() {
    runtime(|| {
        let raw = "| table-no-header | value |\r\n| --- | --- |\n| A | B |";
        let (source, gate, controls) = setup(raw, 0, 0, 0);
        apply(&controls, TableAction::CancelHeader);
        let session = gate.session();
        let state = session.state();
        let pos = cell_position(state.doc(), 0, 0, 0).unwrap();
        let mut tr = state.tr();
        tr.insert_text("!", Some(pos), Some(pos));
        session.dispatch(tr).unwrap();
        assert!(gate.status.get().is_empty(), "{}", gate.status.get());
        assert_headerless(&gate, 0);
        let after = String::from_utf8(bytes(&source)).unwrap();
        assert_eq!(
            without_marker_line(&after),
            raw.replace("| table-no-header", "| !table-no-header")
        );
        gate.shell_history(true).unwrap();
        assert_eq!(
            without_marker_line(&String::from_utf8(bytes(&source)).unwrap()),
            raw
        );
    });
}

#[test]
fn table_cancel_header_structural_actions_keep_or_remove_the_owned_marker() {
    runtime(|| {
        let raw = "| Head | Value |\r\n| :--- | ---: |\n| One | Two |\r\n| Third | Fourth |";
        for (row, column, action) in [
            (0, 0, TableAction::RowBefore),
            (0, 1, TableAction::RowAfter),
            (0, 0, TableAction::DeleteRow),
            (0, 1, TableAction::ColumnBefore),
            (0, 0, TableAction::ColumnAfter),
            (0, 1, TableAction::DeleteColumn),
            (0, 1, TableAction::AlignCenter),
            (1, 0, TableAction::Header),
            (0, 1, TableAction::DeleteTable),
        ] {
            let (source, gate, controls) = setup(raw, 0, 0, 0);
            apply(&controls, TableAction::CancelHeader);
            let annotated = bytes(&source);
            select(&gate, 0, row, column);
            apply(&controls, action);
            let changed = String::from_utf8(bytes(&source)).unwrap();
            match action {
                TableAction::DeleteTable => {
                    assert!(!changed.contains(NO_HEADER_MARKER));
                    assert!(changed.trim().is_empty());
                }
                TableAction::Header => {
                    assert!(!changed.contains(NO_HEADER_MARKER));
                    assert_eq!(
                        gate.session()
                            .state()
                            .doc()
                            .child(0)
                            .child(0)
                            .text_content(),
                        "OneTwo"
                    );
                }
                _ => {
                    assert!(changed.starts_with(NO_HEADER_MARKER));
                    assert_headerless(&gate, 0);
                }
            }
            assert_eq!(history(&source), (2, 0), "{action:?}");
            gate.shell_history(true).unwrap();
            assert_eq!(bytes(&source), annotated, "{action:?}");
            assert_headerless(&gate, 0);
        }
    });
}

fn header_tool(controls: &TableControls) -> (TableAction, bool) {
    let buttons = controls.buttons();
    assert_eq!(buttons.len(), 12);
    let mut header = buttons
        .into_iter()
        .filter(|button| matches!(button.2, TableAction::Header | TableAction::CancelHeader));
    let (_, _, action, enabled) = header.next().unwrap();
    assert!(header.next().is_none());
    (action, enabled)
}

#[test]
fn table_header_slot_follows_the_active_cell_in_either_selection_direction() {
    runtime(|| {
        let raw = "| Head | Value |\r\n| :--- | ---: |\n| One | Two |\r\n| Third | Fourth |";
        let (source, gate, controls) = setup(raw, 0, 0, 0);
        let session = gate.session();
        let state = session.state();
        let first = cell_position(state.doc(), 0, 0, 0).unwrap() - 2;
        let second = cell_position(state.doc(), 0, 1, 1).unwrap() - 2;
        // Exercise the NSH-native selection model directly. The source
        // bridge independently guards selections it cannot map to bytes.
        for (anchor, head, row, col) in [(first, second, 1, 1), (second, first, 0, 0)] {
            let mut selection = state.tr();
            selection
                .set_selection(CellSelection::create(state.doc(), anchor, Some(head)).unwrap());
            let ranged = state.apply(selection).unwrap();
            assert!(ranged.selection().as_custom::<CellSelection>().is_some());
            let table = selected_rect(&ranged).unwrap();
            let active = active_header_cell(&ranged, &table).unwrap();
            assert_eq!((active.top, active.left), (row, col));
        }
        let before_range = audit(&source);
        let mut ranged = state.tr();
        ranged.set_selection(CellSelection::create(state.doc(), first, Some(second)).unwrap());
        session.dispatch(ranged).unwrap();
        if session
            .state()
            .selection()
            .as_custom::<CellSelection>()
            .is_some()
        {
            controls.begin().unwrap();
            assert_eq!(header_tool(&controls), (TableAction::Header, true));
            controls.cancel();
        } else {
            let status = gate.status.get();
            assert!(!status.is_empty());
            assert_eq!(audit(&source), before_range);
            eprintln!(
                "table header selection: Source bridge rejected an unmapped rectangle: {status}"
            );
        }
        select(&gate, 0, 1, 1);
        controls.begin().unwrap();
        assert_eq!(header_tool(&controls), (TableAction::Header, true));
        let active_body = cell_position(session.state().doc(), 0, 1, 1).unwrap() - 2;
        assert_eq!(controls.anchor().unwrap().cell, active_body);
        let before = audit(&source);
        assert!(controls.apply(TableAction::CancelHeader).is_err());
        assert_eq!(audit(&source), before);
        controls.cancel();

        select(&gate, 0, 0, 0);
        controls.begin().unwrap();
        assert_eq!(header_tool(&controls), (TableAction::CancelHeader, true));
        let active_header = cell_position(session.state().doc(), 0, 0, 0).unwrap() - 2;
        assert_eq!(controls.anchor().unwrap().cell, active_header);
        source.set_composing(true);
        let during_ime = audit(&source);
        assert!(controls.apply(TableAction::CancelHeader).is_err());
        assert_eq!(audit(&source), during_ime);
        source.set_composing(false);
        controls.cancel();
    });
}

#[test]
fn table_header_slot_guards_short_rows_and_can_cancel_a_promoted_header() {
    runtime(|| {
        let short = "| Header | Value |\n| --- | --- |\n| Only one |\n";
        let (source, _, controls) = setup(short, 0, 0, 0);
        controls.begin().unwrap();
        assert_eq!(header_tool(&controls), (TableAction::CancelHeader, false));
        let before = audit(&source);
        assert!(controls.apply(TableAction::CancelHeader).is_err());
        assert_eq!(audit(&source), before);

        let raw = "| Head | Value |\r\n| :--- | ---: |\n| One | Two |\r\n| Third | Fourth |";
        let (source, gate, controls) = setup(raw, 0, 1, 1);
        controls.begin().unwrap();
        assert_eq!(header_tool(&controls), (TableAction::Header, true));
        controls.apply(TableAction::Header).unwrap();
        let promoted = String::from_utf8(bytes(&source)).unwrap();
        assert!(promoted.starts_with("| One | Two |\r\n| :--- | ---: |\n"));

        controls.begin().unwrap();
        assert_eq!(header_tool(&controls), (TableAction::CancelHeader, true));
        let caret = gate.session().state().selection().head();
        controls.apply(TableAction::CancelHeader).unwrap();
        assert_eq!(gate.session().state().selection().head(), caret);
        assert_headerless(&gate, 0);
        assert_eq!(
            without_marker_line(&String::from_utf8(bytes(&source)).unwrap()),
            promoted
        );
        assert_eq!(history(&source), (2, 0));
        gate.shell_history(true).unwrap();
        assert_eq!(bytes(&source), promoted.as_bytes());
        assert_eq!(
            gate.session()
                .state()
                .doc()
                .child(0)
                .child(0)
                .child(1)
                .ty()
                .name(),
            "tableHeader"
        );
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
