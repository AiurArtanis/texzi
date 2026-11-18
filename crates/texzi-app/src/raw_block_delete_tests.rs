//! Exact raw-block node deletion through installed keys, shared Store history.
//! Component coverage only; no application-window or manual claim.
use crate::{editor_owner::NativeView, rich_owner::ModeGate, rich_projection::Projection};
use nsh_base::{
    editor::ExternalDocument,
    rich_editor::state::{KeyEvent, TextSelection},
};
use nsh_core::prelude::*;

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
    source.inspect(|store, id| store.history_depth(store.view(id).unwrap().doc).unwrap())
}
fn runtime(f: impl FnOnce()) {
    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    f();
}
fn delete_case(raw: &str, target: &str, expected: &str) {
    let source = NativeView::fixture(raw);
    let rich = source.sibling().unwrap();
    let gate = ModeGate::new(source.clone(), rich.clone());
    gate.to_rich();
    assert!(gate.rich_mode.get(), "{raw:?}: {}", gate.status.get());
    let original = identity(&source);
    let session = gate.session();
    let old = session.state();
    let p = Projection::parse(old.schema(), source.snapshot().unwrap().rope.to_string()).unwrap();
    let action = gate
        .raw_blocks()
        .into_iter()
        .find(|action| p.source[action.source.clone()].contains(target))
        .expect("explicit opaque target has a local whole-block control");
    gate.select_raw_block(&action).unwrap();
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
    assert_eq!(
        identity(&source),
        original,
        "node selection is not a body edit"
    );
    let selected = session.state();
    assert!(selected.selection().as_node().is_some());
    let handled = selected.plugins().iter().any(|plugin| {
        plugin.handle_key_down(&selected, &KeyEvent::new("Delete"), &mut |tr| {
            session.dispatch(tr).unwrap();
        })
    });
    assert!(
        handled,
        "installed Delete binding must delete the selected node"
    );
    assert!(
        gate.status.get().is_empty(),
        "{raw:?}: {}",
        gate.status.get()
    );
    assert_ne!(
        session.state().doc(),
        old.doc(),
        "rejection is not successful deletion"
    );
    let edited = identity(&source);
    assert_eq!(
        edited.bytes,
        expected.as_bytes(),
        "only target and necessary separator disappear"
    );
    assert_ne!(edited.token, original.token);
    assert_eq!(history(&source), (1, 0));
    assert_eq!(identity(&rich), edited, "sibling views share one body");
    source.inspect(|store, id| {
        let s = store.snapshot(store.view(id).unwrap().doc).unwrap();
        let mut retained = Vec::new();
        for (_, origin) in s.body.origins() {
            let origin = origin.expect("deletion must not synthesize any retained bytes");
            assert_eq!(origin.source.as_ref(), raw);
            retained.extend_from_slice(&origin.source.as_bytes()[origin.raw]);
        }
        assert_eq!(
            retained,
            expected.as_bytes(),
            "all surviving bytes retain their original provenance"
        );
    });
    let state = session.state();
    assert_eq!(
        Projection::parse(state.schema(), source.snapshot().unwrap().rope.to_string())
            .unwrap()
            .doc,
        *state.doc()
    );
    gate.to_source();
    assert!(!gate.rich_mode.get(), "{}", gate.status.get());
    gate.source_mounted();
    gate.shell_history(true).unwrap();
    assert_eq!(
        identity(&source),
        original,
        "Source Undo restores bytes/token/EOL provenance"
    );
    assert_eq!(history(&source), (0, 1));
    gate.shell_history(false).unwrap();
    assert_eq!(
        identity(&source),
        edited,
        "Source Redo restores the same edited identity"
    );
    assert_eq!(history(&source), (1, 0));
    gate.to_rich();
    assert!(gate.rich_mode.get(), "{}", gate.status.get());
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
}

#[test]
fn raw_block_delete_html_math_unknown_at_first_middle_last_and_only_position() {
    runtime(|| {
        for block in [
            "<custom>alpha</custom>",
            "$$\r\nalpha + beta\n$$",
            "::: custom\r\nalpha\n:::",
        ] {
            delete_case(&format!("{block}\r\n\r\nright😀\n"), "alpha", "right😀\n");
            delete_case(
                &format!("left😀\r\n\r\n{block}\n\nright甲\r\n"),
                "alpha",
                "left😀\r\n\r\nright甲\r\n",
            );
            delete_case(&format!("left😀\r\n\r\n{block}\n"), "alpha", "left😀\n");
            delete_case(&format!("{block}\r\n"), "alpha", "\r\n");
        }
    });
}

#[test]
fn raw_block_delete_preserves_other_opaque_nodes_and_their_original_eols() {
    runtime(|| {
        delete_case(
            "<stay>left</stay>\r\n\r\n$$\nalpha\r\n$$\n\n<stay>right</stay>\r\n",
            "alpha",
            "<stay>left</stay>\r\n\r\n<stay>right</stay>\r\n",
        );
    });
}

#[test]
fn raw_block_delete_partial_selection_still_rejects_without_a_store_commit() {
    runtime(|| {
        for raw in [
            "<custom>alpha</custom>\r\n",
            "$$\r\nalpha\n$$\r\n",
            "::: custom\r\nalpha\n:::\r\n",
        ] {
            let source = NativeView::fixture(raw);
            let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
            gate.to_rich();
            assert!(gate.rich_mode.get(), "{}", gate.status.get());
            let session = gate.session();
            let old = session.state();
            let p = Projection::parse(old.schema(), source.snapshot().unwrap().rope.to_string())
                .unwrap();
            let byte = p.source.find("alpha").unwrap();
            let at = p.rich_position(byte).unwrap();
            let mut selection = old.tr();
            selection.set_selection(TextSelection::create(old.doc(), at, Some(at + 1)));
            session.dispatch(selection).unwrap();
            assert!(gate.status.get().is_empty(), "{}", gate.status.get());
            let selected = session.state();
            let original = identity(&source);
            let revision = source.snapshot().unwrap().revision;
            assert!(selected.plugins().iter().any(|plugin| {
                plugin.handle_key_down(&selected, &KeyEvent::new("Delete"), &mut |tr| {
                    session.dispatch(tr).unwrap()
                })
            }));
            assert!(
                gate.status.get().contains("编辑源码"),
                "{}",
                gate.status.get()
            );
            assert_eq!(session.state().doc(), selected.doc());
            assert_eq!(identity(&source), original);
            assert_eq!(source.snapshot().unwrap().revision, revision);
            assert_eq!(history(&source), (0, 0));
        }
    });
}
