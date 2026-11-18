//! Executable PRD §8.2 fixtures through RichOwner -> NativeView -> Store.
//! Rejection never counts as direct-edit success; GUI evidence stays separate.
use crate::{
    editor_owner::NativeView,
    rich_owner::{ModeGate, RichOwner},
    rich_projection::Projection,
};
use nsh_base::{
    editor::{ExternalDocument, ExternalEditKind, ExternalSelections},
    rich_editor::{
        model::Node,
        state::{KeyEvent, TextSelection},
    },
};
use nsh_core::prelude::*;
use serde_json::Value;
use std::{panic::AssertUnwindSafe, rc::Rc};

const SPEC: &str = include_str!("../../../docs/t05/SYNTAX-MATRIX.json");

#[derive(Debug, PartialEq, Eq)]
struct Audit {
    bytes: Vec<u8>,
    revision: u64,
    token: texzi_document::StateToken,
    eols: Vec<texzi_document::Eol>,
    origins: String,
    history: (usize, usize),
    dirty: bool,
}
fn audit(view: &NativeView) -> Audit {
    view.inspect(|store, id| {
        let doc = store.view(id).unwrap().doc;
        let snapshot = store.snapshot(doc).unwrap();
        Audit {
            bytes: snapshot.body.to_utf8(),
            revision: snapshot.revision.0,
            token: snapshot.token,
            eols: snapshot.body.eols(),
            origins: format!("{:?}", snapshot.body.origins()),
            history: store.history_depth(doc).unwrap(),
            dirty: store.dirty(doc).unwrap(),
        }
    })
}
fn bytes(view: &NativeView) -> Vec<u8> {
    audit(view).bytes
}
fn runtime(f: impl FnOnce()) {
    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    f();
}
fn spec() -> Value {
    serde_json::from_str(SPEC).expect("checked-in syntax matrix JSON")
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("missing {key}: {value}"))
}
fn cases(spec: &Value) -> impl Iterator<Item = &Value> {
    spec["cases"].as_array().unwrap().iter()
}
fn all_cases(spec: &Value) -> impl Iterator<Item = &Value> {
    cases(spec).chain(spec["combinations"].as_array().unwrap().iter())
}
// Continue independent cases after a failure; keep the ID and exact assertion.
fn check_all<'a>(rows: impl Iterator<Item = &'a Value>, check: impl Fn(&Value)) {
    let mut failures = Vec::new();
    for case in rows {
        if let Err(error) = std::panic::catch_unwind(AssertUnwindSafe(|| check(case))) {
            let detail = error
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| error.downcast_ref::<&str>().map(|text| (*text).to_owned()))
                .unwrap_or_else(|| "non-string assertion panic".into());
            failures.push(format!("{}: {detail}", text(case, "id")));
        }
    }
    assert!(
        failures.is_empty(),
        "{} fixture failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
fn rich(source: &Rc<NativeView>) -> Rc<RichOwner> {
    RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap()
}
fn select(owner: &RichOwner, from: usize, to: usize) {
    let session = owner.session();
    let state = session.state();
    let mut tr = state.tr();
    tr.set_selection(TextSelection::create(state.doc(), from, Some(to)));
    session.dispatch(tr).unwrap();
    assert!(
        owner.status.get().is_empty(),
        "selection: {}",
        owner.status.get()
    );
    assert_eq!(session.state().doc(), state.doc());
    assert_eq!(session.state().selection().anchor(), from);
    assert_eq!(session.state().selection().head(), to);
}
fn history_key(owner: &RichOwner, redo: bool) {
    let session = owner.session();
    let state = session.state();
    let mut event = KeyEvent::new(if redo { "y" } else { "z" });
    event.mod_key = true;
    event.ctrl = true;
    let handled = state.plugins().iter().any(|plugin| {
        plugin.handle_key_down(&state, &event, &mut |tr| {
            session.dispatch(tr).unwrap();
        })
    });
    assert!(
        handled,
        "Store history keymap must handle Ctrl-{}",
        event.key
    );
    assert!(
        owner.status.get().is_empty(),
        "history: {}",
        owner.status.get()
    );
}
fn source_history(source: &NativeView, undo: bool) {
    let snapshot = source.snapshot().unwrap();
    source
        .history(snapshot.revision, snapshot.view_epoch, undo)
        .unwrap();
}
fn assert_edit_history(owner: &RichOwner, source: &NativeView, raw: &str, expected: &str) {
    assert_eq!(
        bytes(source),
        expected.as_bytes(),
        "entire source, including mixed EOL"
    );
    assert_eq!(
        audit(source).history,
        (1, 0),
        "one action = one Store undo item"
    );
    assert!(audit(source).dirty);
    history_key(owner, false);
    assert_eq!(
        bytes(source),
        raw.as_bytes(),
        "rich Undo restores all original bytes"
    );
    assert_eq!(audit(source).history, (0, 1));
    assert!(!audit(source).dirty);
    history_key(owner, true);
    assert_eq!(
        bytes(source),
        expected.as_bytes(),
        "rich Redo restores exact edited bytes"
    );
    assert_eq!(audit(source).history, (1, 0));
    // The source adapter observes the same Store history.
    source_history(source, true);
    assert_eq!(
        bytes(source),
        raw.as_bytes(),
        "source Undo sees the rich edit"
    );
    source_history(source, false);
    assert_eq!(
        bytes(source),
        expected.as_bytes(),
        "source Redo sees the rich edit"
    );
}
fn occurrence(haystack: &str, needle: &str, n: usize) -> usize {
    haystack
        .match_indices(needle)
        .nth(n)
        .unwrap_or_else(|| panic!("missing occurrence {n} of {needle:?}"))
        .0
}
fn leaf_edit(case: &Value, edit: &Value) {
    let raw = text(case, "raw");
    let source = NativeView::fixture(raw);
    let owner = rich(&source);
    let session = owner.session();
    let old = session.state();
    let canonical = source.snapshot().unwrap().rope.to_string();
    let before = text(edit, "before");
    let n = edit["occurrence"].as_u64().unwrap_or(0) as usize;
    let byte = occurrence(&canonical, before, n);
    let p = Projection::parse(old.schema(), canonical).unwrap();
    let from = p
        .rich_position(byte)
        .expect("visible leaf start has a source mapping");
    let to = p
        .rich_position(byte + before.len())
        .expect("visible leaf end has a source mapping");
    let initial = audit(&source);
    select(&owner, from, to);
    assert_eq!(
        audit(&source),
        initial,
        "selection is not a body/history/dirty edit"
    );
    let mut tr = session.state().tr();
    tr.insert_text(text(edit, "insert"), Some(from), Some(to));
    session.dispatch(tr).unwrap();
    assert!(
        owner.status.get().is_empty(),
        "direct edit rejected: {}",
        owner.status.get()
    );
    assert_ne!(
        session.state().doc(),
        old.doc(),
        "visible rich state must actually change"
    );
    let raw_at = occurrence(raw, before, n);
    let mut expected = raw.to_owned();
    expected.replace_range(raw_at..raw_at + before.len(), text(edit, "after"));
    assert_eq!(
        bytes(&source),
        expected.as_bytes(),
        "only the specified raw text range changes"
    );
    source.inspect(|store, id| {
        let snapshot = store.snapshot(store.view(id).unwrap().doc).unwrap();
        let imported = raw.strip_prefix('\u{feff}').unwrap_or(raw);
        let removed = raw_at - (raw.len() - imported.len());
        let origins: Vec<_> = snapshot
            .body
            .origins()
            .into_iter()
            .filter_map(|(_, origin)| origin)
            .collect();
        for original_byte in (0..removed).chain(removed + before.len()..imported.len()) {
            assert!(
                origins
                    .iter()
                    .any(|origin| origin.source.as_ref() == imported
                        && origin.raw.contains(&original_byte)),
                "untouched original byte {original_byte} lost its provenance"
            );
        }
    });
    assert_edit_history(&owner, &source, raw, &expected);
}

#[test]
fn syntax_matrix_has_all_16_prd_rows_and_separate_evidence_layers() {
    let spec = spec();
    let rows: Vec<_> = cases(&spec).collect();
    assert_eq!(rows.len(), 16);
    for (index, case) in rows.iter().enumerate() {
        assert_eq!(text(case, "id"), format!("MD{:02}", index + 1));
        let raw = text(case, "raw");
        assert!(raw.contains("\r\n") && raw.contains('\n') && raw.ends_with('\r'));
        assert!(raw.contains("<opaque-neighbor data-stay=\"yes\">untouched</opaque-neighbor>"));
        for layer in [
            "original_bytes",
            "formatted_display",
            "direct_edit",
            "source_reachable",
            "component",
            "gui",
            "manual",
        ] {
            assert!(
                case["coverage"][layer]["status"].is_string(),
                "{}: missing {layer}",
                text(case, "id")
            );
            assert!(case["coverage"][layer]["evidence"].is_array());
        }
    }
    assert_eq!(
        spec["rules"].as_array().unwrap().len(),
        10,
        "FR-MD-04 through FR-MD-13 remain in scope"
    );
}
fn roundtrip(case: &Value, preserve_redo: bool) {
    let raw = text(case, "raw");
    let source = NativeView::fixture(raw);
    assert_eq!(bytes(&source), raw.as_bytes());
    if preserve_redo {
        let snapshot = source.snapshot().unwrap();
        source
            .edit(
                snapshot.revision,
                snapshot.view_epoch,
                vec![(0..1, "A".into())],
                ExternalSelections {
                    primary: 0,
                    ranges: vec![(1, 1)],
                },
                ExternalEditKind::Other,
            )
            .unwrap();
        source_history(&source, true);
        assert_eq!(audit(&source).history, (0, 1));
    }
    let before = audit(&source);
    assert!(!before.dirty);
    let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
    for turn in 0..100 {
        gate.to_rich();
        assert!(gate.rich_mode.get(), "round {turn}: {}", gate.status.get());
        assert!(gate.status.get().is_empty());
        assert_eq!(audit(&source), before, "source -> rich round {turn}");
        assert!(
            gate.session().state().doc().child_count() > 0,
            "no blank document on switch"
        );
        gate.to_source();
        assert!(
            !gate.rich_mode.get(),
            "rich -> source round {turn}: {}",
            gate.status.get()
        );
        gate.source_mounted();
        assert_eq!(audit(&source), before, "rich -> source round {turn}");
    }
    if preserve_redo {
        assert_eq!(
            source.can_history(),
            (false, true),
            "switches cannot clear redo"
        );
        source_history(&source, false);
        assert_eq!(
            bytes(&source),
            raw.replacen("anchor", "Anchor", 1).as_bytes()
        );
    }
}
#[test]
fn syntax_matrix_100_real_modegate_roundtrips_are_zero_body_history_dirty_changes() {
    runtime(|| check_all(all_cases(&spec()), |case| roundtrip(case, false)));
}
#[test]
fn syntax_matrix_100_real_modegate_roundtrips_keep_existing_redo() {
    runtime(|| check_all(cases(&spec()), |case| roundtrip(case, true)));
}
fn count_nodes(doc: &Node, name: &str) -> usize {
    let mut count = 0;
    doc.descendants(&mut |node, _, _, _| {
        if node.ty().name() == name {
            count += 1;
        }
        true
    });
    count
}
#[test]
fn syntax_matrix_formatted_semantics_and_literal_blocks_are_visible_not_dropped() {
    runtime(|| {
        check_all(cases(&spec()), |case| {
            let source = NativeView::fixture(text(case, "raw"));
            let owner = rich(&source);
            let state = owner.session().state();
            let doc = state.doc();
            let display = &case["display"];
            for required in display["nodes"].as_array().unwrap() {
                let name = text(required, "name");
                assert!(
                    count_nodes(doc, name) >= required["min"].as_u64().unwrap() as usize,
                    "missing semantic node {name} in {}",
                    text(case, "id")
                );
            }
            for required in display["visible"].as_array().unwrap() {
                assert!(
                    doc.text_content().contains(required.as_str().unwrap()),
                    "missing visible content {required}"
                );
            }
            for required in display["marks"].as_array().unwrap() {
                let mut found = false;
                doc.descendants(&mut |node, _, _, _| {
                    found |= node
                        .marks()
                        .iter()
                        .any(|mark| mark.ty.name() == required.as_str().unwrap());
                    true
                });
                assert!(found, "missing formatted mark {required}");
            }
            if let Some(links) = display["links"].as_array() {
                for link in links {
                    let mut found = false;
                    doc.descendants(&mut |node, _, _, _| {
                        found |= node
                            .text()
                            .is_some_and(|visible| visible.contains(text(link, "text")))
                            && node.marks().iter().any(|mark| {
                                mark.ty.name() == "link"
                                    && mark.attrs.get("href").and_then(Value::as_str)
                                        == Some(text(link, "href"))
                            });
                        true
                    });
                    assert!(found, "link text and href must be formatted: {link}");
                }
            }
            if let Some(blocks) = display["literal_blocks"].as_array() {
                for literal in blocks {
                    let mut found = false;
                    doc.descendants(&mut |node, _, _, _| {
                        found |= node.ty().name() == "codeBlock"
                            && node.text_content() == literal.as_str().unwrap();
                        true
                    });
                    assert!(
                        found,
                        "literal source block must show all delimiters/content: {literal}"
                    );
                }
            }
            assert_eq!(bytes(&source), text(case, "raw").as_bytes());
        })
    });
}
#[test]
fn syntax_matrix_direct_visible_leaf_edits_preserve_unmodified_bytes_and_shared_history() {
    runtime(|| {
        check_all(all_cases(&spec()), |case| {
            for edit in case["direct_edits"].as_array().unwrap() {
                leaf_edit(case, edit);
            }
        })
    });
}
#[test]
fn syntax_matrix_source_entry_is_editable_and_undo_restores_every_original_byte() {
    runtime(|| {
        check_all(cases(&spec()), |case| {
            let raw = text(case, "raw");
            let source = NativeView::fixture(raw);
            let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
            gate.to_rich();
            assert!(gate.rich_mode.get(), "{}", gate.status.get());
            gate.to_source();
            gate.source_mounted();
            assert!(!gate.rich_mode.get());
            let snapshot = source.snapshot().unwrap();
            let canonical = snapshot.rope.to_string();
            let body = text(case, "body").replace("\r\n", "\n").replace('\r', "\n");
            let at = canonical
                .find(&body)
                .expect("original syntax remains source-reachable");
            source
                .edit(
                    snapshot.revision,
                    snapshot.view_epoch,
                    vec![(at..at, "SOURCE-EDIT ".into())],
                    ExternalSelections {
                        primary: 0,
                        ranges: vec![(at + 12, at + 12)],
                    },
                    ExternalEditKind::Other,
                )
                .unwrap();
            let raw_at = raw.find(text(case, "body")).unwrap();
            let mut expected = raw.to_owned();
            expected.insert_str(raw_at, "SOURCE-EDIT ");
            assert_eq!(bytes(&source), expected.as_bytes());
            assert_eq!(audit(&source).history, (1, 0));
            source_history(&source, true);
            assert_eq!(bytes(&source), raw.as_bytes());
            assert!(!audit(&source).dirty);
            source_history(&source, false);
            assert_eq!(bytes(&source), expected.as_bytes());
        })
    });
}
#[test]
fn syntax_matrix_task_checkbox_only_changes_its_own_marker() {
    runtime(|| {
        let spec = spec();
        let case = cases(&spec)
            .find(|case| text(case, "id") == "MD06")
            .unwrap();
        let raw = text(case, "raw");
        let source = NativeView::fixture(raw);
        let owner = rich(&source);
        let session = owner.session();
        let state = session.state();
        let mut at = None;
        state.doc().descendants(&mut |node, pos, _, _| {
            if at.is_none() && node.ty().name() == "taskItem" {
                at = Some(pos);
            }
            true
        });
        let mut tr = state.tr();
        tr.set_node_attribute(
            at.expect("native task node"),
            "checked",
            serde_json::json!(true),
        )
        .unwrap();
        session.dispatch(tr).unwrap();
        assert!(owner.status.get().is_empty(), "{}", owner.status.get());
        assert_edit_history(&owner, &source, raw, &raw.replacen("[ ]", "[x]", 1));
    });
}
#[test]
fn syntax_matrix_image_address_and_alt_are_local_node_edits() {
    runtime(|| {
        check_all(
            cases(&spec()).filter(|case| case["attribute_edits"].is_array()),
            |case| {
                for edit in case["attribute_edits"].as_array().unwrap() {
                    let raw = text(case, "raw");
                    let source = NativeView::fixture(raw);
                    let owner = rich(&source);
                    let session = owner.session();
                    let state = session.state();
                    let mut at = None;
                    state.doc().descendants(&mut |node, pos, _, _| {
                        if at.is_none() && node.ty().name() == text(edit, "node") {
                            at = Some(pos);
                        }
                        true
                    });
                    let mut tr = state.tr();
                    tr.set_node_attribute(
                        at.expect("image node"),
                        text(edit, "attribute"),
                        edit["value"].clone(),
                    )
                    .unwrap();
                    session.dispatch(tr).unwrap();
                    assert!(
                        owner.status.get().is_empty(),
                        "image attribute rejected: {}",
                        owner.status.get()
                    );
                    let expected = raw.replacen(text(edit, "before"), text(edit, "after"), 1);
                    assert_edit_history(&owner, &source, raw, &expected);
                }
            },
        )
    });
}
#[test]
fn syntax_matrix_inline_code_backtick_only_lengthens_its_own_fence() {
    runtime(|| {
        let tick = '\u{60}';
        let raw = format!(
            "anchor\r\n\r\nbefore {tick}apple{tick} after\n\n<opaque-neighbor>keep</opaque-neighbor>\r\n"
        );
        let source = NativeView::fixture(&raw);
        let owner = rich(&source);
        let session = owner.session();
        let canonical = source.snapshot().unwrap().rope.to_string();
        let p = Projection::parse(session.state().schema(), canonical.clone()).unwrap();
        let at = p
            .rich_position(canonical.find("apple").unwrap() + 2)
            .unwrap();
        select(&owner, at, at);
        let mut tr = session.state().tr();
        tr.insert_text(&tick.to_string(), Some(at), Some(at));
        session.dispatch(tr).unwrap();
        assert!(
            owner.status.get().is_empty(),
            "inline code input rejected: {}",
            owner.status.get()
        );
        let expected = raw.replacen(
            &format!("{tick}apple{tick}"),
            &format!("{tick}{tick}ap{tick}ple{tick}{tick}"),
            1,
        );
        assert_edit_history(&owner, &source, &raw, &expected);
    });
}
