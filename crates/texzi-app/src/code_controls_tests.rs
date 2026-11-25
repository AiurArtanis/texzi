//! Real code controls -> ModeGate -> RichOwner -> Store fixtures. No GUI,
//! filesystem writes, clipboard, process input injection or network are used.
use crate::{
    code_controls::CodeControls, editor_owner::NativeView, rich_owner::ModeGate,
    rich_projection::Projection,
};
use nsh_base::{
    editor::{ExternalDocument, ExternalEditKind, ExternalSelections},
    rich_editor::state::{KeyEvent, TextSelection},
};
use nsh_core::prelude::*;
use std::rc::Rc;

fn run(f: impl FnOnce()) {
    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    f();
}
fn bytes(source: &NativeView) -> Vec<u8> {
    source.inspect(|store, id| {
        store
            .snapshot(store.view(id).unwrap().doc)
            .unwrap()
            .body
            .to_utf8()
    })
}
fn audit(source: &NativeView) -> String {
    source.inspect(|store, id| {
        let view = store.view(id).unwrap();
        let snapshot = store.snapshot(view.doc).unwrap();
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            view,
            snapshot.token,
            snapshot.body.to_utf8(),
            snapshot.body.origins(),
            store.history_depth(view.doc),
            store.dirty(view.doc)
        )
    })
}
fn depth(source: &NativeView) -> (usize, usize) {
    source.inspect(|store, id| store.history_depth(store.view(id).unwrap().doc).unwrap())
}
fn retained(source: &NativeView) -> usize {
    source.inspect(|store, id| {
        store
            .snapshot(store.view(id).unwrap().doc)
            .unwrap()
            .body
            .origins()
            .into_iter()
            .filter_map(|(_, origin)| origin.map(|origin| origin.raw.len()))
            .sum()
    })
}
fn selection(gate: &ModeGate, source: &NativeView, first: &str, last: &str) {
    let session = gate.session();
    let state = session.state();
    let raw = source.snapshot().unwrap().rope.to_string();
    let p = Projection::parse(state.schema(), raw.clone()).unwrap();
    let a = p.rich_position(raw.find(first).unwrap()).unwrap();
    let z = p
        .rich_position(raw.find(last).unwrap() + last.len())
        .unwrap();
    let mut tr = state.tr();
    tr.set_selection(TextSelection::create(state.doc(), a, Some(z)));
    session.dispatch(tr).unwrap();
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
}
fn setup(raw: &str, needle: &str) -> (Rc<NativeView>, Rc<ModeGate>, Rc<CodeControls>) {
    let source = NativeView::fixture(raw);
    let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
    gate.to_rich();
    assert!(gate.rich_mode.get(), "{}", gate.status.get());
    let session = gate.session();
    let state = session.state();
    let logical = source.snapshot().unwrap().rope.to_string();
    let p = Projection::parse(state.schema(), logical.clone()).unwrap();
    let pos = p.rich_position(logical.find(needle).unwrap()).unwrap();
    let mut tr = state.tr();
    tr.set_selection(TextSelection::create(state.doc(), pos, None));
    session.dispatch(tr).unwrap();
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
    let controls = CodeControls::new(gate.clone(), source.clone());
    (source, gate, controls)
}
fn roundtrip(source: &NativeView, gate: &ModeGate, raw: &str, expected: &str) {
    assert_eq!(bytes(source), expected.as_bytes());
    assert_eq!(depth(source), (1, 0));
    gate.to_source();
    assert!(!gate.rich_mode.get(), "{}", gate.status.get());
    gate.shell_history(true).unwrap();
    assert_eq!(bytes(source), raw.as_bytes());
    assert_eq!(depth(source), (0, 1));
    source.inspect(|store, id| assert!(!store.dirty(store.view(id).unwrap().doc).unwrap()));
    gate.shell_history(false).unwrap();
    assert_eq!(bytes(source), expected.as_bytes());
    assert_eq!(depth(source), (1, 0));
}
fn key(gate: &ModeGate, shift: bool) {
    let session = gate.session();
    let state = session.state();
    let mut event = KeyEvent::new("Tab");
    event.shift = shift;
    assert!(
        state
            .plugins()
            .iter()
            .any(|p| p.handle_key_down(&state, &event, &mut |tr| session.dispatch(tr).unwrap())),
        "installed code Tab binding"
    );
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
}

