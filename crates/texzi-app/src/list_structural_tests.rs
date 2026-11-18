//! Real installed list keymaps through ModeGate, sibling NativeViews and Store.
//! These are component tests; application-window and manual evidence is separate.
use crate::{editor_owner::NativeView, rich_owner::ModeGate, rich_projection::Projection};
use nsh_base::{
    editor::ExternalDocument,
    rich_editor::state::{KeyEvent, TextSelection},
};
use nsh_core::prelude::*;
use std::rc::Rc;

#[derive(Debug, PartialEq, Eq)]
struct Identity {
    bytes: Vec<u8>,
    token: texzi_document::StateToken,
    eols: Vec<texzi_document::Eol>,
    origins: String,
}
fn identity(source: &NativeView) -> Identity {
    source.inspect(|store, id| {
        let s = store.snapshot(store.view(id).unwrap().doc).unwrap();
        Identity {
            bytes: s.body.to_utf8(),
            token: s.token,
            eols: s.body.eols(),
            origins: format!("{:?}", s.body.origins()),
        }
    })
}
fn history(source: &NativeView) -> (usize, usize) {
    source.inspect(|s, id| s.history_depth(s.view(id).unwrap().doc).unwrap())
}
fn runtime(f: impl FnOnce()) {
    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    f();
}
fn fixture(raw: &str) -> (Rc<NativeView>, Rc<ModeGate>) {
    let source = NativeView::fixture(raw);
    let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
    gate.to_rich();
    assert!(gate.rich_mode.get(), "{}", gate.status.get());
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
    (source, gate)
}
fn select_byte(source: &NativeView, gate: &ModeGate, at: usize) {
    let before = identity(source);
    let state = gate.session().state();
    let p = Projection::parse(state.schema(), source.snapshot().unwrap().rope.to_string()).unwrap();
    let pos = p.rich_position(at).expect("exact AST-bound caret");
    let mut tr = state.tr();
    tr.set_selection(TextSelection::create(state.doc(), pos, None));
    gate.session().dispatch(tr).unwrap();
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
    assert_eq!(gate.session().state().selection().anchor(), pos);
    assert_eq!(
        identity(source),
        before,
        "selection must not edit the Store body"
    );
}
fn key(gate: &ModeGate, name: &str, shift: bool) {
    let session = gate.session();
    let state = session.state();
    let mut event = KeyEvent::new(name);
    event.shift = shift;
    let mut attempted = Vec::new();
    assert!(
        state.plugins().iter().any(|plugin| {
            plugin.handle_key_down(&state, &event, &mut |tr| {
                attempted.push(serde_json::json!({
                    "steps": tr.steps.iter().map(|step| step.to_json()).collect::<Vec<_>>(),
                    "next": tr.doc().to_json(),
                }));
                session.dispatch(tr).unwrap();
            })
        }),
        "installed keymap did not handle {name} (shift={shift})"
    );
    assert!(
        gate.status.get().is_empty(),
        "{name}: {}; before={}; attempted={attempted:?}",
        gate.status.get(),
        state.doc().to_json()
    );
    assert_ne!(
        session.state().doc(),
        state.doc(),
        "a rejection is not a successful structural edit"
    );
}
fn source_roundtrip(source: &NativeView, gate: &ModeGate, original: &Identity, expected: &str) {
    let edited = identity(source);
    assert_eq!(edited.bytes, expected.as_bytes());
    assert_ne!(edited.token, original.token);
    assert_eq!(history(source), (1, 0));
    gate.to_source();
    assert!(!gate.rich_mode.get(), "{}", gate.status.get());
    gate.source_mounted();
    gate.shell_history(true).unwrap();
    assert_eq!(
        &identity(source),
        original,
        "Source Undo restores bytes/token/EOL origins"
    );
    assert_eq!(history(source), (0, 1));
    gate.shell_history(false).unwrap();
    assert_eq!(
        identity(source),
        edited,
        "Source Redo restores the exact edited identity"
    );
    assert_eq!(history(source), (1, 0));
    gate.to_rich();
    assert!(gate.rich_mode.get(), "{}", gate.status.get());
    let state = gate.session().state();
    assert_eq!(
        Projection::parse(state.schema(), source.snapshot().unwrap().rope.to_string())
            .unwrap()
            .doc,
        *state.doc()
    );
}

