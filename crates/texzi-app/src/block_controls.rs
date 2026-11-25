//! Small native paragraph-style picker; all body edits use the existing Owner.
use crate::{editor_owner::NativeView, rich_owner::ModeGate, rich_projection::Projection};
use nsh_base::{
    editor::ExternalDocument,
    rich_editor::{
        model::Node,
        state::{EditorState, Transaction, TransactionOrigin},
        view::EditorSession,
    },
};
use nsh_core::{element::view as box_view, prelude::*};
use std::{cell::RefCell, rc::Rc};

struct Draft {
    revision: u64,
    state_revision: u64,
    session: EditorSession,
    doc: Node,
    position: usize,
}
pub struct BlockControls {
    gate: Rc<ModeGate>,
    source: Rc<NativeView>,
    draft: RefCell<Option<Draft>>,
    pub open: Signal<bool>,
    pub current: Signal<Option<u8>>,
    pub info: Signal<String>,
}
impl BlockControls {
    pub fn new(gate: Rc<ModeGate>, source: Rc<NativeView>) -> Rc<Self> {
        Rc::new(Self {
            gate,
            source,
            draft: RefCell::new(None),
            open: signal(false),
            current: signal(None),
            info: signal(String::new()),
        })
    }
    fn ready(&self) -> Result<(u64, Projection, EditorState, usize), String> {
        if self.gate.busy() {
            return Err("请先完成或取消中文组合，再修改段落样式".into());
        }
        if !self.gate.rich_mode.get() {
            return Err("请先切换到正文模式".into());
        }
        let snapshot = self.source.snapshot()?;
        let state = self.gate.session().state();
        let p = Projection::parse(state.schema(), snapshot.rope.to_string())?;
        if &p.doc != state.doc() {
            return Err("正文版本已变化，请重新打开段落样式".into());
        }
        let from = state.selection().resolved_from();
        let to = state.selection().resolved_to();
        if from.depth == 0
            || !from.same_parent(to)
            || !matches!(from.parent().ty().name(), "heading" | "paragraph")
        {
            return Err("请把光标或选区放在同一个独立段落或标题中".into());
        }
        let position = from.before(from.depth);
        let mut base = 0;
        let supported = p.blocks.iter().any(|block| {
            let at = base;
            base += block.node.node_size();
            at == position && !block.opaque && &block.node == from.parent()
        });
        if !supported {
            return Err("此嵌套块的段落样式请在源码中修改".into());
        }
        Ok((snapshot.revision, p, state, position))
    }
    pub fn begin(&self) -> Result<(), String> {
        let (revision, p, state, position) = self.ready()?;
        let node = state.doc().node_at(position).ok_or("当前段落已变化")?;
        self.current.set(
            node.attrs()
                .get("level")
                .and_then(|v| v.as_u64())
                .map(|v| v as u8),
        );
        let mut base = 0;
        let setext = p.blocks.iter().any(|block| {
            let at = base;
            base += block.node.node_size();
            let first = p.source[block.source.clone()]
                .split('\n')
                .next()
                .unwrap_or("");
            let first = first.trim_start_matches(' ');
            let hashes = first.bytes().take_while(|b| *b == b'#').count();
            let atx = (1..=6).contains(&hashes)
                && first
                    .as_bytes()
                    .get(hashes)
                    .is_none_or(|b| matches!(b, b' ' | b'\t'));
            at == position && node.ty().name() == "heading" && !atx
        });
        self.info.set(if setext {
            "此标题采用下划线形式；选择标题 3–6 会改用 # 形式，正文保持原样。".into()
        } else {
            String::new()
        });
        *self.draft.borrow_mut() = Some(Draft {
            revision,
            state_revision: state.revision(),
            session: self.gate.session(),
            doc: state.doc().clone(),
            position,
        });
        self.gate.status.set(String::new());
        self.open.set(true);
        Ok(())
    }
    pub fn cancel(&self) {
        self.draft.borrow_mut().take();
        self.open.set(false);
    }
    fn transaction(
        state: &EditorState,
        position: usize,
        level: Option<u8>,
    ) -> Result<Transaction, String> {
        let old = state.doc().node_at(position).ok_or("当前段落已变化")?;
        let mut tr = state.tr();
        if old.ty().name() == "heading" && level.is_some() {
            tr.set_node_attribute(position, "level", serde_json::json!(level.unwrap()))
                .map_err(|e| e.to_string())?;
        } else {
            let ty = state
                .schema()
                .node_type(if level.is_some() {
                    "heading"
                } else {
                    "paragraph"
                })
                .ok_or("段落样式尚未启用")?;
            let mut attrs = old.attrs().clone();
            attrs.remove("level");
            if let Some(level) = level {
                attrs.insert("level".into(), serde_json::json!(level));
            }
            tr.set_node_markup(position, Some(ty), Some(attrs), None)
                .map_err(|e| e.to_string())?;
        }
        tr.set_origin(TransactionOrigin::User);
        Ok(tr)
    }
    pub fn apply_level(&self, level: Option<u8>) -> Result<(), String> {
        if level.is_some_and(|n| !(1..=6).contains(&n)) {
            return Err("标题级别必须为 H1–H6".into());
        }
        let (revision, p, state, position) = self.ready()?;
        let guard = self.draft.borrow();
        let draft = guard.as_ref().ok_or("请先打开段落样式")?;
        if revision != draft.revision
            || state.revision() != draft.state_revision
            || state.doc() != &draft.doc
            || position != draft.position
            || self.gate.session() != draft.session
        {
            return Err("段落样式的原选区已过期，请取消后重新打开".into());
        }
        let actual = state
            .doc()
            .node_at(position)
            .ok_or("当前段落已变化")?
            .attrs()
            .get("level")
            .and_then(|v| v.as_u64())
            .map(|v| v as u8);
        if actual != level {
            // Reject an unrepresentable style before dispatching anything to
            // Owner/Store. Existing source-seam and raw-block guards stay intact.
            let preview = Self::transaction(&state, position, level)?;
            let (next, applied) = state
                .apply_transaction(preview)
                .map_err(|e| e.to_string())?;
            let edits = p.transaction_edits(next.doc(), &applied)?;
            let mut raw = p.source.clone();
            for (range, text) in edits.iter().rev() {
                raw.replace_range(range.clone(), text);
            }
            if Projection::parse(state.schema(), raw)?.doc != *next.doc() {
                return Err("此段落样式暂不能保真往返，请在源码中修改".into());
            }
            self.gate.status.set(String::new());
            self.gate
                .session()
                .dispatch(Self::transaction(&state, position, level)?)
                .map_err(|e| e.to_string())?;
            let status = self.gate.status.get();
            if !status.is_empty() {
                return Err(status);
            }
        }
        drop(guard);
        self.current.set(level);
        self.cancel();
        Ok(())
    }
    fn report(&self, result: Result<(), String>) {
        if let Err(error) = result {
            self.gate.status.set(error);
        }
    }
}

