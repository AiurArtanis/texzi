//! Owner regressions for the old heading-target sequence. These fixtures do
//! not reproduce a pointer gesture or claim the old GUI target was correct.
use crate::{editor_owner::NativeView, rich_owner::ModeGate, rich_projection::Projection};
use nsh_base::{
    editor::ExternalDocument,
    rich_editor::state::{KeyEvent, TextSelection, Transaction, TransactionOrigin},
};
use std::rc::Rc;

const NEIGHBOR: &str = "a &amp; b \\*转义\\*";
const ORIGINAL: &str = "# 中文标题\na &amp; b \\*转义\\*";

#[derive(Debug, PartialEq, Eq)]
struct ExactBody {
    bytes: Vec<u8>,
    token: texzi_document::StateToken,
    eols: Vec<texzi_document::Eol>,
    profile: texzi_document::Profile,
    origins: String,
    dirty: bool,
}
fn exact(source: &NativeView) -> ExactBody {
    source.inspect(|store, view| {
        let doc = store.view(view).unwrap().doc;
        let s = store.snapshot(doc).unwrap();
        ExactBody {
            bytes: s.body.to_utf8(),
            token: s.token,
            eols: s.body.eols(),
            profile: s.body.profile,
            origins: format!("{:?}", s.body.origins()),
            dirty: store.dirty(doc).unwrap(),
        }
    })
}
fn depth(source: &NativeView) -> (usize, usize) {
    source.inspect(|store, view| store.history_depth(store.view(view).unwrap().doc).unwrap())
}
fn setup(raw: &str) -> (Rc<NativeView>, Rc<NativeView>, Rc<ModeGate>) {
    let source = NativeView::fixture(raw);
    let rich = source.sibling().unwrap();
    let gate = ModeGate::new(source.clone(), rich.clone());
    gate.to_rich();
    assert!(gate.rich_mode.get(), "prepare rich: {}", gate.status.get());
    (source, rich, gate)
}
fn check(
    source: &NativeView,
    rich: &NativeView,
    gate: &ModeGate,
    step: &str,
    raw: &str,
    position: usize,
    source_byte: usize,
) {
    let state = gate.session().state();
    let native = rich.snapshot().unwrap();
    let actual = source.snapshot().unwrap();
    eprintln!(
        "heading step={step} status={:?} revision={} rich_selection={:?} source_selection={:?} history={:?} source={:?}",
        gate.status.get(),
        actual.revision,
        state.selection().to_json(),
        native.selections,
        depth(source),
        actual.rope.to_string()
    );
    assert!(
        gate.status.get().is_empty(),
        "{step}: Owner rejected: {}",
        gate.status.get()
    );
    assert_eq!(
        exact(source).bytes,
        raw.as_bytes(),
        "{step}: authoritative source bytes"
    );
    assert!(
        exact(source).bytes.ends_with(NEIGHBOR.as_bytes()),
        "{step}: entity/escape neighbor bytes must be retained"
    );
    assert_eq!(
        (state.selection().anchor(), state.selection().head()),
        (position, position),
        "{step}: actual rich selection"
    );
    let p = Projection::parse(state.schema(), actual.rope.to_string()).unwrap();
    assert_eq!(
        &p.doc,
        state.doc(),
        "{step}: current session matches authoritative source projection"
    );
    assert_eq!(
        p.source_position(position),
        Some(source_byte),
        "{step}: exact rich/source coordinate"
    );
    assert_eq!(
        native.selections.ranges,
        vec![(source_byte, source_byte)],
        "{step}: actual rich NativeView selection"
    );
    assert_eq!(native.revision, actual.revision);
}
fn select(
    source: &NativeView,
    rich: &NativeView,
    gate: &ModeGate,
    position: usize,
    source_byte: usize,
) {
    let before = exact(source);
    let revision = source.snapshot().unwrap().revision;
    let history = depth(source);
    let state = gate.session().state();
    let mut tr = state.tr();
    tr.set_selection(TextSelection::create(state.doc(), position, None));
    gate.session().dispatch(tr).unwrap();
    assert!(
        gate.status.get().is_empty(),
        "select {position}: {}",
        gate.status.get()
    );
    assert_eq!(exact(source), before);
    assert_eq!(source.snapshot().unwrap().revision, revision);
    assert_eq!(depth(source), history);
    assert_eq!(
        rich.snapshot().unwrap().selections.ranges,
        vec![(source_byte, source_byte)]
    );
}
fn publish(source: &NativeView, gate: &ModeGate, step: &str, tr: Transaction, revision_delta: u64) {
    assert!(
        gate.status.get().is_empty(),
        "{step}: previous Owner error: {}",
        gate.status.get()
    );
    let before = source.snapshot().unwrap();
    let state = gate.session().state();
    let projection = Projection::parse(state.schema(), before.rope.to_string()).unwrap();
    let planned = projection.transaction_edits(tr.doc(), std::slice::from_ref(&tr));
    eprintln!(
        "heading attempt step={step} from={:?} to={:?} inputRule={:?} source_patch={planned:?}",
        state.selection().to_json(),
        tr.selection().to_json(),
        tr.get_meta("inputRule$")
    );
    // Always invoke the real Owner, even when this read-only diagnostic sees
    // a bridge failure. A dispatcher Ok alone cannot count as acceptance.
    gate.session().dispatch(tr).unwrap();
    eprintln!(
        "heading receipt step={step} owner_status={:?} revision={} history={:?}",
        gate.status.get(),
        source.snapshot().unwrap().revision,
        depth(source)
    );
    assert!(
        gate.status.get().is_empty(),
        "{step}: actual Owner rejection: {}",
        gate.status.get()
    );
    assert_eq!(
        source.snapshot().unwrap().revision,
        before.revision + revision_delta,
        "{step}: only the expected Store transactions may publish"
    );
    let neighbor = projection.source.find(NEIGHBOR).unwrap();
    for (range, _) in planned.unwrap_or_else(|error| {
        panic!("{step}: source plan failed although Owner accepted: {error}")
    }) {
        assert!(
            range.end <= neighbor,
            "{step}: source patch {range:?} crossed unchanged entity/escape paragraph at {neighbor}"
        );
    }
}
fn input(source: &NativeView, gate: &ModeGate, step: &str, text: &str, revision_delta: u64) {
    let state = gate.session().state();
    let (from, to) = (state.selection().from(), state.selection().to());
    let mut made = None;
    let handled = state
        .plugins()
        .iter()
        .any(|plugin| plugin.handle_text_input(&state, from, to, text, &mut |tr| made = Some(tr)));
    let tr = if handled {
        made.expect("installed input rule handled without a transaction")
    } else {
        let mut tr = state.tr();
        tr.insert_text(text, Some(from), Some(to));
        tr.set_origin(TransactionOrigin::Input);
        tr
    };
    publish(source, gate, step, tr, revision_delta);
}
fn enter(source: &NativeView, gate: &ModeGate) {
    let state = gate.session().state();
    let event = KeyEvent::new("Enter");
    let mut made = None;
    let handled = state
        .plugins()
        .iter()
        .any(|plugin| plugin.handle_key_down(&state, &event, &mut |tr| made = Some(tr)));
    assert!(handled, "installed Enter was not handled");
    publish(
        source,
        gate,
        "installed Enter",
        made.expect("installed Enter handled without a transaction"),
        1,
    );
}
fn assert_format_undo(
    source: &NativeView,
    rich: &NativeView,
    gate: &ModeGate,
    typed: &str,
    typed_position: usize,
    typed_byte: usize,
    formatted: &str,
    position: usize,
    source_byte: usize,
) {
    let converted = exact(source);
    let converted_depth = depth(source);
    let revision = source.snapshot().unwrap().revision;
    gate.shell_history(true).unwrap();
    check(
        source,
        rich,
        gate,
        "Undo formatting restores accepted trigger-space input",
        typed,
        typed_position,
        typed_byte,
    );
    assert_eq!(source.snapshot().unwrap().revision, revision + 1);
    assert_eq!(depth(source), (converted_depth.0 - 1, 1));
    let actual_typed = exact(source);
    gate.shell_history(false).unwrap();
    check(
        source,
        rich,
        gate,
        "Redo formatting",
        formatted,
        position,
        source_byte,
    );
    assert_eq!(
        exact(source),
        converted,
        "Redo restores exact converted token/profile/EOL/provenance/dirty"
    );
    assert_eq!(depth(source), converted_depth);
    gate.to_source();
    assert!(!gate.rich_mode.get());
    gate.shell_history(true).unwrap();
    assert_eq!(
        exact(source),
        actual_typed,
        "Source Undo reaches the same accepted typing state"
    );
    assert_eq!(
        source.snapshot().unwrap().selections.ranges,
        vec![(typed_byte, typed_byte)]
    );
    gate.shell_history(false).unwrap();
    assert_eq!(
        exact(source),
        converted,
        "Source Redo uses the same history"
    );
}

