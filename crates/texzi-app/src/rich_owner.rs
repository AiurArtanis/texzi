//! UI-thread dispatcher: rich states are projections, never a second history.
use crate::{
    editor_owner::NativeView,
    rich_projection::{Projection, manager},
};
use nsh_base::{
    editor::{ExternalDocument, ExternalEditKind, ExternalSelections},
    rich_editor::{
        commands::BoxCommand,
        extensions::keymap::keymap,
        state::{EditorState, EditorStateConfig, TextSelection, Transaction, TransactionOrigin},
        view::{EditorSession, NodeViews},
    },
};
use nsh_core::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

thread_local! {static OWNERS:RefCell<HashMap<u64,Weak<RichOwner>>>=RefCell::new(HashMap::new());}
static NEXT: AtomicU64 = AtomicU64::new(1);
pub struct RichOwner {
    id: u64,
    native: Rc<NativeView>,
    projection: RefCell<Projection>,
    revision: Cell<u64>,
    epoch: Cell<u64>,
    session: EditorSession,
    pub status: Signal<String>,
}
impl Drop for RichOwner {
    fn drop(&mut self) {
        OWNERS.with(|x| {
            x.borrow_mut().remove(&self.id);
        });
    }
}
fn owner(id: u64) -> Option<Rc<RichOwner>> {
    OWNERS.with(|x| x.borrow().get(&id).and_then(Weak::upgrade))
}
impl RichOwner {
    pub fn new(native: Rc<NativeView>, status: Signal<String>) -> Result<Rc<Self>, String> {
        let m = manager();
        let s = native.snapshot()?;
        let p = Projection::parse(m.schema(), s.rope.to_string())?;
        if std::env::var_os("TEXZI_T05_FAIL_PREPARE").as_deref() == Some(std::ffi::OsStr::new("1"))
        {
            return Err("测试故障：富文本投影准备失败，已保留源码视图".into());
        }
        let id = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| "富文本视图标识已耗尽")?;
        let undo: BoxCommand = Arc::new(move |_, dispatch| {
            if dispatch.is_some() {
                if let Some(o) = owner(id) {
                    o.history(true)
                }
            }
            true
        });
        let redo: BoxCommand = Arc::new(move |_, dispatch| {
            if dispatch.is_some() {
                if let Some(o) = owner(id) {
                    o.history(false)
                }
            }
            true
        });
        let mut plugins = vec![keymap(vec![
            ("Mod-z".into(), undo),
            ("Mod-y".into(), redo.clone()),
            ("Mod-Shift-z".into(), redo),
        ])];
        plugins.extend(m.plugins());
        let mut config = EditorStateConfig::new(m.schema().clone())
            .doc(p.doc.clone())
            .plugins(plugins);
        if let Some((a, h)) = restored_selection(&p, &s.selections) {
            config = config.selection(TextSelection::create(&p.doc, a, Some(h)));
        }
        let state = EditorState::create(config).map_err(|e| e.to_string())?;
        let session = EditorSession::new(state, NodeViews::new());
        let this = Rc::new(Self {
            id,
            native,
            projection: RefCell::new(p),
            revision: Cell::new(s.revision),
            epoch: Cell::new(s.view_epoch),
            session: session.clone(),
            status,
        });
        OWNERS.with(|x| {
            x.borrow_mut().insert(id, Rc::downgrade(&this));
        });
        session.set_composition_listener(Some(Arc::new(move |active| {
            if let Some(o) = owner(id) {
                o.native.set_composing(active);
            }
        })));
        // Only the numeric ID crosses the Send+Sync callback contract. All Store
        // access stays in its UI thread's weak registry; no unsafe Send/Sync.
        session.set_dispatch_transaction(Some(Arc::new(move |session, tr| {
            if let Some(o) = owner(id) {
                if let Err(e) = o.dispatch(session, tr) {
                    o.status.set(e.clone());
                    log::warn!("rich transaction rejected: {e}");
                }
            } else {
                log::error!("rich dispatch outside its owning UI thread or after disposal");
            }
        })));
        Ok(this)
    }
    pub fn session(&self) -> EditorSession {
        self.session.clone()
    }
    fn activate(&self) -> Result<(), String> {
        let current = self.native.snapshot()?;
        if current.revision != self.revision.get() || current.view_epoch != self.epoch.get() {
            return Err("准备期间原文已变化，已保留当前模式".into());
        }
        let state = self.session.state();
        let p = self.projection.borrow();
        let a = p
            .source_position(state.selection().anchor())
            .ok_or("该段初始位置暂不能映射，请继续在源码编辑")?;
        let h = p
            .source_position(state.selection().head())
            .ok_or("该段初始位置暂不能映射，请继续在源码编辑")?;
        let selected = self.native.select(
            current.revision,
            current.view_epoch,
            ExternalSelections {
                primary: 0,
                ranges: vec![(a, h)],
            },
        )?;
        self.epoch.set(selected.view_epoch);
        Ok(())
    }
    fn dispatch(&self, session: &EditorSession, tr: Transaction) -> Result<(), String> {
        let current = self.native.snapshot()?;
        if current.revision != self.revision.get() || current.view_epoch != self.epoch.get() {
            return Err("富文本视图版本已过期，请切回源码后重新打开富文本".into());
        }
        let old = session.state();
        let (next, applied) = old.apply_transaction(tr).map_err(|e| e.to_string())?;
        let p = self.projection.borrow();
        if old.doc() != &p.doc {
            return Err("富文本视图已过期，请切回源码后重新打开富文本".into());
        }
        let edits = p.source_edits(next.doc())?;
        if edits.is_empty() && old.doc() != next.doc() {
            return Err("该结构变更不能映射到原文，已保留原状态".into());
        }
        if !edits.is_empty() {
            if !selection_matches(&p, &old, &current.selections) {
                return Err("当前选区尚未同步到原文，请切到源码编辑；原文未变".into());
            }
            let mut raw = p.source.clone();
            for (range, text) in edits.iter().rev() {
                raw.replace_range(range.clone(), text);
            }
            let projected = Projection::parse(next.schema(), raw)?;
            if &projected.doc != next.doc() {
                return Err("该富文本操作不能保真往返；请在源码中编辑此结构".into());
            }
            let a = projected
                .source_position(next.selection().anchor())
                .ok_or("选区起点暂不能保真映射，请切到源码编辑此范围")?;
            let h = projected
                .source_position(next.selection().head())
                .ok_or("选区终点暂不能保真映射，请切到源码编辑此范围")?;
            let kind = if applied.iter().any(|t| t.get_meta("composition").is_some()) {
                ExternalEditKind::ImeCommit
            } else if applied
                .iter()
                .all(|t| matches!(t.origin(), Some(TransactionOrigin::Input)))
            {
                ExternalEditKind::Typing
            } else {
                ExternalEditKind::Other
            };
            let update = self.native.edit(
                current.revision,
                current.view_epoch,
                edits,
                ExternalSelections {
                    primary: 0,
                    ranges: vec![(a, h)],
                },
                kind,
            )?;
            self.revision.set(update.snapshot.revision);
            self.epoch.set(update.snapshot.view_epoch);
            drop(p);
            *self.projection.borrow_mut() = projected;
            self.native.notify();
        } else {
            let a = p
                .source_position(next.selection().anchor())
                .ok_or("选区起点暂不能保真映射，已保留原选区")?;
            let h = p
                .source_position(next.selection().head())
                .ok_or("选区终点暂不能保真映射，已保留原选区")?;
            let selected = self.native.select(
                current.revision,
                current.view_epoch,
                ExternalSelections {
                    primary: 0,
                    ranges: vec![(a, h)],
                },
            )?;
            self.epoch.set(selected.view_epoch);
        }
        session.commit(next, &applied);
        self.status.set(String::new());
        Ok(())
    }
    fn history(&self, undo: bool) {
        let state = self.session.state();
        let p = self.projection.borrow();
        let current = match self.native.snapshot() {
            Ok(s) => s,
            Err(e) => {
                self.status.set(e);
                return;
            }
        };
        if !selection_matches(&p, &state, &current.selections) {
            self.status
                .set("当前选区尚未同步到原文，请切到源码撤销或重做".into());
            return;
        }
        drop(p);
        match self
            .native
            .history(self.revision.get(), self.epoch.get(), undo)
        {
            Ok(u) => {
                let state = self.session.state();
                match Projection::parse(state.schema(), u.snapshot.rope.to_string()).and_then(|p| {
                    let mut config = EditorStateConfig::new(state.schema().clone())
                        .doc(p.doc.clone())
                        .plugins(state.plugins().to_vec());
                    if let Some((a, h)) = restored_selection(&p, &u.snapshot.selections) {
                        config = config.selection(TextSelection::create(&p.doc, a, Some(h)));
                    }
                    let new = EditorState::create(config).map_err(|e| e.to_string())?;
                    Ok((p, new))
                }) {
                    Ok((p, new)) => {
                        self.revision.set(u.snapshot.revision);
                        self.epoch.set(u.snapshot.view_epoch);
                        *self.projection.borrow_mut() = p;
                        self.session.update_state(new);
                        self.native.notify();
                        if selection_matches(
                            &self.projection.borrow(),
                            &self.session.state(),
                            &u.snapshot.selections,
                        ) {
                            self.status.set(String::new());
                        } else {
                            self.status
                                .set("该版本的选区暂不能映射，请在源码中继续编辑".into());
                        }
                    }
                    Err(e) => {
                        self.status.set(e.clone());
                        log::error!("rich refresh after history: {e}");
                    }
                }
            }
            Err(e) => self.status.set(e),
        }
    }
}