#[test]
fn code_language_updates_only_fence_token_preserving_info_body_eols_and_shared_history() {
    run(|| {
        let raw = "# before\r\n\r\n~~~~  rust  linenums=1 extra=keep\r\nlet rust = 1;\n~~~~  \r\n\r\n<custom>stay</custom>\n";
        let (source, gate, controls) = setup(raw, "let rust");
        controls.begin().unwrap();
        assert_eq!(controls.language.get(), "rust");
        assert_eq!(controls.info.get(), "linenums=1 extra=keep");
        controls.language.set("python".into());
        controls.apply_language().unwrap();
        let expected = raw.replacen("~~~~  rust  ", "~~~~  python  ", 1);
        assert_eq!(retained(&source), raw.len() - 4);
        roundtrip(&source, &gate, raw, &expected);
    });
}

#[test]
fn code_language_add_clear_and_cancel_keep_original_fence_style() {
    run(|| {
        for (raw, language, expected) in [
            ("~~~\r\nalpha\n~~~\r\n", "rust", "~~~rust\r\nalpha\n~~~\r\n"),
            ("```rust   \nalpha\n```", "", "```   \nalpha\n```"),
        ] {
            let (source, gate, controls) = setup(raw, "alpha");
            let before = audit(&source);
            controls.begin().unwrap();
            controls.language.set(language.into());
            controls.cancel();
            assert_eq!(audit(&source), before);
            controls.begin().unwrap();
            controls.language.set(language.into());
            controls.apply_language().unwrap();
            roundtrip(&source, &gate, raw, expected);
        }
    });
}

#[test]
fn code_exit_reuses_next_editable_block_skipping_literal_source_without_body_history_edit() {
    run(|| {
        let raw = "~~~rust\r\none\n~~~\r\n\r\n<custom>stay</custom>\n\n## after\r\n";
        let (source, gate, controls) = setup(raw, "one");
        let before = audit(&source);
        controls.exit().unwrap();
        assert_eq!(audit(&source), before);
        assert_eq!(depth(&source), (0, 0));
        let logical = source.snapshot().unwrap().rope.to_string();
        let expected = logical.find("after").unwrap();
        gate.to_source();
        assert_eq!(
            source.snapshot().unwrap().selections.ranges,
            vec![(expected, expected)]
        );
        assert_eq!(bytes(&source), raw.as_bytes());
    });
}

#[test]
fn code_exit_at_document_end_appends_one_empty_paragraph_and_undo_restores_terminal_eol() {
    run(|| {
        for (raw, expected) in [
            ("~~~rust\r\none\r\n~~~", "~~~rust\r\none\r\n~~~\r\n\r\n"),
            ("~~~rust\none\n~~~\n", "~~~rust\none\n~~~\n\n"),
            (
                "~~~rust\none\n~~~\n\n<custom>stay</custom>\n",
                "~~~rust\none\n~~~\n\n<custom>stay</custom>\n\n",
            ),
        ] {
            let (source, gate, controls) = setup(raw, "one");
            let token = source
                .inspect(|store, id| store.snapshot(store.view(id).unwrap().doc).unwrap().token);
            controls.exit().unwrap();
            let state = gate.session().state();
            let last = state.doc().last_child().unwrap();
            assert_eq!(last.ty().name(), "paragraph");
            assert_eq!(last.content().size(), 0);
            assert_eq!(retained(&source), raw.len());
            roundtrip(&source, &gate, raw, expected);
            gate.shell_history(true).unwrap();
            assert_eq!(
                source.inspect(|store, id| store
                    .snapshot(store.view(id).unwrap().doc)
                    .unwrap()
                    .token),
                token
            );
        }
    });
}

#[test]
fn installed_code_tab_and_shift_tab_keep_mixed_eol_origins_and_source_undo() {
    run(|| {
        let raw = "~~~rust\r\n甲alpha\nbeta\r\n~~~\n\n<custom>stay</custom>\r\n";
        let (source, gate, _) = setup(raw, "甲alpha");
        selection(&gate, &source, "甲alpha", "beta");
        key(&gate, false);
        let indented = raw
            .replacen("甲alpha", "    甲alpha", 1)
            .replacen("beta", "    beta", 1);
        assert_eq!(bytes(&source), indented.as_bytes());
        assert_eq!(retained(&source), raw.len());
        assert_eq!(depth(&source), (1, 0));
        selection(&gate, &source, "    甲alpha", "    beta");
        key(&gate, true);
        assert_eq!(bytes(&source), raw.as_bytes());
        assert_eq!(depth(&source), (2, 0));
        gate.to_source();
        gate.shell_history(true).unwrap();
        assert_eq!(bytes(&source), indented.as_bytes());
        gate.shell_history(true).unwrap();
        assert_eq!(bytes(&source), raw.as_bytes());
        gate.shell_history(false).unwrap();
        assert_eq!(bytes(&source), indented.as_bytes());
    });
}

