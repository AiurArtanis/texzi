//! Shared toolbar/palette forms exercise the real controllers, live RichOwner
//! and one Store. These are headless component tests, not GUI coverage.
use super::*;
use crate::{
    rich_commands::{self, FormRequest, Outcome},
    rich_projection::Projection,
    table_controls::TableAction,
};
use nsh_base::rich_editor::state::TextSelection;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExactBody {
    token: texzi_document::StateToken,
    bytes: Vec<u8>,
    logical: String,
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

fn body(view: &NativeView) -> ExactBody {
    view.inspect(|store, id| {
        let doc = store.view(id).unwrap().doc;
        let snapshot = store.snapshot(doc).unwrap();
        ExactBody {
            token: snapshot.token,
            bytes: snapshot.body.to_utf8(),
            logical: snapshot.body.source().to_string(),
            profile: snapshot.body.profile,
            eols: snapshot.body.eols(),
            origins: format!("{:?}", snapshot.body.origins()),
            dirty: store.dirty(doc).unwrap(),
        }
    })
}

fn history(view: &NativeView) -> (usize, usize) {
    view.inspect(|store, id| store.history_depth(store.view(id).unwrap().doc).unwrap())
}

fn audit(view: &NativeView) -> String {
    view.inspect(|store, id| {
        let state = store.view(id).unwrap();
        let snapshot = store.snapshot(state.doc).unwrap();
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            state,
            snapshot.doc,
            snapshot.revision,
            snapshot.token,
            snapshot.body.to_utf8(),
            snapshot.body.profile,
            snapshot.body.eols(),
            snapshot.body.origins(),
            (store.history_depth(state.doc), store.dirty(state.doc)),
            store.last_fence(state.doc)
        )
    })
}

fn audits(source: &NativeView, rich: &NativeView) -> (String, String) {
    (audit(source), audit(rich))
}

fn setup(
    raw: &str,
    needle: &str,
) -> (Rc<NativeView>, Rc<NativeView>, Rc<ModeGate>, RichControlSet) {
    let source = NativeView::fixture(raw);
    let rich = source.sibling().unwrap();
    let gate = ModeGate::new(source.clone(), rich.clone());
    gate.to_rich();
    assert!(gate.rich_mode.get(), "{}", gate.status.get());
    let session = gate.session();
    let state = session.state();
    let logical = source.snapshot().unwrap().rope.to_string();
    let projection = Projection::parse(state.schema(), logical.clone()).unwrap();
    let position = projection
        .rich_position(logical.find(needle).unwrap())
        .unwrap();
    let mut transaction = state.tr();
    transaction.set_selection(TextSelection::create(state.doc(), position, None));
    session.dispatch(transaction).unwrap();
    assert!(gate.status.get().is_empty(), "{}", gate.status.get());
    let controls = RichControlSet::new(&gate, &source);
    (source, rich, gate, controls)
}

fn form_request(
    gate: &ModeGate,
    source: &NativeView,
    rich: &NativeView,
    id: &str,
    expected: FormRequest,
) -> FormRequest {
    let before = audits(source, rich);
    let state = gate.session().state();
    let doc = state.doc().clone();
    let selection = state.selection().to_json();
    assert_eq!(
        rich_commands::run(gate, source, id, true).unwrap(),
        Outcome::Form(expected)
    );
    assert_eq!(audits(source, rich), before, "routing {id} must not write");
    assert_eq!(gate.session().state().doc(), &doc);
    assert_eq!(gate.session().state().selection().to_json(), selection);
    expected
}

fn one_transaction_roundtrip(
    gate: &ModeGate,
    source: &NativeView,
    rich: &NativeView,
    initial: &ExactBody,
    expected: &str,
) {
    let applied = body(source);
    assert_eq!(applied.bytes, expected.as_bytes());
    assert_eq!(applied, body(rich));
    assert_ne!(applied.token, initial.token);
    assert_eq!(applied.profile, initial.profile);
    assert_eq!(applied.eols, initial.eols);
    assert!(applied.dirty);
    assert_eq!(history(source), (1, 0));
    gate.to_source();
    assert!(!gate.rich_mode.get(), "{}", gate.status.get());
    gate.shell_history(true).unwrap();
    assert_eq!(body(source), *initial, "one Undo restores the exact body");
    assert_eq!(body(rich), *initial);
    assert_eq!(history(source), (0, 1));
    gate.shell_history(false).unwrap();
    assert_eq!(body(source), applied, "one Redo restores the exact body");
    assert_eq!(body(rich), applied);
    assert_eq!(history(source), (1, 0));
}

#[test]
fn shared_code_language_begin_edit_draft_cancel_has_zero_store_history_or_selection_writes() {
    run(|| {
        let raw = "before\r\n\r\n~~~~  rust  linenums=1 extra=keep\r\nlet 中文 = 1;\n~~~~  \r\n\r\n<custom>stay</custom>\n";
        let (source, rich, gate, controls) = setup(raw, "let 中文");
        let before = audits(&source, &rich);
        let doc = gate.session().state().doc().clone();
        let selection = gate.session().state().selection().to_json();
        controls.begin(FormRequest::CodeLanguage).unwrap();
        assert!(controls.open() && controls.code.open.get());
        assert!(!controls.resources.open.get() && !controls.table.is_open());
        assert_eq!(controls.code.language.get(), "rust");
        assert_eq!(controls.code.info.get(), "linenums=1 extra=keep");
        controls.code.language.set("json".into());
        assert_eq!(audits(&source, &rich), before);
        controls.cancel();
        assert!(!controls.open());
        assert!(
            controls.code.apply_language().is_err(),
            "cancel discards the draft"
        );
        assert_eq!(audits(&source, &rich), before);
        assert_eq!(gate.session().state().doc(), &doc);
        assert_eq!(gate.session().state().selection().to_json(), selection);
        assert_eq!(history(&source), (0, 0));
    });
}

