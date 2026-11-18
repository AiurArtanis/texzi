//! A Source history entry may restore a fence boundary while Rich is active.
//! Nearest visible positioning must synchronize the real Rich NativeView too.
use crate::{editor_owner::NativeView, rich_owner::ModeGate, rich_projection::Projection};
use nsh_base::editor::{ExternalDocument, ExternalEditKind, ExternalSelections};
use nsh_core::prelude::*;

#[derive(Debug, PartialEq, Eq)]
struct BodyIdentity {
    bytes: Vec<u8>,
    token: texzi_document::StateToken,
    eols: Vec<texzi_document::Eol>,
    origins: String,
    profile: String,
}
#[derive(Debug, PartialEq, Eq)]
struct Audit {
    body: BodyIdentity,
    revision: u64,
    history: (usize, usize),
    dirty: bool,
}
fn audit(view: &NativeView) -> Audit {
    view.inspect(|store, id| {
        let s = store.snapshot(store.view(id).unwrap().doc).unwrap();
        Audit {
            body: BodyIdentity {
                bytes: s.body.to_utf8(),
                token: s.token,
                eols: s.body.eols(),
                origins: format!("{:?}", s.body.origins()),
                profile: format!("{:?}", s.body.profile),
            },
            revision: s.revision.0,
            history: store.history_depth(s.doc).unwrap(),
            dirty: store.dirty(s.doc).unwrap(),
        }
    })
}
fn assert_code_selection(source: &NativeView, rich: &NativeView, gate: &ModeGate) {
    let before = audit(source);
    assert!(gate.rich_mode.get());
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
    let state = gate.session().state();
    let native = rich.snapshot().unwrap();
    let canonical = native.rope.to_string();
    let p = Projection::parse(state.schema(), canonical.clone()).unwrap();
    assert_eq!(p.doc, *state.doc());
    let mut base = 0;
    let code = p
        .blocks
        .iter()
        .find_map(|block| {
            let start = base;
            base += block.node.node_size();
            (block.node.ty().name() == "codeBlock"
                && !block.opaque
                && block.node.text_content().contains("键🚀"))
            .then_some((start + 1, base - 1))
        })
        .expect("the original JSON block must remain editable");
    let (a, h) = native.selections.ranges[native.selections.primary];
    let body_start = canonical.find("{\"键🚀\"").unwrap();
    // A preceding paragraph also has a newline before the opening fence.
    // The independent fixture oracle must search after the JSON body start.
    let body_end = body_start + canonical[body_start..].find("\n```").unwrap();
    for (position, byte) in [
        (state.selection().anchor(), a),
        (state.selection().head(), h),
    ] {
        assert!(
            position >= code.0 && position <= code.1,
            "nearest position must stay inside JSON, not the adjacent raw block"
        );
        assert!(
            byte >= body_start && byte <= body_end,
            "native SourceByte must point to visible JSON, not a fence"
        );
        assert_eq!(
            p.source_position(position),
            Some(byte),
            "derived Rich selection and actual NativeView must agree"
        );
        assert_eq!(p.rich_position(byte), Some(position));
    }
    assert_eq!(
        audit(rich),
        before,
        "both views share one authoritative body/history"
    );
    assert_eq!(
        audit(source),
        before,
        "reading the projection and native selection cannot add a body transaction"
    );
}

#[test]
fn rich_history_of_external_source_edits_syncs_nearest_code_caret_without_extra_body_revision() {
    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    let raw = "```json\r\n{\"键🚀\":true,\"s\":\"x\\\"y\"}\n```\r\n\r\n<custom>stay</custom>\n";
    let bool_raw = raw.replacen("true", "false", 1);
    let prefix = "额🚀\n\n";
    let prefixed_raw = format!("额🚀\r\n\r\n{bool_raw}");
    let source = NativeView::fixture(raw);
    assert_eq!(source.snapshot().unwrap().selections.ranges, vec![(0, 0)]);
    let original = audit(&source);
    assert_eq!(original.body.bytes, raw.as_bytes());
    assert_eq!(original.history, (0, 0));
    assert!(!original.dirty);
    let rich = source.sibling().unwrap();
    let gate = ModeGate::new(source.clone(), rich.clone());
    gate.to_rich();
    assert!(gate.rich_mode.get(), "{}", gate.status.get());
    assert_eq!(
        audit(&source),
        original,
        "switching mode is view state only"
    );
    assert_code_selection(&source, &rich, &gate);

    let s = source.snapshot().unwrap();
    let at = s.rope.to_string().find("true").unwrap();
    source
        .edit(
            s.revision,
            s.view_epoch,
            vec![(at..at + 4, "false".into())],
            ExternalSelections {
                primary: 0,
                ranges: vec![(at + 5, at + 5)],
            },
            ExternalEditKind::Other,
        )
        .unwrap();
    source.notify();
    let bool_edit = audit(&source);
    assert_eq!(bool_edit.body.bytes, bool_raw.as_bytes());
    assert_eq!(bool_edit.revision, original.revision + 1);
    assert_eq!(bool_edit.history, (1, 0));
    assert!(bool_edit.dirty);
    assert_code_selection(&source, &rich, &gate);

    let s = source.snapshot().unwrap();
    source
        .edit(
            s.revision,
            s.view_epoch,
            vec![(0..0, prefix.into())],
            ExternalSelections {
                primary: 0,
                ranges: vec![(prefix.len(), prefix.len())],
            },
            ExternalEditKind::Other,
        )
        .unwrap();
    source.notify();
    let prefix_edit = audit(&source);
    assert_eq!(prefix_edit.body.bytes, prefixed_raw.as_bytes());
    assert_eq!(prefix_edit.revision, bool_edit.revision + 1);
    assert_eq!(prefix_edit.history, (2, 0));
    assert!(prefix_edit.dirty);
    assert_code_selection(&source, &rich, &gate);

    let mut revision = prefix_edit.revision;
    for (undo, expected, history, dirty) in [
        (true, &bool_edit.body, (1, 1), true),
        (true, &original.body, (0, 2), false),
        (false, &bool_edit.body, (1, 1), true),
        (false, &prefix_edit.body, (2, 0), true),
    ] {
        gate.shell_history(undo)
            .expect("Rich history must succeed even when Source recorded a fence boundary");
        let restored = audit(&source);
        assert_eq!(
            &restored.body, expected,
            "bytes/token/EOL/profile/origins must restore the complete history snapshot"
        );
        revision += 1;
        assert_eq!(
            restored.revision, revision,
            "one Undo/Redo creates exactly one body revision"
        );
        assert_eq!(restored.history, history);
        assert_eq!(restored.dirty, dirty);
        assert_code_selection(&source, &rich, &gate);
    }
}