fn restored_selection(p: &Projection, s: &ExternalSelections) -> Option<(usize, usize)> {
    let (a, h) = *s.ranges.get(s.primary)?;
    Some((p.rich_position(a)?, p.rich_position(h)?))
}
fn selection_matches(p: &Projection, state: &EditorState, s: &ExternalSelections) -> bool {
    match (
        p.source_position(state.selection().anchor()),
        p.source_position(state.selection().head()),
    ) {
        (Some(a), Some(h)) => s.primary == 0 && s.ranges == vec![(a, h)],
        _ => false,
    }
}

/// The visible mode changes only after a complete destination was prepared.
pub struct ModeGate {
    source: Rc<NativeView>,
    rich: Rc<NativeView>,
    active: RefCell<Option<Rc<RichOwner>>>,
    pub rich_mode: Signal<bool>,
    pub status: Signal<String>,
}
impl ModeGate {
    pub fn new(source: Rc<NativeView>, rich: Rc<NativeView>) -> Rc<Self> {
        Rc::new(Self {
            source,
            rich,
            active: RefCell::new(None),
            rich_mode: signal(false),
            status: signal(String::new()),
        })
    }
    pub fn busy(&self) -> bool {
        self.source.is_composing() || self.rich.is_composing()
    }
    fn prepare_and_switch(
        &self,
        prepare: impl FnOnce() -> Result<Rc<RichOwner>, String>,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("请先完成或取消中文组合，再切换模式".into());
        }
        if self.rich_mode.get() {
            return Ok(());
        }
        let next = prepare()?;
        next.activate()?;
        self.source.fence();
        self.source.trace_state("mode_rich");
        *self.active.borrow_mut() = Some(next);
        self.rich_mode.set(true);
        self.status.set(String::new());
        Ok(())
    }
    pub fn to_rich(&self) {
        if let Err(e) = self.prepare_and_switch(|| RichOwner::new(self.rich.clone(), self.status)) {
            self.status.set(e);
        }
    }
    pub fn to_source(&self) {
        if self.busy() {
            self.status.set("请先完成或取消中文组合，再切换模式".into());
            return;
        }
        if !self.rich_mode.get() {
            return;
        }
        if let Err(e) = self.source.snapshot() {
            self.status.set(e);
            return;
        }
        self.source.fence();
        self.source.trace_state("mode_source");
        self.rich_mode.set(false);
        self.status.set(String::new());
    }
    pub fn session(&self) -> EditorSession {
        self.active
            .borrow()
            .as_ref()
            .expect("mode published only after successful preparation")
            .session()
    }
    pub fn source_mounted(&self) {
        self.active.borrow_mut().take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_base::rich_editor::state::TextSelection;
    fn run(f: impl FnOnce()) {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        f();
    }
    fn audit(n: &NativeView) -> String {
        n.inspect(|s, id| {
            let v = s.view(id).unwrap();
            let p = s.snapshot(v.doc).unwrap();
            format!(
                "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                v,
                p.token,
                p.body.to_utf8(),
                p.body.eols(),
                p.body.origins(),
                s.history_depth(v.doc),
                s.dirty(v.doc),
                s.last_fence(v.doc)
            )
        })
    }
    fn bytes(n: &NativeView) -> Vec<u8> {
        n.inspect(|s, id| s.snapshot(s.view(id).unwrap().doc).unwrap().body.to_utf8())
    }
    fn select(owner: &RichOwner, pos: usize) {
        let state = owner.session.state();
        let mut tr = state.tr();
        tr.set_selection(TextSelection::create(state.doc(), pos, None));
        owner.session.dispatch(tr).unwrap();
        assert!(owner.status.get().is_empty(), "{}", owner.status.get());
    }
    fn split(owner: &RichOwner) {
        let state = owner.session.state();
        let mut dispatch = |tr| owner.session.dispatch(tr).unwrap();
        assert!(nsh_base::rich_editor::commands::split_block(
            &state,
            Some(&mut dispatch)
        ));
        assert!(owner.status.get().is_empty(), "{}", owner.status.get());
    }
    #[test]
    fn paragraph_enter_start_middle_end_and_empty_roundtrip_through_owner() {
        run(|| {
            for (raw, pos, expected) in [
                ("abc", 1, "\n\nabc"),
                ("abc", 2, "a\n\nbc"),
                ("abc", 4, "abc\n\n"),
                ("", 1, "\n\n"),
                ("甲😀乙", 4, "甲😀\n\n乙"),
            ] {
                let source = NativeView::fixture(raw);
                // Newly inserted separators use Store's existing EOL policy;
                // a document without any EOL defaults to Windows CRLF.
                let expected = if raw.contains('\n') {
                    expected.to_owned()
                } else {
                    expected.replace('\n', "\r\n")
                };
                let owner =
                    RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
                select(&owner, pos);
                split(&owner);
                assert_eq!(bytes(&source), expected.as_bytes());
                let rebuilt = Projection::parse(
                    owner.session.state().schema(),
                    source.snapshot().unwrap().rope.to_string(),
                )
                .unwrap();
                assert_eq!(&rebuilt.doc, owner.session.state().doc());
                owner.history(true);
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                assert_eq!(bytes(&source), raw.as_bytes());
                owner.history(false);
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                assert_eq!(bytes(&source), expected.as_bytes());
            }
        });
    }
    #[test]
    fn consecutive_empty_paragraphs_accept_text_and_backspace_join() {
        run(|| {
            let source = NativeView::fixture("abc");
            let owner = RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
            select(&owner, 4);
            split(&owner);
            split(&owner);
            assert_eq!(bytes(&source), b"abc\r\n\r\n\r\n\r\n");
            let mut tr = owner.session.state().tr();
            tr.insert_text("甲", None, None);
            owner.session.dispatch(tr).unwrap();
            assert!(owner.status.get().is_empty(), "{}", owner.status.get());
            assert_eq!(bytes(&source), "abc\r\n\r\n\r\n\r\n甲".as_bytes());
            // Join the last paragraph into its empty predecessor, then abc.
            select(&owner, 8);
            let state = owner.session.state();
            let mut dispatch = |tr| owner.session.dispatch(tr).unwrap();
            assert!(nsh_base::rich_editor::commands::join_backward(
                &state,
                Some(&mut dispatch)
            ));
            assert!(owner.status.get().is_empty(), "{}", owner.status.get());
            assert_eq!(bytes(&source), "abc\r\n\r\n甲".as_bytes());
            select(&owner, 6);
            let state = owner.session.state();
            assert!(nsh_base::rich_editor::commands::join_backward(
                &state,
                Some(&mut dispatch)
            ));
            assert!(owner.status.get().is_empty(), "{}", owner.status.get());
            assert_eq!(bytes(&source), "abc甲".as_bytes());
        });
    }
    #[test]
    fn leading_empty_join_and_middle_empty_insert_use_one_separator_pair() {
        run(|| {
            let source = NativeView::fixture("\n\nabc");
            let owner = RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
            select(&owner, 3);
            let state = owner.session.state();
            let mut dispatch = |tr| owner.session.dispatch(tr).unwrap();
            assert!(nsh_base::rich_editor::commands::join_backward(
                &state,
                Some(&mut dispatch)
            ));
            assert!(owner.status.get().is_empty(), "{}", owner.status.get());
            assert_eq!(bytes(&source), b"abc");
            let source = NativeView::fixture("abc\n\ndef");
            let owner = RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
            select(&owner, 4);
            split(&owner);
            assert_eq!(bytes(&source), b"abc\n\n\n\ndef");
        });
    }
    #[test]
    fn owner_opaque_single_letter_preserves_mixed_eols_origins_and_undo() {
        run(|| {
            let raw = "<custom>\r\nalpha\nbeta\r\n</custom>\n";
            let source = NativeView::fixture(raw);
            let rich = source.sibling().unwrap();
            let owner = RichOwner::new(rich.clone(), signal(String::new())).unwrap();
            let state = owner.session.state();
            let mut tr = state.tr();
            tr.insert_text("A", Some(10), Some(11));
            owner.session.dispatch(tr).unwrap();
            assert!(owner.status.get().is_empty(), "{}", owner.status.get());
            assert_eq!(bytes(&source), raw.replacen("alpha", "Alpha", 1).as_bytes());
            source.inspect(|s, id| {
                let p = s.snapshot(s.view(id).unwrap().doc).unwrap();
                let mut original_bytes = 0;
                let mut inserted = 0;
                for (logical, origin) in p.body.origins() {
                    match origin {
                        Some(o) => {
                            assert_eq!(o.source.as_ref(), raw);
                            original_bytes += o.raw.len();
                        }
                        None => inserted += logical.len(),
                    }
                }
                assert_eq!(original_bytes, raw.len() - 1);
                assert_eq!(inserted, 1);
            });
            owner.history(true);
            assert_eq!(bytes(&source), raw.as_bytes());
            owner.history(false);
            assert_eq!(bytes(&source), raw.replacen("alpha", "Alpha", 1).as_bytes());
        });
    }
    #[test]
    fn owner_multiline_bold_preserves_original_newlines_and_text_origins() {
        run(|| {
            let raw = "alpha\r\nbeta\n";
            let source = NativeView::fixture(raw);
            let rich = source.sibling().unwrap();
            let owner = RichOwner::new(rich, signal(String::new())).unwrap();
            let state = owner.session.state();
            let mut tr = state.tr();
            let mark = state.schema().mark("bold", None).unwrap();
            tr.add_mark(1, 1 + state.doc().child(0).content().size(), mark)
                .unwrap();
            owner.session.dispatch(tr).unwrap();
            assert!(owner.status.get().is_empty(), "{}", owner.status.get());
            assert_eq!(bytes(&source), b"**alpha\r\nbeta**\n");
            source.inspect(|s, id| {
                let p = s.snapshot(s.view(id).unwrap().doc).unwrap();
                let retained: usize = p
                    .body
                    .origins()
                    .into_iter()
                    .filter_map(|(_, o)| o.map(|o| o.raw.len()))
                    .sum();
                assert_eq!(retained, raw.len());
            });
            owner.history(true);
            assert_eq!(bytes(&source), raw.as_bytes());
            owner.history(false);
            assert_eq!(bytes(&source), b"**alpha\r\nbeta**\n");
        });
    }
    #[test]
    fn unmapped_entity_and_duplicate_leaf_selection_are_atomic_rejections() {
        run(|| {
            for raw in ["a &amp; b", "**foo** foo"] {
                let source = NativeView::fixture(raw);
                let rich = source.sibling().unwrap();
                let owner = RichOwner::new(rich.clone(), signal(String::new())).unwrap();
                let old = owner.session.state();
                let before = (audit(&source), audit(&rich));
                let mut tr = old.tr();
                tr.set_selection(TextSelection::create(old.doc(), 2, None));
                owner.session.dispatch(tr).unwrap();
                assert!(!owner.status.get().is_empty());
                assert_eq!(owner.session.state().selection(), old.selection());
                assert_eq!(owner.session.state().doc(), old.doc());
                assert_eq!(before, (audit(&source), audit(&rich)));
                let mut tr = owner.session.state().tr();
                tr.insert_text("X", None, None);
                owner.session.dispatch(tr).unwrap();
                assert!(!owner.status.get().is_empty());
                assert_eq!(before, (audit(&source), audit(&rich)));
                owner.history(true);
                assert!(!owner.status.get().is_empty());
                assert_eq!(before, (audit(&source), audit(&rich)));
            }
        });
    }
    #[test]
    fn supported_selection_synchronizes_authoritative_view() {
        run(|| {
            let source = NativeView::fixture("abc");
            let rich = source.sibling().unwrap();
            let owner = RichOwner::new(rich.clone(), signal(String::new())).unwrap();
            let old = owner.session.state();
            let mut tr = old.tr();
            tr.set_selection(TextSelection::create(old.doc(), 2, Some(3)));
            owner.session.dispatch(tr).unwrap();
            assert!(owner.status.get().is_empty());
            assert_eq!(rich.snapshot().unwrap().selections.ranges, vec![(1, 2)]);
            assert_eq!(bytes(&source), b"abc");
        });
    }
    #[test]
    fn ambiguous_multiline_newline_edit_is_rejected_before_any_owner_commit() {
        run(|| {
            let raw = "<custom>\r\nalpha\nbeta\r\n</custom>\n";
            let source = NativeView::fixture(raw);
            let rich = source.sibling().unwrap();
            let owner = RichOwner::new(rich.clone(), signal(String::new())).unwrap();
            let old = owner.session.state();
            let before = (audit(&source), audit(&rich));
            let mut tr = old.tr();
            tr.insert_text("\n", Some(10), Some(10));
            owner.session.dispatch(tr).unwrap();
            assert!(!owner.status.get().is_empty());
            assert_eq!(owner.session.state().doc(), old.doc());
            assert_eq!(owner.session.state().selection(), old.selection());
            assert_eq!(before, (audit(&source), audit(&rich)));
        });
    }
    #[test]
    fn composition_and_prepare_failure_keep_mode_and_all_owner_state() {
        run(|| {
            let source = NativeView::fixture("abc");
            let rich = source.sibling().unwrap();
            let gate = ModeGate::new(source.clone(), rich.clone());
            let before = (audit(&source), audit(&rich));
            source.set_composing(true);
            gate.to_rich();
            assert!(!gate.rich_mode.get());
            assert_eq!(before, (audit(&source), audit(&rich)));
            source.set_composing(false);
            assert!(
                gate.prepare_and_switch(|| Err("controlled destination failure".into()))
                    .is_err()
            );
            assert!(!gate.rich_mode.get());
            assert!(gate.active.borrow().is_none());
            assert_eq!(before, (audit(&source), audit(&rich)));
            gate.to_rich();
            assert!(gate.rich_mode.get());
            rich.set_composing(true);
            let before = (audit(&source), audit(&rich));
            gate.to_source();
            assert!(gate.rich_mode.get());
            assert_eq!(before, (audit(&source), audit(&rich)));
            rich.set_composing(false);
            gate.to_source();
            assert!(!gate.rich_mode.get());
            assert_eq!(bytes(&source), b"abc");
        });
    }
    #[test]
    fn unrepresentable_document_change_cannot_publish_only_to_rich_state() {
        run(|| {
            use nsh_base::rich_editor::formats::{DocFormat, MarkdownFormat};
            let source = NativeView::fixture("abc");
            let rich = source.sibling().unwrap();
            let owner = RichOwner::new(rich.clone(), signal(String::new())).unwrap();
            let old = owner.session.state();
            let mut preview = old.tr();
            preview
                .set_node_attribute(0, "textAlign", serde_json::json!("center"))
                .unwrap();
            let candidate = old.apply(preview).unwrap();
            assert_ne!(candidate.doc(), old.doc());
            assert_eq!(
                MarkdownFormat::default()
                    .serialize(candidate.doc())
                    .unwrap(),
                MarkdownFormat::default().serialize(old.doc()).unwrap()
            );
            let before = (audit(&source), audit(&rich));
            let mut tr = old.tr();
            tr.set_node_attribute(0, "textAlign", serde_json::json!("center"))
                .unwrap();
            owner.session.dispatch(tr).unwrap();
            assert!(!owner.status.get().is_empty());
            assert_eq!(owner.session.state().doc(), old.doc());
            assert_eq!(before, (audit(&source), audit(&rich)));
        });
    }
}