pub fn dialog(controls: Rc<BlockControls>, focus: Rc<dyn Fn()>) -> impl IntoElement {
    let modal = controls;
    dynamic(move || {
        if !modal.open.get() {
            return None;
        }
        let dismiss = modal.clone();
        let dismiss_focus = focus.clone();
        let cancel = modal.clone();
        let cancel_focus = focus.clone();
        let mut choices = box_view().gap(px(4.0));
        for n in 0..=6 {
            let level = (n != 0).then_some(n);
            let choose = modal.clone();
            let choose_focus = focus.clone();
            let label = if n == 0 {
                "正文 (P)".into()
            } else {
                format!("标题 {n} (H{n})")
            };
            choices = choices.child(nsh_base::button().child(text(label)).on_press(
                move |_: &()| {
                    let result = choose.apply_level(level);
                    let done = result.is_ok();
                    choose.report(result);
                    if done {
                        choose_focus();
                    }
                },
            ));
        }
        let status = modal.gate.status;
        let info = modal.info;
        Some(
            nsh_base::dialog()
                .label("段落样式")
                .on_cancel(move |_: &()| {
                    dismiss.cancel();
                    dismiss_focus();
                })
                .child(
                    box_view()
                        .w(px(240.0))
                        .p(px(12.0))
                        .gap(px(8.0))
                        .bg(crate::app_shell::background)
                        .text_color(crate::app_shell::foreground)
                        .child(text("段落样式"))
                        .child(text(move || info.get()))
                        .child(choices)
                        .child(text(move || status.get()))
                        .child(
                            nsh_base::button()
                                .child(text("取消"))
                                .on_press(move |_: &()| {
                                    cancel.cancel();
                                    cancel_focus();
                                }),
                        ),
                )
                .into_element(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_base::{
        editor::{ExternalEditKind, ExternalSelections},
        rich_editor::state::TextSelection,
    };

    #[derive(Debug, PartialEq, Eq)]
    struct Identity {
        bytes: Vec<u8>,
        token: texzi_document::StateToken,
        eols: Vec<texzi_document::Eol>,
        origins: String,
        dirty: bool,
    }
    fn identity(source: &NativeView) -> Identity {
        source.inspect(|store, id| {
            let s = store.snapshot(store.view(id).unwrap().doc).unwrap();
            Identity {
                bytes: s.body.to_utf8(),
                token: s.token,
                eols: s.body.eols(),
                origins: format!("{:?}", s.body.origins()),
                dirty: store.dirty(s.doc).unwrap(),
            }
        })
    }
    fn audit(view: &NativeView) -> String {
        view.inspect(|store, id| {
            let v = store.view(id).unwrap();
            let s = store.snapshot(v.doc).unwrap();
            format!(
                "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                v,
                s.revision,
                s.token,
                s.body.to_utf8(),
                s.body.origins(),
                store.history_depth(v.doc).unwrap(),
                store.dirty(v.doc).unwrap()
            )
        })
    }
    fn runtime(f: impl FnOnce()) {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        f();
    }
    fn fixture(
        raw: &str,
    ) -> (
        Rc<NativeView>,
        Rc<NativeView>,
        Rc<ModeGate>,
        Rc<BlockControls>,
    ) {
        let source = NativeView::fixture(raw);
        let rich = source.sibling().unwrap();
        let gate = ModeGate::new(source.clone(), rich.clone());
        gate.to_rich();
        assert!(gate.rich_mode.get(), "{}", gate.status.get());
        let controls = BlockControls::new(gate.clone(), source.clone());
        (source, rich, gate, controls)
    }
    fn select(source: &NativeView, gate: &ModeGate, needle: &str) {
        let state = gate.session().state();
        let raw = source.snapshot().unwrap().rope.to_string();
        let byte = raw.find(needle).unwrap();
        let p = Projection::parse(state.schema(), raw).unwrap();
        let mut tr = state.tr();
        tr.set_selection(TextSelection::create(
            state.doc(),
            p.rich_position(byte).unwrap(),
            None,
        ));
        gate.session().dispatch(tr).unwrap();
        assert!(gate.status.get().is_empty(), "{}", gate.status.get());
    }
    fn depth(source: &NativeView) -> (usize, usize) {
        source.inspect(|s, id| s.history_depth(s.view(id).unwrap().doc).unwrap())
    }
    fn retained_body(source: &NativeView, raw: &str, text: &str) {
        let start = raw.find(text).unwrap();
        source.inspect(|store, id| {
            let s = store.snapshot(store.view(id).unwrap().doc).unwrap();
            let origins: Vec<_> = s
                .body
                .origins()
                .into_iter()
                .filter_map(|(_, origin)| origin)
                .collect();
            for byte in start..start + text.len() {
                assert!(
                    origins
                        .iter()
                        .any(|o| o.source.as_ref() == raw && o.raw.contains(&byte)),
                    "unchanged heading-body byte {byte} must keep its original provenance"
                );
            }
        });
    }
    fn roundtrip(source: &NativeView, gate: &ModeGate, original: &Identity, expected: &str) {
        let edited = identity(source);
        assert_eq!(edited.bytes, expected.as_bytes());
        assert_eq!(depth(source), (1, 0));
        gate.to_source();
        assert!(!gate.rich_mode.get(), "{}", gate.status.get());
        gate.source_mounted();
        let revision = source.snapshot().unwrap().revision;
        gate.shell_history(true).unwrap();
        assert_eq!(&identity(source), original);
        assert_eq!(source.snapshot().unwrap().revision, revision + 1);
        gate.shell_history(false).unwrap();
        assert_eq!(identity(source), edited);
        assert_eq!(source.snapshot().unwrap().revision, revision + 2);
        assert_eq!(depth(source), (1, 0));
    }

    #[test]
    fn block_styles_h1_through_h6_preserve_atx_prefix_closing_markers_entities_and_source_history()
    {
        runtime(|| {
            for level in 1..=6 {
                let old = if level == 2 { "#" } else { "##" };
                let raw = format!("  {old}  中文 &amp; \\*字\\* ##\r\n\n<custom>keep</custom>\n");
                let expected = raw.replacen(old, &"#".repeat(level as usize), 1);
                let (source, _, gate, controls) = fixture(&raw);
                select(&source, &gate, "中文");
                let original = identity(&source);
                controls.begin().unwrap();
                controls.apply_level(Some(level)).unwrap();
                assert!(!controls.open.get());
                let state = gate.session().state();
                assert_eq!(
                    state.doc().child(0).attrs().get("level"),
                    Some(&serde_json::json!(level))
                );
                assert_eq!(state.doc().child(0).text_content(), "中文 & *字*");
                retained_body(&source, &raw, "中文 &amp; \\*字\\*");
                roundtrip(&source, &gate, &original, &expected);
            }
        });
    }

    #[test]
    fn block_styles_setext_h1_h2_keep_original_underline_length_and_mixed_eols() {
        runtime(|| {
            for (underline, level, next) in [("=======", 2, "-------"), ("-------", 1, "=======")] {
                let raw =
                    format!("中文 &amp; \\*字\\*\r\n{underline}\n\n<custom>keep</custom>\r\n");
                let expected = raw.replacen(underline, next, 1);
                let (source, _, gate, controls) = fixture(&raw);
                select(&source, &gate, "中文");
                let original = identity(&source);
                controls.begin().unwrap();
                controls.apply_level(Some(level)).unwrap();
                roundtrip(&source, &gate, &original, &expected);
            }
        });
    }

    #[test]
    fn block_styles_atx_to_paragraph_removes_only_own_opening_and_closing_markers() {
        runtime(|| {
            for terminal in ["", "\n", "\r\n"] {
                let raw = format!("  ##  中文 &amp; \\*字\\* ##{terminal}");
                let expected = format!("  中文 &amp; \\*字\\*{terminal}");
                let (source, _, gate, controls) = fixture(&raw);
                select(&source, &gate, "中文");
                let original = identity(&source);
                controls.begin().unwrap();
                controls.apply_level(None).unwrap();
                assert_eq!(
                    gate.session().state().doc().child(0).ty().name(),
                    "paragraph"
                );
                retained_body(&source, &raw, "中文 &amp; \\*字\\*");
                roundtrip(&source, &gate, &original, &expected);
            }
        });
    }

    #[test]
    fn block_styles_empty_closed_atx_and_hash_text_setext_keep_exact_source_history() {
        runtime(|| {
            for terminal in ["", "\n", "\r\n"] {
                let raw = format!("# ###{terminal}");
                let (source, _, gate, controls) = fixture(&raw);
                let original = identity(&source);
                controls.begin().unwrap();
                assert!(controls.info.get().is_empty());
                controls.apply_level(None).unwrap();
                assert_eq!(
                    gate.session().state().doc().child(0).ty().name(),
                    "paragraph"
                );
                roundtrip(&source, &gate, &original, terminal);
            }
            let raw = "##literal\r\n=======\n\n<custom>keep</custom>\r\n";
            let expected = "### ##literal\n\n<custom>keep</custom>\r\n";
            let (source, _, gate, controls) = fixture(raw);
            select(&source, &gate, "##literal");
            let original = identity(&source);
            // runtime24: distinguish importer semantics from a delimiter-plan
            // error before applying this exact Setext -> H3 regression.
            let state = gate.session().state();
            let canonical = source.snapshot().unwrap().rope.to_string();
            let ast = markdown::to_mdast(&canonical, &markdown::ParseOptions::gfm()).unwrap();
            let heading = ast.children().unwrap().first().unwrap();
            assert!(
                matches!(heading, markdown::mdast::Node::Heading(h) if h.depth == 1),
                "original GFM AST must be Setext H1: {ast:?}"
            );
            assert_eq!(
                state.doc().child(0).text_content(),
                "##literal",
                "original import cannot discard literal hashes: AST={ast:?}, doc={}",
                state.doc().to_json()
            );
            assert_eq!(
                state.doc().child(0).attrs().get("level"),
                Some(&serde_json::json!(1))
            );
            let original_projection = Projection::parse(state.schema(), canonical.clone()).unwrap();
            assert_eq!(original_projection.rich_position(0), Some(1));
            assert_eq!(original_projection.source_position(1), Some(0));
            let expected_projection =
                Projection::parse(state.schema(), expected.replace("\r\n", "\n")).unwrap();
            assert_eq!(
                expected_projection.doc.child(0).text_content(),
                "##literal",
                "target ATX import cannot discard literal hashes: {}",
                expected_projection.doc.to_json()
            );
            assert_eq!(
                expected_projection.doc.child(0).attrs().get("level"),
                Some(&serde_json::json!(3))
            );
            for (byte, rich) in [(4, 1), (5, 2), (6, 3)] {
                assert_eq!(
                    expected_projection.rich_position(byte),
                    Some(rich),
                    "literal hashes keep original target-source boundaries"
                );
                assert_eq!(expected_projection.source_position(rich), Some(byte));
            }
            let preview = BlockControls::transaction(&state, 0, Some(3)).unwrap();
            let (preview, _) = state.apply_transaction(preview).unwrap();
            assert_eq!(
                &expected_projection.doc,
                preview.doc(),
                "fixed target source and requested style must have the same whole tree; original={}; expected={}; preview={}",
                state.doc().to_json(),
                expected_projection.doc.to_json(),
                preview.doc().to_json()
            );
            controls.begin().unwrap();
            assert!(controls.info.get().contains("下划线"));
            controls.apply_level(Some(3)).unwrap_or_else(|error| {
                let block = &original_projection.blocks[0];
                panic!("{error}; original_source={canonical:?}; block_span={:?}; block_raw={:?}; original_AST={ast:?}; requested={}",
                    block.source, &canonical[block.source.clone()], preview.doc().to_json());
            });
            retained_body(&source, raw, "##literal");
            roundtrip(&source, &gate, &original, expected);
        });
    }

    #[test]
    fn block_styles_single_line_setext_h3_h6_and_paragraph_keep_body_spelling_terminal_eol_and_neighbor()
     {
        runtime(|| {
            let text = "中文 &amp; \\*字\\*";
            for level in [Some(3), Some(6), None] {
                for (terminal, neighbor) in [
                    ("", ""),
                    ("\n", ""),
                    ("\r\n", ""),
                    ("\n", "\n<custom>keep</custom>\r\n"),
                ] {
                    let raw = format!("{text}\r\n======={terminal}{neighbor}");
                    let prefix = level
                        .map(|n| format!("{} ", "#".repeat(n as usize)))
                        .unwrap_or_default();
                    let expected = format!("{prefix}{text}{terminal}{neighbor}");
                    let (source, _, gate, controls) = fixture(&raw);
                    select(&source, &gate, "中文");
                    let original = identity(&source);
                    controls.begin().unwrap();
                    assert!(controls.info.get().contains("改用 # 形式"));
                    controls.apply_level(level).unwrap();
                    retained_body(&source, &raw, text);
                    roundtrip(&source, &gate, &original, &expected);
                }
            }
        });
    }

    #[test]
    fn block_styles_paragraph_heading_paragraph_uses_two_shared_source_history_entries() {
        runtime(|| {
            let raw = "中文 &amp; \\*字\\*\r\n\r\n<custom>keep</custom>\n";
            let expected = format!("#### {raw}");
            let (source, _, gate, controls) = fixture(raw);
            let original = identity(&source);
            controls.begin().unwrap();
            controls.apply_level(Some(4)).unwrap();
            let heading = identity(&source);
            assert_eq!(heading.bytes, expected.as_bytes());
            controls.begin().unwrap();
            controls.apply_level(None).unwrap();
            let paragraph = identity(&source);
            assert_eq!(paragraph.bytes, raw.as_bytes());
            assert_eq!(
                gate.session().state().doc().child(0).ty().name(),
                "paragraph"
            );
            gate.to_source();
            gate.source_mounted();
            for (undo, expected, history) in [
                (true, &heading, (1, 1)),
                (true, &original, (0, 2)),
                (false, &heading, (1, 1)),
                (false, &paragraph, (2, 0)),
            ] {
                gate.shell_history(undo).unwrap();
                assert_eq!(&identity(&source), expected);
                assert_eq!(depth(&source), history);
            }
        });
    }

    #[test]
    fn block_styles_cancel_ime_stale_selection_revision_and_source_mode_never_apply_old_draft() {
        runtime(|| {
            let raw = "## alpha\r\n\r\nbeta\n\n<custom>keep</custom>\r\n";
            let (source, rich, gate, controls) = fixture(raw);
            select(&source, &gate, "alpha");
            let original = (audit(&source), audit(&rich));
            controls.begin().unwrap();
            controls.cancel();
            assert_eq!((audit(&source), audit(&rich)), original);
            source.set_composing(true);
            assert!(controls.begin().is_err());
            source.set_composing(false);
            controls.begin().unwrap();
            rich.set_composing(true);
            assert!(controls.apply_level(Some(3)).is_err());
            assert_eq!((audit(&source), audit(&rich)), original);
            rich.set_composing(false);
            controls.cancel();
            controls.begin().unwrap();
            select(&source, &gate, "beta");
            select(&source, &gate, "alpha");
            let before = (audit(&source), audit(&rich));
            assert!(controls.apply_level(Some(3)).unwrap_err().contains("过期"));
            assert_eq!((audit(&source), audit(&rich)), before);
            controls.cancel();
            controls.begin().unwrap();
            let s = source.snapshot().unwrap();
            let at = s.rope.to_string().find("alpha").unwrap();
            source
                .edit(
                    s.revision,
                    s.view_epoch,
                    vec![(at..at + 1, "A".into())],
                    ExternalSelections {
                        primary: 0,
                        ranges: vec![(at + 1, at + 1)],
                    },
                    ExternalEditKind::Other,
                )
                .unwrap();
            source.notify();
            let before = (audit(&source), audit(&rich));
            assert!(controls.apply_level(Some(3)).is_err());
            assert_eq!((audit(&source), audit(&rich)), before);
            controls.cancel();
            controls.begin().unwrap();
            gate.to_source();
            let before = (audit(&source), audit(&rich));
            assert!(controls.apply_level(Some(3)).is_err());
            assert_eq!((audit(&source), audit(&rich)), before);
            assert_eq!(depth(&source), (1, 0));
        });
    }
}