#[test]
fn palette_code_language_form_reuses_toolbar_controller_and_only_patches_language_token() {
    run(|| {
        let raw = "\u{feff}# before\r\n\r\n~~~~  rust  linenums=1 extra=keep\r\nlet 中文 = 1;\n~~~~  \r\n\r\n<custom>stay</custom>\n";
        let (source, rich, gate, controls) = setup(raw, "let 中文");
        let initial = body(&source);
        let toolbar_controller = controls.code.clone();
        let before = audits(&source, &rich);
        toolbar_controller.begin().unwrap();
        toolbar_controller.language.set("discarded".into());
        controls.cancel();
        assert_eq!(audits(&source, &rich), before);
        let request = form_request(
            &gate,
            &source,
            &rich,
            "texzi.markdown.setCodeLanguage",
            FormRequest::CodeLanguage,
        );
        controls.begin(request).unwrap();
        assert!(Rc::ptr_eq(&controls.code, &toolbar_controller));
        assert!(toolbar_controller.open.get());
        assert_eq!(toolbar_controller.language.get(), "rust");
        assert_eq!(toolbar_controller.info.get(), "linenums=1 extra=keep");
        assert_eq!(audits(&source, &rich), before);
        toolbar_controller.language.set("json".into());
        toolbar_controller.apply_language().unwrap();
        assert!(!controls.open());
        let expected = raw.replacen("~~~~  rust  ", "~~~~  json  ", 1);
        one_transaction_roundtrip(&gate, &source, &rich, &initial, &expected);
    });
}

#[test]
fn palette_link_form_cancel_and_apply_use_the_same_resource_controller_and_store_history() {
    run(|| {
        let raw = "before\r\n\r\n[**la &amp; bel**](<https://old.invalid/a> \"keep\")\n\n<custom>stay</custom>\r\n";
        let (source, rich, gate, controls) = setup(raw, "la &amp;");
        let initial = body(&source);
        let toolbar_controller = controls.resources.clone();
        let before = audits(&source, &rich);
        controls.begin(FormRequest::Link).unwrap();
        assert!(controls.open() && toolbar_controller.open.get());
        assert!(!controls.code.open.get() && !controls.table.is_open());
        toolbar_controller.url.set("./discarded".into());
        toolbar_controller.title.set("取消不写入".into());
        controls.cancel();
        assert!(!controls.open());
        assert_eq!(audits(&source, &rich), before);
        let request = form_request(
            &gate,
            &source,
            &rich,
            "texzi.markdown.insertLink",
            FormRequest::Link,
        );
        controls.begin(request).unwrap();
        assert!(Rc::ptr_eq(&controls.resources, &toolbar_controller));
        assert_eq!(toolbar_controller.url.get(), "https://old.invalid/a");
        assert_eq!(toolbar_controller.title.get(), "keep");
        assert_eq!(audits(&source, &rich), before);
        toolbar_controller
            .url
            .set("https://new.invalid/path".into());
        toolbar_controller.title.set("新标题".into());
        toolbar_controller.apply().unwrap();
        assert!(!controls.open());
        let expected = raw.replacen(
            "<https://old.invalid/a> \"keep\"",
            "<https://new.invalid/path> \"新标题\"",
            1,
        );
        one_transaction_roundtrip(&gate, &source, &rich, &initial, &expected);
    });
}

#[test]
fn shared_table_forms_route_without_writes_cancel_and_apply_via_the_same_table_controller() {
    run(|| {
        let raw = "before\r\n\r\n| key | value |\r\n| --- | --- |\n| one | two |\r\n\r\n<custom>stay</custom>\n";
        let (source, rich, gate, controls) = setup(raw, "two");
        let initial = body(&source);
        let toolbar_controller = controls.table.clone();
        for (id, expected) in [(
            "texzi.markdown.table.alignColumn",
            FormRequest::TableAlignment,
        )] {
            let before = audits(&source, &rich);
            let request = form_request(&gate, &source, &rich, id, expected);
            controls.begin(request).unwrap();
            assert!(controls.open() && toolbar_controller.is_open());
            assert!(!controls.resources.open.get() && !controls.code.open.get());
            assert!(Rc::ptr_eq(&controls.table, &toolbar_controller));
            assert_eq!(audits(&source, &rich), before);
            controls.cancel();
            assert!(!controls.open());
            assert_eq!(audits(&source, &rich), before);
        }
        controls.begin(FormRequest::TableAlignment).unwrap();
        toolbar_controller.apply(TableAction::AlignCenter).unwrap();
        assert!(!controls.open());
        let expected = raw.replacen("| --- | --- |", "| --- | :---: |", 1);
        one_transaction_roundtrip(&gate, &source, &rich, &initial, &expected);
    });
}