#[test]
fn list_structural_nested_ordered_enter_preserves_starts_delimiter_and_mixed_eols() {
    runtime(|| {
        // runtime06: `7) root\n   4) alpha甲\n   5) spare` is one
        // continuing paragraph, not the intended nested ordered list.
        // A non-1 ordered marker cannot interrupt the parent's paragraph.
        // The blank line makes this an actual nested list starting at four.
        let raw = "7) root\r\n\r\n   4) alpha甲\n   5) spare\r\n\r\n<custom>KEEP</custom>\n";
        let expected = raw.replacen("alpha", "alpha\r\n   5) ", 1);
        let (source, gate) = fixture(raw);
        let original = identity(&source);
        let initial = gate.session().state();
        let initial_nested = initial.doc().child(0).child(0).child(1);
        assert_eq!(initial_nested.ty().name(), "orderedList");
        assert_eq!(
            initial_nested.attrs().get("start"),
            Some(&serde_json::json!(4))
        );
        assert_eq!(initial_nested.child_count(), 2);
        let canonical = source.snapshot().unwrap().rope.to_string();
        select_byte(&source, &gate, canonical.find("alpha").unwrap() + 5);
        key(&gate, "Enter", false);
        let state = gate.session().state();
        let list = state.doc().child(0);
        assert_eq!(list.ty().name(), "orderedList");
        assert_eq!(list.attrs().get("start"), Some(&serde_json::json!(7)));
        let nested = list.child(0).child(1);
        assert_eq!(nested.attrs().get("start"), Some(&serde_json::json!(4)));
        assert_eq!(nested.child_count(), 3);
        source_roundtrip(&source, &gate, &original, &expected);
    });
}

