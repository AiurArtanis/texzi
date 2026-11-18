//! Exact entity/escape selection regression through ModeGate -> RichOwner ->
//! NativeView -> Store. Logical positions do not claim a real Ctrl+End key or
//! pointer hit-test result. No document writes are used to prepare a selection.
use crate::{editor_owner::NativeView, rich_owner::ModeGate, rich_projection::Projection};
use nsh_base::{
    editor::{ExternalDocument, ExternalSelections},
    rich_editor::state::{Selection, TextSelection, TransactionOrigin},
};

const LOGICAL: &str = "# 中文标题\na &amp; b \\*转义\\*";

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExactBody {
    bytes: Vec<u8>,
    logical: String,
    token: texzi_document::StateToken,
    profile: texzi_document::Profile,
    eols: Vec<texzi_document::Eol>,
    origins: String,
    dirty: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct DocumentState {
    doc: texzi_document::DocId,
    revision: texzi_document::Revision,
    body: ExactBody,
    history: (usize, usize),
}
fn run(f: impl FnOnce()) {
    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    f();
}
fn document(view: &NativeView) -> DocumentState {
    view.inspect(|store, id| {
        let doc = store.view(id).unwrap().doc;
        let snapshot = store.snapshot(doc).unwrap();
        DocumentState {
            doc,
            revision: snapshot.revision,
            body: ExactBody {
                bytes: snapshot.body.to_utf8(),
                logical: snapshot.body.source().to_string(),
                token: snapshot.token,
                profile: snapshot.body.profile,
                eols: snapshot.body.eols(),
                origins: format!("{:?}", snapshot.body.origins()),
                dirty: store.dirty(doc).unwrap(),
            },
            history: store.history_depth(doc).unwrap(),
        }
    })
}
fn retained_bytes(view: &NativeView) -> Vec<u8> {
    view.inspect(|store, id| {
        let snapshot = store.snapshot(store.view(id).unwrap().doc).unwrap();
        let mut bytes = Vec::new();
        for (_, origin) in snapshot.body.origins() {
            if let Some(origin) = origin {
                bytes.extend_from_slice(origin.source[origin.raw].as_bytes());
            }
        }
        bytes
    })
}
fn select_source(source: &NativeView, position: usize) {
    let snapshot = source.snapshot().unwrap();
    source
        .select(
            snapshot.revision,
            snapshot.view_epoch,
            ExternalSelections {
                primary: 0,
                ranges: vec![(position, position)],
            },
        )
        .unwrap();
}
fn assert_position(gate: &ModeGate, rich: &NativeView, position: usize, byte: usize) {
    let state = gate.session().state();
    assert_eq!(state.selection().anchor(), position);
    assert_eq!(state.selection().head(), position);
    let native = rich.snapshot().unwrap();
    assert_eq!(native.selections.primary, 0);
    assert_eq!(native.selections.ranges, vec![(byte, byte)]);
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
}

#[test]
fn entity_heading_fixed_source_rich_boundaries_and_selection_transactions_preserve_document_state()
{
    run(|| {
        let source = NativeView::fixture(LOGICAL);
        let rich = source.sibling().unwrap();
        let gate = ModeGate::new(source.clone(), rich.clone());
        let initial = document(&source);
        assert_eq!(initial.body.logical, LOGICAL);
        assert_eq!(initial.body.bytes, LOGICAL.as_bytes());
        assert_eq!(initial.history, (0, 0));
        assert!(!initial.body.dirty);
        gate.to_rich();
        assert!(gate.rich_mode.get(), "{}", gate.status.get());
        assert_eq!(document(&source), initial);
        assert_eq!(document(&rich), initial);
        let session = gate.session();
        let state = session.state();
        assert_eq!(state.doc().child_count(), 2);
        assert_eq!(state.doc().child(0).ty().name(), "heading");
        assert_eq!(state.doc().child(0).text_content(), "中文标题");
        assert_eq!(state.doc().child(1).ty().name(), "paragraph");
        assert_eq!(state.doc().child(1).text_content(), "a & b *转义*");
        let projection = Projection::parse(state.schema(), LOGICAL.into()).unwrap();
        // Hand-written oracle: Source is UTF8 bytes; Rich is UTF16 units plus
        // node boundaries. Each escape/entity owns all of its source spelling.
        for (byte, position) in [
            (2, 1),
            (5, 2),
            (8, 3),
            (11, 4),
            (14, 5),
            (15, 7),
            (16, 8),
            (17, 9),
            (22, 10),
            (23, 11),
            (24, 12),
            (25, 13),
            (27, 14),
            (30, 15),
            (33, 16),
            (35, 17),
        ] {
            assert_eq!(
                projection.rich_position(byte),
                Some(position),
                "Source{byte}"
            );
            assert_eq!(
                projection.source_position(position),
                Some(byte),
                "Rich{position}"
            );
        }
        let doc = state.doc().clone();
        let end = Selection::at_end(&doc);
        assert_eq!((end.anchor(), end.head()), (17, 17));
        let mut transaction = state.tr();
        transaction.set_selection(end);
        session.dispatch(transaction).unwrap();
        assert_position(&gate, &rich, 17, 35);
        assert_eq!(session.state().doc(), &doc);
        assert_eq!(document(&source), initial);
        assert_eq!(document(&rich), initial);
        let state = session.state();
        let mut transaction = state.tr();
        transaction.set_selection(TextSelection::create(state.doc(), 14, None));
        session.dispatch(transaction).unwrap();
        assert_position(&gate, &rich, 14, 27);
        assert_eq!(session.state().doc(), &doc);
        assert_eq!(document(&source), initial);
        assert_eq!(document(&rich), initial);
        gate.to_source();
        assert!(!gate.rich_mode.get(), "{}", gate.status.get());
        assert_eq!(source.snapshot().unwrap().selections.ranges, vec![(27, 27)]);
        assert_eq!(document(&source), initial);
        assert_eq!(document(&rich), initial);
    });
}

#[test]
fn entity_heading_source_positions_prepare_live_rich_selection_and_local_typing_has_single_shared_undo_redo()
 {
    run(|| {
        // These are one logical fixture with LF and CRLF storage. Their exact
        // source map is identical; the only line's original EOL must survive.
        for raw in [LOGICAL, "# 中文标题\r\na &amp; b \\*转义\\*"] {
            let source = NativeView::fixture(raw);
            let rich = source.sibling().unwrap();
            let gate = ModeGate::new(source.clone(), rich.clone());
            let initial = document(&source);
            assert_eq!(initial.body.logical, LOGICAL);
            assert_eq!(initial.history, (0, 0));
            assert!(!initial.body.dirty);
            // Move only the Source view. The old Rich sibling still has its
            // initial position, so the destination cannot rely on that cache.
            select_source(&source, 35);
            assert_eq!(rich.snapshot().unwrap().selections.ranges, vec![(0, 0)]);
            assert_eq!(document(&source), initial);
            gate.to_rich();
            assert!(gate.rich_mode.get(), "{}", gate.status.get());
            assert_position(&gate, &rich, 17, 35);
            assert_eq!(document(&source), initial);
            assert_eq!(document(&rich), initial);
            gate.to_source();
            assert!(!gate.rich_mode.get(), "{}", gate.status.get());
            assert_eq!(source.snapshot().unwrap().selections.ranges, vec![(35, 35)]);
            select_source(&source, 27);
            assert_eq!(rich.snapshot().unwrap().selections.ranges, vec![(35, 35)]);
            gate.to_rich();
            assert!(gate.rich_mode.get(), "{}", gate.status.get());
            assert_position(&gate, &rich, 14, 27);
            assert_eq!(document(&source), initial);
            assert_eq!(document(&rich), initial);
            let session = gate.session();
            let state = session.state();
            let mut transaction = state.tr();
            // Use the actual prepared selection, not an explicit insert range.
            transaction.insert_text("乙", None, None);
            transaction.set_origin(TransactionOrigin::Input);
            session.dispatch(transaction).unwrap();
            assert_position(&gate, &rich, 15, 30);
            let expected = raw.replacen("\\*转义\\*", "\\*乙转义\\*", 1);
            let applied = document(&source);
            assert_eq!(applied.body.bytes, expected.as_bytes());
            assert_eq!(applied.body.logical, "# 中文标题\na &amp; b \\*乙转义\\*");
            assert_eq!(applied.body.profile, initial.body.profile);
            assert_eq!(applied.body.eols, initial.body.eols);
            assert_eq!(retained_bytes(&source), raw.as_bytes());
            assert_ne!(applied.body.token, initial.body.token);
            assert!(applied.body.dirty);
            assert_eq!(applied.revision.0, initial.revision.0 + 1);
            assert_eq!(applied.history, (1, 0));
            assert_eq!(document(&rich), applied);
            assert_eq!(session.state().doc().child(0).text_content(), "中文标题");
            assert_eq!(
                session.state().doc().child(1).text_content(),
                "a & b *乙转义*"
            );
            gate.to_source();
            assert!(!gate.rich_mode.get(), "{}", gate.status.get());
            assert_eq!(source.snapshot().unwrap().selections.ranges, vec![(30, 30)]);
            assert_eq!(document(&source), applied);
            gate.shell_history(true).unwrap();
            let undone = document(&source);
            assert_eq!(undone.body, initial.body);
            assert_eq!(undone.revision.0, applied.revision.0 + 1);
            assert_eq!(undone.history, (0, 1));
            assert_eq!(document(&rich), undone);
            gate.shell_history(false).unwrap();
            let redone = document(&source);
            assert_eq!(redone.body, applied.body);
            assert_eq!(redone.revision.0, undone.revision.0 + 1);
            assert_eq!(redone.history, (1, 0));
            assert_eq!(document(&rich), redone);
        }
    });
}

#[test]
fn entity_heading_selected_amp_entity_replacement_changes_only_its_source_token_and_roundtrips_history()
 {
    run(|| {
        for raw in [LOGICAL, "# 中文标题\r\na &amp; b \\*转义\\*"] {
            let source = NativeView::fixture(raw);
            let rich = source.sibling().unwrap();
            let gate = ModeGate::new(source.clone(), rich.clone());
            let initial = document(&source);
            let snapshot = source.snapshot().unwrap();
            source
                .select(
                    snapshot.revision,
                    snapshot.view_epoch,
                    ExternalSelections {
                        primary: 0,
                        ranges: vec![(17, 22)],
                    },
                )
                .unwrap();
            assert_eq!(document(&source), initial);
            assert_eq!(document(&rich), initial);
            gate.to_rich();
            assert!(gate.rich_mode.get(), "{}", gate.status.get());
            let session = gate.session();
            let state = session.state();
            assert_eq!(
                (state.selection().anchor(), state.selection().head()),
                (9, 10)
            );
            assert_eq!(rich.snapshot().unwrap().selections.ranges, vec![(17, 22)]);
            assert_eq!(state.doc().text_between(9, 10, None, None), "&");
            assert_eq!(document(&source), initial);
            assert_eq!(document(&rich), initial);
            let mut transaction = state.tr();
            // Replace the actual prepared & selection, not an explicit patch.
            transaction.insert_text("和", None, None);
            transaction.set_origin(TransactionOrigin::Input);
            session.dispatch(transaction).unwrap();
            assert_position(&gate, &rich, 10, 20);
            let expected = raw.replacen("&amp;", "和", 1);
            let applied = document(&source);
            assert_eq!(applied.body.bytes, expected.as_bytes());
            assert_eq!(applied.body.logical, "# 中文标题\na 和 b \\*转义\\*");
            assert_eq!(applied.body.profile, initial.body.profile);
            assert_eq!(applied.body.eols, initial.body.eols);
            assert_ne!(applied.body.token, initial.body.token);
            assert!(applied.body.dirty);
            assert_eq!(applied.revision.0, initial.revision.0 + 1);
            assert_eq!(applied.history, (1, 0));
            assert_eq!(document(&rich), applied);
            // Actual Store provenance, not a second Projection as the oracle:
            // every original byte outside &amp; is retained, in original order;
            // the only new logical bytes are the UTF8 encoding of 和 at 17..20.
            assert_eq!(
                retained_bytes(&source),
                raw.replacen("&amp;", "", 1).as_bytes()
            );
            source.inspect(|store, id| {
                let snapshot = store.snapshot(store.view(id).unwrap().doc).unwrap();
                let inserted: Vec<_> = snapshot
                    .body
                    .origins()
                    .into_iter()
                    .filter_map(|(range, origin)| origin.is_none().then_some(range))
                    .collect();
                assert_eq!(inserted, vec![17..20]);
            });
            assert_eq!(session.state().doc().child(0).text_content(), "中文标题");
            assert_eq!(
                session.state().doc().child(1).text_content(),
                "a 和 b *转义*"
            );
            gate.to_source();
            assert!(!gate.rich_mode.get(), "{}", gate.status.get());
            assert_eq!(source.snapshot().unwrap().selections.ranges, vec![(20, 20)]);
            assert_eq!(document(&source), applied);
            gate.shell_history(true).unwrap();
            let undone = document(&source);
            assert_eq!(undone.body, initial.body);
            assert_eq!(undone.revision.0, applied.revision.0 + 1);
            assert_eq!(undone.history, (0, 1));
            assert_eq!(document(&rich), undone);
            gate.shell_history(false).unwrap();
            let redone = document(&source);
            assert_eq!(redone.body, applied.body);
            assert_eq!(redone.revision.0, undone.revision.0 + 1);
            assert_eq!(redone.history, (1, 0));
            assert_eq!(document(&rich), redone);
        }
    });
}