#[test]
fn code_tab_at_adjacent_empty_fence_materializes_only_missing_body_separator() {
    run(|| {
        let raw = "~~~rust\r\n~~~";
        let source = NativeView::fixture(raw);
        let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
        gate.to_rich();
        assert!(gate.rich_mode.get(), "{}", gate.status.get());
        let controls = CodeControls::new(gate.clone(), source.clone());
        let session = gate.session();
        let state = session.state();
        let mut tr = state.tr();
        tr.set_selection(TextSelection::create(state.doc(), 1, None));
        session.dispatch(tr).unwrap();
        controls.indent(false).unwrap();
        assert_eq!(retained(&source), raw.len());
        roundtrip(&source, &gate, raw, "~~~rust\r\n    \r\n~~~");
    });
}

#[test]
fn code_controls_reject_literal_indented_language_unclosed_exit_ime_and_stale_draft() {
    run(|| {
        for (raw, needle) in [("<custom>stay</custom>\n", "stay"), ("$x+y$\n", "x+y")] {
            let (source, _, controls) = setup(raw, needle);
            let before = audit(&source);
            assert!(controls.begin().is_err());
            assert!(controls.exit().is_err());
            assert!(controls.indent(false).is_err());
            assert_eq!(audit(&source), before);
        }
        let (source, _, controls) = setup("    alpha\r\n    beta\n", "alpha");
        let before = audit(&source);
        assert!(controls.begin().is_err());
        assert_eq!(audit(&source), before);
        let (source, _, controls) = setup("~~~rust\nalpha", "alpha");
        let before = audit(&source);
        assert!(controls.exit().is_err());
        assert_eq!(audit(&source), before);
        let (source, _, controls) = setup("~~~rust\nalpha\n~~~\n\nkeep\n", "alpha");
        controls.begin().unwrap();
        let before_ime = audit(&source);
        source.set_composing(true);
        assert!(controls.begin().is_err());
        assert!(controls.apply_language().is_err());
        assert!(controls.exit().is_err());
        assert!(controls.indent(false).is_err());
        assert_eq!(audit(&source), before_ime);
        source.set_composing(false);
        let before = audit(&source);
        source.set_composing(true);
        assert!(controls.begin().is_err());
        assert!(controls.apply_language().is_err());
        assert!(controls.exit().is_err());
        assert!(controls.indent(false).is_err());
        assert_eq!(audit(&source), before);
        source.set_composing(false);
        let snapshot = source.snapshot().unwrap();
        let logical = snapshot.rope.to_string();
        let at = logical.find("keep").unwrap();
        batch(|| {
            source
                .edit(
                    snapshot.revision,
                    snapshot.view_epoch,
                    vec![(at..at + 1, "K".into())],
                    ExternalSelections {
                        primary: 0,
                        ranges: vec![(at + 1, at + 1)],
                    },
                    ExternalEditKind::Other,
                )
                .unwrap();
            source.notify();
        });
        let changed = audit(&source);
        controls.language.set("python".into());
        assert!(controls.apply_language().is_err());
        assert_eq!(audit(&source), changed);
    });
}

#[test]
fn code_language_metadata_cannot_change_body_or_a_literal_neighbor() {
    run(|| {
        let raw = "~~~rust\nalpha\n~~~\n\n<custom>stay</custom>\n";
        let (source, gate, _) = setup(raw, "alpha");
        let before = audit(&source);
        let session = gate.session();
        let state = session.state();
        let p =
            Projection::parse(state.schema(), source.snapshot().unwrap().rope.to_string()).unwrap();
        let target = p.current_code_target(&state).unwrap();
        let mut tr = p
            .code_language_transaction(&state, target.position, Some("python"))
            .unwrap();
        tr.insert_text(
            "FORGED",
            Some(state.selection().head()),
            Some(state.selection().head()),
        );
        session.dispatch(tr).unwrap();
        assert!(!gate.status.get().is_empty());
        assert_eq!(audit(&source), before);
        assert_eq!(bytes(&source), raw.as_bytes());
    });
}