#[test]
fn list_structural_task_enter_keeps_original_checked_spelling_and_resets_new_item() {
    runtime(|| {
        let raw = "* [X] alpha甲\r\n* [ ] spare\n\n<custom>KEEP</custom>\r\n";
        let expected = raw.replacen("alpha", "alpha\r\n* [ ] ", 1);
        let (source, gate) = fixture(raw);
        let original = identity(&source);
        select_byte(&source, &gate, "* [X] alpha".len());
        key(&gate, "Enter", false);
        let state = gate.session().state();
        let list = state.doc().child(0);
        assert_eq!(list.ty().name(), "taskList");
        assert_eq!(list.child_count(), 3);
        assert_eq!(
            list.child(0).attrs().get("checked"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            list.child(1).attrs().get("checked"),
            Some(&serde_json::json!(false))
        );
        source_roundtrip(&source, &gate, &original, &expected);
    });
}

#[test]
fn list_structural_bullet_tab_and_shift_tab_keep_marker_body_and_opaque_neighbor() {
    runtime(|| {
        for (raw, expected, shift) in [
            (
                "+ first\r\n+ second甲\n\n<custom>KEEP</custom>\r\n",
                "+ first\r\n  + second甲\n\n<custom>KEEP</custom>\r\n",
                false,
            ),
            (
                "+ first\r\n  + second甲\n\n<custom>KEEP</custom>\r\n",
                "+ first\r\n+ second甲\n\n<custom>KEEP</custom>\r\n",
                true,
            ),
        ] {
            let (source, gate) = fixture(raw);
            let original = identity(&source);
            let canonical = source.snapshot().unwrap().rope.to_string();
            select_byte(&source, &gate, canonical.find("second").unwrap() + 6);
            key(&gate, "Tab", shift);
            source_roundtrip(&source, &gate, &original, expected);
        }
    });
}

#[test]
fn list_structural_empty_item_enter_exits_top_level_and_lifts_nested_item() {
    runtime(|| {
        // runtime06's `+ first\n  + ` has an empty marker interrupting
        // a paragraph, so it is not a nested empty item. A preceding child
        // establishes the nested list before its final empty item.
        for (raw, caret, expected) in [
            ("+ first\r\n+ \n", "+ first\n+ ".len(), "+ first\r\n\n"),
            (
                "+ first\r\n  + child\n  + \n",
                "+ first\n  + child\n  + ".len(),
                "+ first\r\n  + child\n+ \n",
            ),
            ("7) first\r\n8) \n", "7) first\n8) ".len(), "7) first\r\n\n"),
        ] {
            eprintln!("empty-list Enter fixture: {raw:?}");
            let (source, gate) = fixture(raw);
            let original = identity(&source);
            let initial = gate.session().state();
            let list = if raw.contains("child") {
                let nested = initial.doc().child(0).child(0).child(1);
                assert_eq!(nested.ty().name(), "bulletList");
                nested
            } else {
                initial.doc().child(0)
            };
            assert_eq!(list.child_count(), 2);
            assert_eq!(list.child(1).child(0).content().size(), 0);
            select_byte(&source, &gate, caret);
            key(&gate, "Enter", false);
            source_roundtrip(&source, &gate, &original, expected);
        }
    });
}

#[test]
fn list_structural_task_tab_is_not_advertised_as_a_supported_nested_binding() {
    runtime(|| {
        let (source, gate) = fixture("* [x] first\r\n* [ ] second\n");
        let canonical = source.snapshot().unwrap().rope.to_string();
        select_byte(&source, &gate, canonical.find("second").unwrap() + 3);
        let before = identity(&source);
        let session = gate.session();
        let state = session.state();
        let handled = state.plugins().iter().any(|plugin| {
            plugin.handle_key_down(&state, &KeyEvent::new("Tab"), &mut |tr| {
                session.dispatch(tr).unwrap();
            })
        });
        assert!(!handled, "taskItem nested=false has no Tab action");
        assert_eq!(identity(&source), before);
        assert_eq!(history(&source), (0, 0));
        assert_eq!(session.state().doc(), state.doc());
    });
}

#[test]
fn list_terminal_trivia_keeps_imported_empty_items_and_mode_switches_byte_exact() {
    runtime(|| {
        for (raw, blocks, items, empty_tail) in [
            ("+ first\n", 1, 1, false),
            ("+ first\r\n", 1, 1, false),
            ("+ first\r\n+ \n", 1, 2, false),
            ("+ first\r\n\n", 2, 1, true),
            ("+ first\r\n+ second\n\nend\r\n", 2, 2, false),
        ] {
            let source = NativeView::fixture(raw);
            let original = identity(&source);
            let revision = source.snapshot().unwrap().revision;
            let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
            for _ in 0..2 {
                gate.to_rich();
                assert!(gate.rich_mode.get(), "{raw:?}: {}", gate.status.get());
                let state = gate.session().state();
                assert_eq!(state.doc().child_count(), blocks, "{raw:?}");
                assert_eq!(state.doc().child(0).ty().name(), "bulletList");
                assert_eq!(state.doc().child(0).child_count(), items);
                if items == 2 && !raw.contains("second") {
                    assert_eq!(state.doc().child(0).child(1).child(0).content().size(), 0);
                }
                if empty_tail {
                    assert_eq!(state.doc().child(1).ty().name(), "paragraph");
                    assert_eq!(state.doc().child(1).content().size(), 0);
                }
                assert_eq!(identity(&source), original);
                assert_eq!(source.snapshot().unwrap().revision, revision);
                assert_eq!(history(&source), (0, 0));
                assert!(!source.inspect(|s, id| s.dirty(s.view(id).unwrap().doc).unwrap()));
                gate.to_source();
                assert!(!gate.rich_mode.get(), "{}", gate.status.get());
                gate.source_mounted();
                assert_eq!(identity(&source), original);
            }
        }
    });
}
