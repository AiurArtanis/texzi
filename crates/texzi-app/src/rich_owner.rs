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
        let edits = p.transaction_edits(next.doc(), &applied)?;
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
    #[test]
    fn known_structure_text_edits_preserve_original_syntax_eols_and_neighbor() {
        run(|| {
            for raw in [
                "+ apple\r\n+ pear\n\n<custom>keep</custom>\r\n",
                "7) apple\r\n8) pear\n\n<custom>keep</custom>\r\n",
                "- [ ] apple\r\n- [x] pear\n\n<custom>keep</custom>\r\n",
                "| headA | headB |\r\n| :--- | ---: |\n| apple | pear |\r\n\r\n<custom>keep</custom>\n",
                "~~~ rust\r\napple\npear\r\n~~~\n\n<custom>keep</custom>\r\n",
                "[apple](<https://example.invalid/path> \"keep title\")\r\n\r\n<custom>keep</custom>\n",
                "apple\r\n=====\n\n<custom>keep</custom>\r\n",
            ] {
                let source = NativeView::fixture(raw);
                let owner =
                    RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
                owner.activate().unwrap();
                let pos = owner
                    .projection
                    .borrow()
                    .rich_position(
                        source
                            .snapshot()
                            .unwrap()
                            .rope
                            .to_string()
                            .find("apple")
                            .unwrap(),
                    )
                    .unwrap();
                select(&owner, pos);
                let mut tr = owner.session.state().tr();
                tr.insert_text("A", Some(pos), Some(pos + 1));
                owner.session.dispatch(tr).unwrap();
                assert!(
                    owner.status.get().is_empty(),
                    "fixture {raw:?}: {}",
                    owner.status.get()
                );
                assert_eq!(bytes(&source), raw.replacen("apple", "Apple", 1).as_bytes());
                owner.history(true);
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                assert_eq!(bytes(&source), raw.as_bytes());
                owner.history(false);
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                assert_eq!(bytes(&source), raw.replacen("apple", "Apple", 1).as_bytes());
            }
        });
    }
    #[test]
    fn code_newline_uses_step_range_preserving_existing_mixed_eol_identity() {
        run(|| {
            let raw = "~~~ rust\r\nalpha\nbeta\r\n~~~\r\n\r\n<custom>keep</custom>\r\n";
            let source = NativeView::fixture(raw);
            let owner = RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
            owner.activate().unwrap();
            let at = source
                .snapshot()
                .unwrap()
                .rope
                .to_string()
                .find("alpha")
                .unwrap()
                + 5;
            let pos = owner.projection.borrow().rich_position(at).unwrap();
            select(&owner, pos);
            let mut tr = owner.session.state().tr();
            tr.insert_text("\n", Some(pos), Some(pos));
            owner.session.dispatch(tr).unwrap();
            assert!(owner.status.get().is_empty(), "{}", owner.status.get());
            let expected = raw.replacen("alpha", "alpha\r\n", 1);
            assert_eq!(bytes(&source), expected.as_bytes());
            source.inspect(|s, id| {
                let p = s.snapshot(s.view(id).unwrap().doc).unwrap();
                let kept: usize = p
                    .body
                    .origins()
                    .into_iter()
                    .filter_map(|(_, o)| {
                        o.map(|o| {
                            assert_eq!(o.source.as_ref(), raw);
                            o.raw.len()
                        })
                    })
                    .sum();
                assert_eq!(kept, raw.len());
            });
            owner.history(true);
            assert_eq!(bytes(&source), raw.as_bytes());
            owner.history(false);
            assert_eq!(bytes(&source), expected.as_bytes());
        });
    }
    #[test]
    fn two_step_table_leaf_edit_keeps_alignment_padding_and_original_origins() {
        run(|| {
            let raw = "| H1  | H2 |\r\n| :------ | ---: |\n| apple   | pear |\r\n\r\n<custom>keep</custom>\n";
            let source = NativeView::fixture(raw);
            let owner = RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
            owner.activate().unwrap();
            let at = source
                .snapshot()
                .unwrap()
                .rope
                .to_string()
                .find("apple")
                .unwrap();
            let pos = owner.projection.borrow().rich_position(at).unwrap();
            select(&owner, pos);
            let mut tr = owner.session.state().tr();
            tr.delete(pos, pos + 1).unwrap();
            tr.insert_text("A", Some(pos), Some(pos));
            assert_eq!(tr.steps.len(), 2);
            owner.session.dispatch(tr).unwrap();
            assert!(owner.status.get().is_empty(), "{}", owner.status.get());
            assert_eq!(bytes(&source), raw.replacen("apple", "Apple", 1).as_bytes());
            source.inspect(|s, id| {
                let p = s.snapshot(s.view(id).unwrap().doc).unwrap();
                let kept: usize = p
                    .body
                    .origins()
                    .into_iter()
                    .filter_map(|(_, o)| o.map(|o| o.raw.len()))
                    .sum();
                assert_eq!(kept, raw.len() - 1);
            });
            owner.history(true);
            assert_eq!(bytes(&source), raw.as_bytes());
            owner.history(false);
            assert_eq!(bytes(&source), raw.replacen("apple", "Apple", 1).as_bytes());
        });
    }
    #[test]
    fn table_last_cell_tab_adds_empty_row_through_installed_keymap() {
        run(|| {
            // Read-only reconstruction of round-table-01/trace.jsonl, driven
            // by its driver.ps1: the existing cell edit/redo precedes End+Tab.
            let raw = "| name | value |\n| --- | --- |\n| apple乙 | 10 |";
            assert_table_tab_source_roundtrip(
                raw,
                &format!("{raw}\n|  |  |"),
                &format!("{raw}\n| 甲 |  |"),
            );
        });
    }
    #[test]
    fn table_tab_empty_cells_accept_text_and_preserve_source_history_eols_and_origins() {
        run(|| {
            for eol in ["\n", "\r\n"] {
                let raw = format!("| name | value |{eol}| --- | --- |{eol}| apple乙 | 10 |{eol}");
                let after_tab = format!("{raw}|  |  |{eol}");
                let after_text = format!("{raw}| 甲 |  |{eol}");
                assert_table_tab_source_roundtrip(&raw, &after_tab, &after_text);
            }
            // CRLF dominates, while the last original row's LF must retain
            // its identity before the inserted row. Delimiter widths, cell
            // padding and the opaque neighbor are original bytes throughout.
            let raw = "| name   | value |\r\n| -------- | ----- |\r\n| apple乙  | 10 |\n\r\n<custom>keep</custom>\n";
            let after_tab = "| name   | value |\r\n| -------- | ----- |\r\n| apple乙  | 10 |\n|  |  |\r\n\r\n<custom>keep</custom>\n";
            let after_text = "| name   | value |\r\n| -------- | ----- |\r\n| apple乙  | 10 |\n| 甲 |  |\r\n\r\n<custom>keep</custom>\n";
            assert_table_tab_source_roundtrip(raw, after_tab, after_text);
        });
    }
    fn assert_table_tab_source_roundtrip(raw: &str, after_tab: &str, after_text: &str) {
        let source = NativeView::fixture(raw);
        let rich = source.sibling().unwrap();
        let owner = RichOwner::new(rich.clone(), signal(String::new())).unwrap();
        owner.activate().unwrap();
        let identity = || {
            source.inspect(|store, id| {
                let snapshot = store.snapshot(store.view(id).unwrap().doc).unwrap();
                (
                    snapshot.token,
                    snapshot.body.eols(),
                    format!("{:?}", snapshot.body.origins()),
                )
            })
        };
        let original_identity = identity();
        let normalized = source.snapshot().unwrap().rope.to_string();
        let end = owner
            .projection
            .borrow()
            .rich_position(normalized.find("10").unwrap() + 2)
            .unwrap();
        select(&owner, end);
        key(&owner, "Tab");
        assert_eq!(bytes(&source), after_tab.as_bytes(), "fixture {raw:?}");
        assert_eq!(bytes(&rich), after_tab.as_bytes());
        let tab_identity = identity();
        let state = owner.session.state();
        let table = state.doc().child(0);
        assert_eq!(table.ty().name(), "table");
        assert_eq!(table.child_count(), 3);
        let row = table.child(2);
        assert_eq!(row.child_count(), 2);
        for column in 0..2 {
            let cell = row.child(column);
            assert_eq!(cell.ty().name(), "tableCell");
            assert_eq!(cell.child_count(), 1);
            assert_eq!(cell.child(0).ty().name(), "paragraph");
            assert_eq!(cell.child(0).content().size(), 0);
        }
        let mut empty_cells = Vec::new();
        state.doc().descendants(&mut |node, pos, parent, _| {
            if node.ty().name() == "paragraph"
                && node.content().size() == 0
                && parent.is_some_and(|parent| parent.ty().name() == "tableCell")
            {
                empty_cells.push(pos + 1);
            }
            true
        });
        assert_eq!(empty_cells.len(), 2);
        let row_start = source
            .snapshot()
            .unwrap()
            .rope
            .to_string()
            .find("|  |  |")
            .unwrap();
        {
            let projection = owner.projection.borrow();
            for (rich_pos, source_pos) in empty_cells.iter().zip([row_start + 2, row_start + 5]) {
                assert_eq!(projection.source_position(*rich_pos), Some(source_pos));
                assert_eq!(projection.rich_position(source_pos), Some(*rich_pos));
            }
        }
        assert_eq!(state.selection().anchor(), empty_cells[0]);
        assert_eq!(state.selection().head(), empty_cells[0]);
        assert_eq!(
            rich.snapshot().unwrap().selections.ranges,
            vec![(row_start + 2, row_start + 2)]
        );
        // Move across both empty cells through the same installed Tab binding,
        // then return to the first cell without creating another history edit.
        key(&owner, "Tab");
        assert_eq!(owner.session.state().selection().anchor(), empty_cells[1]);
        assert_eq!(
            rich.snapshot().unwrap().selections.ranges,
            vec![(row_start + 5, row_start + 5)]
        );
        select(&owner, empty_cells[0]);
        let mut tr = owner.session.state().tr();
        tr.insert_text("甲", None, None);
        owner.session.dispatch(tr).unwrap();
        assert!(owner.status.get().is_empty(), "{}", owner.status.get());
        assert_eq!(bytes(&source), after_text.as_bytes());
        assert_eq!(bytes(&rich), after_text.as_bytes());
        let text_identity = identity();
        source.inspect(|store, id| {
            let snapshot = store.snapshot(store.view(id).unwrap().doc).unwrap();
            let mut kept_original = Vec::new();
            for (_, origin) in snapshot.body.origins() {
                if let Some(origin) = origin {
                    assert_eq!(origin.source.as_ref(), raw);
                    kept_original.extend_from_slice(&origin.source.as_bytes()[origin.raw]);
                }
            }
            assert_eq!(kept_original, raw.as_bytes());
            assert_eq!(store.history_depth(snapshot.doc).unwrap(), (2, 0));
            assert_eq!(
                snapshot.doc,
                rich.inspect(|other, id| other.view(id).unwrap().doc)
            );
        });
        assert!(
            nsh_base::rich_editor::extensions::history::history_state(&owner.session.state())
                .is_none()
        );
        let final_doc = owner.session.state().doc().clone();
        // Undo/redo through the Source view alone: the rich sibling sees the
        // same Store body, EOL provenance, token and two-entry history.
        for (undo, expected, expected_identity, depth) in [
            (true, after_tab, &tab_identity, (1, 1)),
            (true, raw, &original_identity, (0, 2)),
            (false, after_tab, &tab_identity, (1, 1)),
            (false, after_text, &text_identity, (2, 0)),
        ] {
            let snapshot = source.snapshot().unwrap();
            source
                .history(snapshot.revision, snapshot.view_epoch, undo)
                .unwrap();
            assert_eq!(bytes(&source), expected.as_bytes());
            assert_eq!(bytes(&rich), expected.as_bytes());
            assert_eq!(&identity(), expected_identity);
            source.inspect(|store, id| {
                assert_eq!(
                    store.history_depth(store.view(id).unwrap().doc).unwrap(),
                    depth
                );
            });
        }
        let reopened = RichOwner::new(rich, signal(String::new())).unwrap();
        reopened.activate().unwrap();
        // A reopened editor owns a fresh Schema/NodeType identity. Compare
        // complete serialized node types, attrs, marks and content instead.
        assert_eq!(
            reopened.session.state().doc().to_json(),
            final_doc.to_json()
        );
    }
    #[test]
    fn empty_container_input_cannot_bind_escaped_text_to_another_code_leaf() {
        run(|| {
            for raw in [
                "| name   | value |\r\n| -------- | ----- |\r\n| `*a*` | 10 |\r\n|  |  |\r\n",
                "+ `*a*`\r\n+ \r\n",
            ] {
                let source = NativeView::fixture(raw);
                let owner =
                    RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
                owner.activate().unwrap();
                let normalized = source.snapshot().unwrap().rope.to_string();
                let at = if raw.starts_with('|') {
                    normalized.find("|  |  |").unwrap() + 2
                } else {
                    normalized.rfind("+ ").unwrap() + 2
                };
                let rich_pos = owner.projection.borrow().rich_position(at).unwrap();
                select(&owner, rich_pos);
                let before = audit(&source);
                let old = owner.session.state();
                let mut tr = old.tr();
                tr.insert_text("*a*", None, None);
                owner.session.dispatch(tr).unwrap();
                assert_eq!(bytes(&source), raw.as_bytes());
                assert_eq!(audit(&source), before);
                assert!(!owner.status.get().is_empty());
                assert_eq!(owner.session.state().doc(), old.doc());
            }
        });
    }
    #[test]
    fn empty_fenced_code_and_language_duplicate_map_to_body_not_fence() {
        run(|| {
            for raw in ["```\r\n\r\n```\r\n", "~~~ rust\r\nrust\r\n~~~\r\n"] {
                let source = NativeView::fixture(raw);
                let owner =
                    RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
                owner.activate().unwrap();
                select(&owner, 1);
                let mut tr = owner.session.state().tr();
                tr.insert_text("甲", None, None);
                owner.session.dispatch(tr).unwrap();
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                let at = raw.find('\n').unwrap() + 1;
                let mut expected = raw.to_owned();
                expected.insert_str(at, "甲");
                assert_eq!(bytes(&source), expected.as_bytes());
                owner.history(true);
                assert_eq!(bytes(&source), raw.as_bytes());
                owner.history(false);
                assert_eq!(bytes(&source), expected.as_bytes());
            }
        });
    }
    #[test]
    fn identical_text_replacement_is_zero_body_transaction() {
        run(|| {
            let source = NativeView::fixture("abc");
            let owner = RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
            let before = source.inspect(|s, id| {
                let p = s.snapshot(s.view(id).unwrap().doc).unwrap();
                (
                    p.revision,
                    p.token,
                    s.history_depth(p.doc).unwrap(),
                    s.dirty(p.doc).unwrap(),
                )
            });
            let mut tr = owner.session.state().tr();
            tr.insert_text("a", Some(1), Some(2));
            owner.session.dispatch(tr).unwrap();
            assert!(owner.status.get().is_empty(), "{}", owner.status.get());
            assert_eq!(
                before,
                source.inspect(|s, id| {
                    let p = s.snapshot(s.view(id).unwrap().doc).unwrap();
                    (
                        p.revision,
                        p.token,
                        s.history_depth(p.doc).unwrap(),
                        s.dirty(p.doc).unwrap(),
                    )
                })
            );
        });
    }
    #[test]
    fn task_checkbox_changes_only_marker_and_undo_preserves_other_task_style() {
        run(|| {
            let raw = "+ [ ] apple\r\n+ [X] pear\n\n<custom>keep</custom>\r\n";
            let source = NativeView::fixture(raw);
            let owner = RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
            owner.activate().unwrap();
            let state = owner.session.state();
            let mut at = None;
            state.doc().descendants(&mut |n, p, _, _| {
                if at.is_none() && n.ty().name() == "taskItem" {
                    at = Some(p);
                }
                true
            });
            let mut tr = state.tr();
            tr.set_node_attribute(at.unwrap(), "checked", serde_json::json!(true))
                .unwrap();
            owner.session.dispatch(tr).unwrap();
            assert!(owner.status.get().is_empty(), "{}", owner.status.get());
            assert_eq!(bytes(&source), raw.replacen("[ ]", "[x]", 1).as_bytes());
            owner.history(true);
            assert_eq!(bytes(&source), raw.as_bytes());
            owner.history(false);
            assert_eq!(bytes(&source), raw.replacen("[ ]", "[x]", 1).as_bytes());
        });
    }
    #[test]
    fn uncheck_task_prefix_does_not_match_lowercase_marker_in_body() {
        run(|| {
            for marker in ["X", "x"] {
                let raw = format!("+ [{marker}] apple [x]\r\n");
                let source = NativeView::fixture(&raw);
                let owner =
                    RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
                owner.activate().unwrap();
                let state = owner.session.state();
                let mut at = None;
                state.doc().descendants(&mut |n, p, _, _| {
                    if at.is_none() && n.ty().name() == "taskItem" {
                        at = Some(p);
                    }
                    true
                });
                let mut tr = state.tr();
                tr.set_node_attribute(at.unwrap(), "checked", serde_json::json!(false))
                    .unwrap();
                owner.session.dispatch(tr).unwrap();
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                let expected = "+ [ ] apple [x]\r\n";
                assert_eq!(bytes(&source), expected.as_bytes());
                owner.history(true);
                assert_eq!(bytes(&source), raw.as_bytes());
                owner.history(false);
                assert_eq!(bytes(&source), expected.as_bytes());
                let before = source.inspect(|s, id| {
                    let p = s.snapshot(s.view(id).unwrap().doc).unwrap();
                    (p.revision, p.token, s.history_depth(p.doc).unwrap())
                });
                let mut tr = owner.session.state().tr();
                tr.set_node_attribute(at.unwrap(), "checked", serde_json::json!(false))
                    .unwrap();
                owner.session.dispatch(tr).unwrap();
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                assert_eq!(
                    before,
                    source.inspect(|s, id| {
                        let p = s.snapshot(s.view(id).unwrap().doc).unwrap();
                        (p.revision, p.token, s.history_depth(p.doc).unwrap())
                    })
                );
            }
        });
    }
    fn select(owner: &RichOwner, pos: usize) {
        let state = owner.session.state();
        let mut tr = state.tr();
        tr.set_selection(TextSelection::create(state.doc(), pos, None));
        owner.session.dispatch(tr).unwrap();
        assert!(owner.status.get().is_empty(), "{}", owner.status.get());
    }
    fn key(owner: &RichOwner, name: &str) {
        let state = owner.session.state();
        let event = nsh_base::rich_editor::state::KeyEvent::new(name);
        let handled = state.plugins().iter().any(|plugin| {
            plugin.handle_key_down(&state, &event, &mut |tr| {
                owner.session.dispatch(tr).unwrap();
            })
        });
        assert!(handled, "installed keymap did not handle {name}");
        assert!(
            owner.status.get().is_empty(),
            "{name}: {}",
            owner.status.get()
        );
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
    fn list_item_enter_at_end_accepts_text_and_source_undo_redo_with_terminal_eol() {
        run(|| {
            for eol in ["\n", "\r\n", ""] {
                let raw = format!("+ apple甲{eol}");
                // With no existing EOL, Store inserts CRLF but must not add a
                // terminal EOL that the original document did not contain.
                let separator = if eol.is_empty() { "\r\n" } else { eol };
                let after_enter = format!("+ apple甲{separator}+ {eol}");
                let after_text = format!("+ apple甲{separator}+ 乙{eol}");
                let source = NativeView::fixture(&raw);
                let rich = source.sibling().unwrap();
                let owner = RichOwner::new(rich.clone(), signal(String::new())).unwrap();
                owner.activate().unwrap();
                let end = owner
                    .projection
                    .borrow()
                    .rich_position("+ apple甲".len())
                    .unwrap();
                select(&owner, end);
                let original_token =
                    source.inspect(|s, id| s.snapshot(s.view(id).unwrap().doc).unwrap().token);
                let state = owner.session.state();
                // Use the exact command installed by NSH's listItem extension.
                let (_, enter) =
                    nsh_base::rich_editor::extensions::kit::list_item_keys(state.schema())
                        .into_iter()
                        .find(|(key, _)| key == "Enter")
                        .expect("listItem must bind Enter");
                let mut dispatch = |tr| owner.session.dispatch(tr).unwrap();
                assert!(enter(&state, Some(&mut dispatch)));
                assert!(
                    owner.status.get().is_empty(),
                    "Enter with {eol:?}: {}",
                    owner.status.get()
                );
                assert_eq!(bytes(&source), after_enter.as_bytes());
                let state = owner.session.state();
                let list = state.doc().child(0);
                assert_eq!(list.ty().name(), "bulletList");
                assert_eq!(list.child_count(), 2);
                assert_eq!(list.child(1).ty().name(), "listItem");
                assert_eq!(list.child(1).child(0).ty().name(), "paragraph");
                assert_eq!(list.child(1).child(0).content().size(), 0);
                let caret = "+ apple甲\n+ ".len();
                let projection = owner.projection.borrow();
                assert_eq!(
                    projection.source_position(state.selection().anchor()),
                    Some(caret)
                );
                assert_eq!(
                    projection.rich_position(caret),
                    Some(state.selection().anchor())
                );
                drop(projection);
                assert_eq!(
                    rich.snapshot().unwrap().selections.ranges,
                    vec![(caret, caret)]
                );
                let mut tr = state.tr();
                tr.insert_text("乙", None, None);
                owner.session.dispatch(tr).unwrap();
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                assert_eq!(bytes(&source), after_text.as_bytes());
                assert_eq!(bytes(&rich), after_text.as_bytes());
                assert!(
                    nsh_base::rich_editor::extensions::history::history_state(
                        &owner.session.state()
                    )
                    .is_none()
                );
                source.inspect(|s, id| {
                    assert_eq!(s.history_depth(s.view(id).unwrap().doc).unwrap(), (2, 0));
                });
                // Source history owns both edits; no rich-only undo stack exists.
                for (undo, expected) in [
                    (true, after_enter.as_str()),
                    (true, raw.as_str()),
                    (false, after_enter.as_str()),
                    (false, after_text.as_str()),
                ] {
                    let snapshot = source.snapshot().unwrap();
                    source
                        .history(snapshot.revision, snapshot.view_epoch, undo)
                        .unwrap();
                    assert_eq!(bytes(&source), expected.as_bytes());
                    assert_eq!(bytes(&rich), expected.as_bytes());
                    if undo && expected == raw {
                        source.inspect(|s, id| {
                            assert_eq!(
                                s.snapshot(s.view(id).unwrap().doc).unwrap().token,
                                original_token
                            );
                        });
                    }
                }
                source.inspect(|s, id| {
                    assert_eq!(s.history_depth(s.view(id).unwrap().doc).unwrap(), (2, 0));
                });
            }
        });
    }
    #[test]
    fn list_item_split_preserves_bullets_mixed_eols_origins_and_opaque_neighbor() {
        run(|| {
            for bullet in ["+", "-", "*"] {
                let raw = format!("{bullet} apple甲\r\n{bullet} pear\n\n<custom>keep</custom>\r\n");
                let expected = raw.replacen("apple", &format!("apple\r\n{bullet} "), 1);
                let source = NativeView::fixture(&raw);
                let owner =
                    RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
                owner.activate().unwrap();
                let at = owner
                    .projection
                    .borrow()
                    .rich_position(format!("{bullet} apple").len())
                    .unwrap();
                select(&owner, at);
                let state = owner.session.state();
                let (_, enter) =
                    nsh_base::rich_editor::extensions::kit::list_item_keys(state.schema())
                        .into_iter()
                        .find(|(key, _)| key == "Enter")
                        .unwrap();
                let mut dispatch = |tr| owner.session.dispatch(tr).unwrap();
                assert!(enter(&state, Some(&mut dispatch)));
                assert!(
                    owner.status.get().is_empty(),
                    "{bullet}: {}",
                    owner.status.get()
                );
                assert_eq!(bytes(&source), expected.as_bytes());
                source.inspect(|s, id| {
                    let snapshot = s.snapshot(s.view(id).unwrap().doc).unwrap();
                    let mut original_bytes = 0;
                    let mut inserted = 0;
                    for (logical, origin) in snapshot.body.origins() {
                        match origin {
                            Some(origin) => {
                                assert_eq!(origin.source.as_ref(), raw);
                                original_bytes += origin.raw.len();
                            }
                            None => inserted += logical.len(),
                        }
                    }
                    assert_eq!(original_bytes, raw.len());
                    assert_eq!(inserted, 3);
                    assert_eq!(s.history_depth(snapshot.doc).unwrap(), (1, 0));
                });
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
    fn marker_only_bullet_does_not_gain_lossy_text_mapping() {
        run(|| {
            for raw in ["+", "-", "*"] {
                let source = NativeView::fixture(raw);
                let owner =
                    RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
                // The old marker-only case has no source text insertion point.
                // Whether activation rejects or succeeds, typing must not rewrite its marker.
                let _activation = owner.activate();
                let before = audit(&source);
                let old = owner.session.state();
                let mut tr = old.tr();
                tr.insert_text("甲", None, None);
                owner.session.dispatch(tr).unwrap();
                assert_eq!(bytes(&source), raw.as_bytes(), "marker-only {raw}");
                assert!(!owner.status.get().is_empty());
                assert_eq!(audit(&source), before);
                assert_eq!(owner.session.state().doc(), old.doc());
            }
        });
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
    fn leading_split_join_has_its_own_undo_redo_and_preserves_terminal_eol() {
        run(|| {
            for (raw, split_first, before_join, after_join) in [
                ("abc", true, "\r\n\r\nabc", "abc"),
                ("\n\nabc\n", false, "\n\nabc\n", "abc\n"),
                ("\r\n\nabc\r\n", false, "\r\n\nabc\r\n", "abc\r\n"),
            ] {
                let source = NativeView::fixture(raw);
                let owner =
                    RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
                if split_first {
                    select(&owner, 1);
                    split(&owner);
                }
                select(&owner, 3);
                assert_eq!(bytes(&source), before_join.as_bytes());
                let before =
                    source.inspect(|s, id| s.snapshot(s.view(id).unwrap().doc).unwrap().token);
                let state = owner.session.state();
                let mut dispatch = |tr| owner.session.dispatch(tr).unwrap();
                assert!(nsh_base::rich_editor::commands::join_backward(
                    &state,
                    Some(&mut dispatch)
                ));
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                assert_eq!(bytes(&source), after_join.as_bytes());
                let after =
                    source.inspect(|s, id| s.snapshot(s.view(id).unwrap().doc).unwrap().token);
                assert_ne!(before, after);
                owner.history(true);
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                assert_eq!(bytes(&source), before_join.as_bytes());
                assert_eq!(
                    source.inspect(|s, id| s.snapshot(s.view(id).unwrap().doc).unwrap().token),
                    before
                );
                owner.history(false);
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                assert_eq!(bytes(&source), after_join.as_bytes());
                assert_eq!(
                    source.inspect(|s, id| s.snapshot(s.view(id).unwrap().doc).unwrap().token),
                    after
                );
                source.inspect(|s, id| {
                    assert_eq!(
                        s.history_depth(s.view(id).unwrap().doc).unwrap(),
                        (if split_first { 2 } else { 1 }, 0)
                    )
                });
            }
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