#[test]
fn old_heading_target_sequence_types_each_hash_inside_the_right_heading_fragment() {
    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    assert_eq!(
        ORIGINAL.len(),
        35,
        "single backslash before each literal star in the original fixture"
    );
    let (source, rich, gate) = setup(ORIGINAL);
    let p = Projection::parse(
        gate.session().state().schema(),
        source.snapshot().unwrap().rope.to_string(),
    )
    .unwrap();
    assert_eq!(p.source_position(1), Some(2));
    assert_eq!(p.source_position(5), Some(14));
    assert_eq!(p.source_position(17), Some(35));
    select(&source, &rich, &gate, 1, 2);
    input(&source, &gate, "乙 at original heading start", "乙", 1);
    check(
        &source,
        &rich,
        &gate,
        "乙",
        "# 乙中文标题\na &amp; b \\*转义\\*",
        2,
        5,
    );
    enter(&source, &gate);
    check(
        &source,
        &rich,
        &gate,
        "split heading after 乙",
        "# 乙\n\n# 中文标题\na &amp; b \\*转义\\*",
        4,
        9,
    );
    assert_eq!(
        gate.session().state().doc().child(1).ty().name(),
        "heading",
        "the right fragment remains a heading, not a paragraph"
    );
    for (step, raw, position, source_byte) in [
        ("hash 1", "# 乙\n\n# #中文标题\na &amp; b \\*转义\\*", 5, 10),
        (
            "hash 2",
            "# 乙\n\n# ##中文标题\na &amp; b \\*转义\\*",
            6,
            11,
        ),
        (
            "hash 3",
            "# 乙\n\n# ###中文标题\na &amp; b \\*转义\\*",
            7,
            12,
        ),
    ] {
        input(&source, &gate, step, "#", 1);
        check(&source, &rich, &gate, step, raw, position, source_byte);
    }
    input(
        &source,
        &gate,
        "trigger space inside existing heading",
        " ",
        2,
    );
    let formatted = "# 乙\n\n### 中文标题\na &amp; b \\*转义\\*";
    check(
        &source,
        &rich,
        &gate,
        "converted existing heading",
        formatted,
        4,
        11,
    );
    assert_eq!(
        gate.session().state().doc().child(1).attrs()["level"],
        serde_json::json!(3)
    );
    assert_format_undo(
        &source,
        &rich,
        &gate,
        "# 乙\n\n# ### 中文标题\na &amp; b \\*转义\\*",
        8,
        13,
        formatted,
        4,
        11,
    );
}

