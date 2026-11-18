//! New links use the same form, real RichOwner and single Store history.
use super::*;
use crate::rich_commands::{self, FormRequest, Outcome};
use nsh_base::{
    editor::{ExternalEditKind, ExternalSelections},
    rich_editor::state::TextSelection,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExactBody {
    bytes: Vec<u8>,
    token: texzi_document::StateToken,
    profile: texzi_document::Profile,
    eols: Vec<texzi_document::Eol>,
    origins: String,
    dirty: bool,
}
fn run(f: impl FnOnce()) {
    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    f();
}
fn body(source: &NativeView) -> ExactBody {
    source.inspect(|store, id| {
        let doc = store.view(id).unwrap().doc;
        let snapshot = store.snapshot(doc).unwrap();
        ExactBody {
            bytes: snapshot.body.to_utf8(),
            token: snapshot.token,
            profile: snapshot.body.profile,
            eols: snapshot.body.eols(),
            origins: format!("{:?}", snapshot.body.origins()),
            dirty: store.dirty(doc).unwrap(),
        }
    })
}
fn audit(source: &NativeView) -> String {
    source.inspect(|store, id| {
        let view = store.view(id).unwrap();
        let snapshot = store.snapshot(view.doc).unwrap();
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            view,
            snapshot.revision,
            snapshot.token,
            snapshot.body.to_utf8(),
            snapshot.body.profile,
            snapshot.body.eols(),
            snapshot.body.origins(),
            store.history_depth(view.doc),
            store.dirty(view.doc),
            store.last_fence(view.doc)
        )
    })
}
fn history(source: &NativeView) -> (usize, usize) {
    source.inspect(|store, id| store.history_depth(store.view(id).unwrap().doc).unwrap())
}
fn retained_original_bytes(source: &NativeView) -> Vec<u8> {
    source.inspect(|store, id| {
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
fn setup(
    raw: &str,
    first: &str,
    last: Option<&str>,
) -> (
    Rc<NativeView>,
    Rc<NativeView>,
    Rc<ModeGate>,
    Rc<ResourceControls>,
) {
    let source = NativeView::fixture(raw);
    let rich = source.sibling().unwrap();
    let gate = ModeGate::new(source.clone(), rich.clone());
    gate.to_rich();
    assert!(gate.rich_mode.get(), "{}", gate.status.get());
    let session = gate.session();
    let state = session.state();
    let logical = source.snapshot().unwrap().rope.to_string();
    let p = Projection::parse(state.schema(), logical.clone()).unwrap();
    let start = logical.find(first).unwrap();
    let end = last.map_or(start, |last| logical.find(last).unwrap() + last.len());
    let a = p
        .rich_position(start)
        .unwrap_or_else(|| p.nearest_rich_position(start).unwrap());
    let z = if last.is_none() {
        a
    } else {
        p.rich_position(end).unwrap()
    };
    let mut tr = state.tr();
    tr.set_selection(TextSelection::create(state.doc(), a, Some(z)));
    session.dispatch(tr).unwrap();
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
    let controls = ResourceControls::new(gate.clone(), source.clone());
    (source, rich, gate, controls)
}
fn submit(controls: &ResourceControls, url: &str, title: &str) {
    controls.begin(ResourceKind::Link).unwrap();
    assert!(controls.creating.get());
    controls.url.set(url.into());
    controls.title.set(title.into());
    controls.apply().unwrap();
    assert!(!controls.open.get());
}
fn roundtrip(
    source: &NativeView,
    rich: &NativeView,
    gate: &ModeGate,
    initial: &ExactBody,
    expected: &str,
) {
    let applied = body(source);
    assert_eq!(applied.bytes, expected.as_bytes());
    assert_eq!(body(rich), applied);
    assert_ne!(applied.token, initial.token);
    assert_eq!(applied.profile, initial.profile);
    assert_eq!(applied.eols, initial.eols);
    assert_eq!(history(source), (1, 0));
    assert!(applied.dirty);
    let rich_source_selection = rich.snapshot().unwrap().selections;
    gate.to_source();
    assert!(!gate.rich_mode.get(), "{}", gate.status.get());
    let source_selection = source.snapshot().unwrap().selections;
    assert_eq!(source_selection.primary, rich_source_selection.primary);
    assert_eq!(source_selection.ranges, rich_source_selection.ranges);
    gate.shell_history(true).unwrap();
    assert_eq!(body(source), *initial);
    assert_eq!(body(rich), *initial);
    assert_eq!(history(source), (0, 1));
    gate.shell_history(false).unwrap();
    assert_eq!(body(source), applied);
    assert_eq!(body(rich), applied);
    assert_eq!(history(source), (1, 0));
}

#[test]
fn new_link_over_plain_selection_keeps_escaped_entity_unicode_original_bytes_and_one_history() {
    run(|| {
        let raw =
            "\u{feff}before\r\n\r\n甲\\* &amp; 🚀 tail\n\n<custom data-a='1'>stay</custom>\r\n";
        let (source, rich, gate, controls) = setup(raw, "甲\\*", Some("🚀"));
        let initial = body(&source);
        let initial_origins = retained_original_bytes(&source);
        // BOM is an explicit profile bit, not a provenance span. Origin bytes
        // still include each original CRLF and all escaped/entity spellings.
        assert!(initial.profile.utf8_bom);
        assert_eq!(
            initial_origins,
            raw.strip_prefix('\u{feff}').unwrap().as_bytes()
        );
        let before = (audit(&source), audit(&rich));
        assert!(
            rich_commands::availability(&gate, &source, "texzi.markdown.insertLink", true).is_ok()
        );
        assert_eq!(
            rich_commands::run(&gate, &source, "texzi.markdown.insertLink", true).unwrap(),
            Outcome::Form(FormRequest::Link)
        );
        assert_eq!((audit(&source), audit(&rich)), before);
        controls.begin(ResourceKind::Link).unwrap();
        assert_eq!(controls.text.get(), "甲* & 🚀");
        controls.url.set("./new?x=1&copy;".into());
        controls.title.set("新标题".into());
        controls.apply().unwrap();
        let expected = raw.replacen(
            "甲\\* &amp; 🚀",
            "[甲\\* &amp; 🚀](<./new?x=1&amp;copy;> \"新标题\")",
            1,
        );
        assert_eq!(retained_original_bytes(&source), initial_origins);
        assert_eq!(initial_origins.len() + '\u{feff}'.len_utf8(), raw.len());
        // Fixed independent positions: `before` occupies a six-unit paragraph
        // (node size eight), then the seven UTF16 units in `甲* & 🚀`.
        let selected = gate.session().state();
        assert_eq!(
            (selected.selection().anchor(), selected.selection().head()),
            (9, 16)
        );
        // Store selections are UTF8 byte offsets in BOM-free/LF-normalized
        // Source. At the shared inline boundary the existing right bias maps
        // the head to the following ` tail`, after the complete link syntax.
        assert_eq!(rich.snapshot().unwrap().selections.ranges, vec![(9, 61)]);
        roundtrip(&source, &rich, &gate, &initial, &expected);
    });
}

#[test]
fn new_links_preserve_whole_nested_and_partial_emphasis_spellings_when_edges_are_split() {
    run(|| {
        for (inline, first, last, expected) in [
            (
                "**bold** and _em_",
                "bold",
                "em",
                "[**bold** and _em_](<./new>)",
            ),
            (
                "**bold** tail",
                "old",
                "tail",
                "**b**[**old** tail](<./new>)",
            ),
            (
                "head **bold**",
                "head",
                "bo",
                "[head **bo**](<./new>)**ld**",
            ),
            (
                "_em **bold** end_",
                "bold",
                "bold",
                "_em [**bold**](<./new>) end_",
            ),
            (
                "__bold__ + ~~gone~~",
                "bold",
                "gone",
                "[__bold__ + ~~gone~~](<./new>)",
            ),
        ] {
            let raw = format!("before\r\n\r\n{inline}\n\n<custom>stay</custom>\r\n");
            let (source, rich, gate, controls) = setup(&raw, first, Some(last));
            let initial = body(&source);
            submit(&controls, "./new", "");
            let expected = raw.replacen(inline, expected, 1);
            roundtrip(&source, &rich, &gate, &initial, &expected);
        }
    });
}

#[test]
fn empty_caret_form_inserts_literal_link_text_in_empty_and_existing_body_without_fake_success() {
    run(|| {
        for (raw, needle, expected) in [
            ("", "", "[新 \\*字\\* &amp; 🚀](<./new> \"T\")"),
            (
                "before after\r\n\r\n<custom>stay</custom>\n",
                "after",
                "before [新 \\*字\\* &amp; 🚀](<./new> \"T\")after\r\n\r\n<custom>stay</custom>\n",
            ),
            (
                "**a b**\n\n<custom>stay</custom>\r\n",
                "b",
                "**a [新 \\*字\\* &amp; 🚀](<./new> \"T\")b**\n\n<custom>stay</custom>\r\n",
            ),
        ] {
            let (source, rich, gate, controls) = setup(raw, needle, None);
            let initial = body(&source);
            controls.begin(ResourceKind::Link).unwrap();
            assert!(controls.empty_selection.get());
            controls.url.set("./new".into());
            controls.title.set("T".into());
            let before = (audit(&source), audit(&rich));
            assert!(
                controls.apply().is_err(),
                "empty label cannot report a creation"
            );
            assert_eq!((audit(&source), audit(&rich)), before);
            controls.text.set("新 *字* & 🚀".into());
            controls.apply().unwrap();
            roundtrip(&source, &rich, &gate, &initial, expected);
        }
    });
}

#[test]
fn insertion_form_cancel_ime_invalid_values_and_stale_revision_have_no_partial_store_commit() {
    run(|| {
        let (source, rich, gate, controls) = setup(
            "before\r\n\r\nplain text\n\n<custom>stay</custom>\r\n",
            "plain",
            Some("text"),
        );
        let before = (audit(&source), audit(&rich));
        source.set_composing(true);
        assert!(controls.begin(ResourceKind::Link).is_err());
        source.set_composing(false);
        assert_eq!((audit(&source), audit(&rich)), before);
        controls.begin(ResourceKind::Link).unwrap();
        controls.url.set("./new".into());
        controls.cancel();
        assert_eq!((audit(&source), audit(&rich)), before);
        controls.begin(ResourceKind::Link).unwrap();
        controls.url.set("./new".into());
        source.set_composing(true);
        assert!(controls.apply().is_err());
        source.set_composing(false);
        assert_eq!((audit(&source), audit(&rich)), before);
        controls.url.set("./new\ninvalid".into());
        assert!(controls.apply().is_err());
        assert_eq!((audit(&source), audit(&rich)), before);
        controls.url.set("javascript:alert(1)".into());
        assert!(controls.apply().is_err());
        assert_eq!((audit(&source), audit(&rich)), before);
        controls.url.set("./new".into());
        let old = source.snapshot().unwrap();
        batch(|| {
            source
                .edit(
                    old.revision,
                    old.view_epoch,
                    vec![(0..1, "B".into())],
                    ExternalSelections {
                        primary: 0,
                        ranges: vec![(1, 1)],
                    },
                    ExternalEditKind::Other,
                )
                .unwrap();
            source.notify();
        });
        let stale = (audit(&source), audit(&rich));
        assert!(controls.apply().is_err());
        assert_eq!((audit(&source), audit(&rich)), stale);
        assert_eq!(history(&source), (1, 0));
        controls.cancel();
        assert!(gate.rich_mode.get());
    });
}

#[test]
fn existing_reference_selected_label_still_edits_only_its_definition_not_same_href_inline_link() {
    run(|| {
        let raw = "[one][shared] and [two][shared] and [inline](./old)\r\n\r\n[shared]: <./old> \"keep\"\n[other]: ./old\r\n\r\n<custom>stay</custom>\n";
        let (source, rich, gate, controls) = setup(raw, "one", Some("one"));
        let initial = body(&source);
        controls.begin(ResourceKind::Link).unwrap();
        assert!(!controls.creating.get());
        assert!(controls.reference.get());
        controls.url.set("./new".into());
        controls.title.set("新标题".into());
        controls.apply().unwrap();
        let expected = raw.replacen(
            "[shared]: <./old> \"keep\"",
            "[shared]: <./new> \"新标题\"",
            1,
        );
        roundtrip(&source, &rich, &gate, &initial, &expected);
    });
}

#[test]
fn insertion_rejects_cross_block_and_overlapping_existing_link_instead_of_editing_one_endpoint() {
    run(|| {
        for (raw, first, last) in [
            (
                "first\n\nsecond\n\n<custom>stay</custom>",
                "first",
                "second",
            ),
            (
                "plain [label](./old)\n\n<custom>stay</custom>",
                "plain",
                "label",
            ),
            ("`code` text\n\n<custom>stay</custom>", "code", "text"),
        ] {
            let (source, rich, gate, controls) = setup(raw, first, Some(last));
            let before = (audit(&source), audit(&rich));
            assert!(controls.begin(ResourceKind::Link).is_err());
            assert!(rich_commands::run(&gate, &source, "texzi.markdown.insertLink", true).is_err());
            assert!(!controls.open.get());
            assert_eq!((audit(&source), audit(&rich)), before);
        }
    });
}