#[test]
fn paragraph_heading_input_control_keeps_entity_neighbor_and_separate_format_undo() {
    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    let raw = "# 中文标题\n\n\n\na &amp; b \\*转义\\*";
    let (source, rich, gate) = setup(raw);
    select(&source, &rich, &gate, 7, 16);
    for (step, raw, position, source_byte) in [
        (
            "paragraph hash 1",
            "# 中文标题\n\n\\#\n\na &amp; b \\*转义\\*",
            8,
            18,
        ),
        (
            "paragraph hash 2",
            "# 中文标题\n\n\\##\n\na &amp; b \\*转义\\*",
            9,
            19,
        ),
        (
            "paragraph hash 3",
            "# 中文标题\n\n\\###\n\na &amp; b \\*转义\\*",
            10,
            20,
        ),
    ] {
        input(&source, &gate, step, "#", 1);
        check(&source, &rich, &gate, step, raw, position, source_byte);
    }
    input(&source, &gate, "paragraph trigger space", " ", 2);
    let formatted = "# 中文标题\n\n### \n\na &amp; b \\*转义\\*";
    check(
        &source,
        &rich,
        &gate,
        "paragraph converted",
        formatted,
        7,
        20,
    );
    assert_eq!(
        gate.session().state().doc().child(1).attrs()["level"],
        serde_json::json!(3)
    );
    assert_format_undo(
        &source,
        &rich,
        &gate,
        "# 中文标题\n\n\\###&#32;\n\na &amp; b \\*转义\\*",
        11,
        25,
        formatted,
        7,
        20,
    );
}

#[test]
fn enter_at_chinese_heading_end_creates_room_for_h3_before_entity_paragraph() {
    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    let (source, rich, gate) = setup(ORIGINAL);
    select(&source, &rich, &gate, 5, 14);
    enter(&source, &gate);
    let initial_gap = "# 中文标题\n\n\n\na &amp; b \\*转义\\*";
    check(
        &source,
        &rich,
        &gate,
        "empty paragraph below H1",
        initial_gap,
        7,
        16,
    );
    for _ in 0..3 {
        input(&source, &gate, "hash in new paragraph", "#", 1);
    }
    input(&source, &gate, "heading trigger", " ", 2);
    input(&source, &gate, "H3 title", "新标题", 1);
    let raw = "# 中文标题\n\n### 新标题\n\na &amp; b \\*转义\\*";
    check(&source, &rich, &gate, "H3 below H1", raw, 10, 29);
    assert_eq!(
        gate.session().state().doc().child(1).attrs()["level"],
        serde_json::json!(3)
    );
}
