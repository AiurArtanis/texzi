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
        let weak = Rc::downgrade(&this);
        effect(move || {
            let Some(owner) = weak.upgrade() else { return };
            owner.native.track();
            if owner.native.is_composing() {
                return;
            }
            let result = owner.refresh_external();
            if let Err(error) = result {
                owner.status.set(error);
            }
        });
        Ok(this)
    }
    /// Other logical views publish through the same Store. Refresh the derived
    /// tree only, keeping the one body/history and the mapped view selection.
    fn refresh_external(&self) -> Result<(), String> {
        let current = self.native.snapshot()?;
        if current.revision == self.revision.get() && current.view_epoch == self.epoch.get() {
            return Ok(());
        }
        let previous = self.session.state();
        let projected = Projection::parse(previous.schema(), current.rope.to_string())?;
        let (a, h) = restored_selection(&projected, &current.selections)
            .ok_or("更新后的正文位置不能映射，原文仍可从源码访问")?;
        let next = EditorState::create(
            EditorStateConfig::new(previous.schema().clone())
                .doc(projected.doc.clone())
                .plugins(previous.plugins().to_vec())
                .selection(TextSelection::create(&projected.doc, a, Some(h))),
        )
        .map_err(|e| e.to_string())?;
        self.session.update_state(next);
        *self.projection.borrow_mut() = projected;
        self.revision.set(current.revision);
        self.epoch.set(current.view_epoch);
        self.activate()?;
        self.status.set(String::new());
        Ok(())
    }
    pub fn session(&self) -> EditorSession {
        self.session.clone()
    }
    fn find_literal(&self, query: &str, previous: bool) -> Result<(usize, usize), String> {
        if query.is_empty() {
            return Err("请输入查找文字".into());
        }
        self.refresh_external()?;
        let state = self.session.state();
        let mut text = String::new();
        let mut positions = std::collections::BTreeMap::new();
        fn flatten(
            node: &nsh_base::rich_editor::model::Node,
            pos: usize,
            root: bool,
            text: &mut String,
            positions: &mut std::collections::BTreeMap<usize, usize>,
        ) {
            if let Some(value) = node.text() {
                let mut offset = pos;
                positions.insert(text.len(), offset);
                for ch in value.chars() {
                    text.push(ch);
                    offset += ch.len_utf16();
                    positions.insert(text.len(), offset);
                }
            } else if node.ty().name() == "hardBreak" {
                positions.insert(text.len(), pos);
                text.push('\n');
                positions.insert(text.len(), pos + 1);
            } else {
                if node.is_textblock() && node.child_count() == 0 {
                    positions.insert(text.len(), pos + 1);
                }
                let mut offset = if root { pos } else { pos + 1 };
                for i in 0..node.child_count() {
                    let child = node.child(i);
                    if i > 0 && child.is_block() {
                        text.push(if node.ty().name() == "tableRow" {
                            '\t'
                        } else {
                            '\n'
                        });
                    }
                    flatten(child, offset, false, text, positions);
                    offset += child.node_size();
                }
            }
        }
        flatten(state.doc(), 0, true, &mut text, &mut positions);
        let hits: Vec<_> = text
            .match_indices(query)
            .filter_map(|(at, _)| {
                Some((*positions.get(&at)?, *positions.get(&(at + query.len()))?))
            })
            .collect();
        if hits.is_empty() {
            return Ok((0, 0));
        }
        let index = if previous {
            hits.iter()
                .rposition(|(_, z)| *z <= state.selection().from())
                .unwrap_or(hits.len() - 1)
        } else {
            hits.iter()
                .position(|(a, _)| *a >= state.selection().to())
                .unwrap_or(0)
        };
        let (a, z) = hits[index];
        let mut tr = state.tr();
        tr.set_selection(TextSelection::create(state.doc(), a, Some(z)));
        self.dispatch(&self.session, tr)?;
        Ok((index + 1, hits.len()))
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
    fn dispatch(&self, session: &EditorSession, mut tr: Transaction) -> Result<(), String> {
        let mut current = self.native.snapshot()?;
        if current.revision != self.revision.get() || current.view_epoch != self.epoch.get() {
            return Err("富文本视图版本已过期，请切回源码后重新打开富文本".into());
        }
        let old = session.state();
        self.projection.borrow().inherit_replacement_link(&mut tr)?;
        let rule_input = tr.get_meta("inputRule$").cloned();
        let (next, applied) = old.apply_transaction(tr).map_err(|e| e.to_string())?;
        let p = self.projection.borrow();
        if old.doc() != &p.doc {
            return Err("富文本视图已过期，请切回源码后重新打开富文本".into());
        }
        let mut edits = p.transaction_edits(next.doc(), &applied)?;
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
            let mut projected = Projection::parse(next.schema(), raw)?;
            if &projected.doc != next.doc() {
                return Err("该富文本操作不能保真往返；请在源码中编辑此结构".into());
            }
            let mut a = projected
                .source_position(next.selection().anchor())
                .ok_or("选区起点暂不能保真映射，请切到源码编辑此范围")?;
            let mut h = projected
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
            // A text input and its automatic formatting are distinct commands.
            // Publish the typed spelling first so one Undo of the conversion
            // restores it (including the trigger space), using the same Store.
            // Validate both source projections before the first mutation.
            let mut typed_fallback = None;
            if let Some(meta) =
                rule_input.filter(|m| m["text"].as_str().is_some_and(|t| !t.is_empty()))
            {
                let from = meta["from"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or("无效输入规则起点")?;
                let to = meta["to"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or("无效输入规则终点")?;
                let text = meta["text"].as_str().ok_or("无效输入规则正文")?;
                let mut typed = old.tr();
                typed.insert_text(text, Some(from), Some(to));
                let (typed_state, typed_applied) =
                    old.apply_transaction(typed).map_err(|e| e.to_string())?;
                if typed_state.doc() != next.doc() {
                    let input_edits = p.transaction_edits(typed_state.doc(), &typed_applied)?;
                    let mut typed_raw = p.source.clone();
                    for (range, text) in input_edits.iter().rev() {
                        typed_raw.replace_range(range.clone(), text);
                    }
                    let typed_projection = Projection::parse(next.schema(), typed_raw)?;
                    let (range, text) = typed_projection
                        .patch(next.doc())?
                        .ok_or("输入规则没有可提交的格式变化")?;
                    let mut final_raw = typed_projection.source.clone();
                    final_raw.replace_range(range.clone(), &text);
                    let final_projection = Projection::parse(next.schema(), final_raw)?;
                    if final_projection.doc != *next.doc() {
                        return Err("输入规则不能保真往返，原文未变".into());
                    }
                    let typed_a = typed_projection
                        .source_position(typed_state.selection().anchor())
                        .ok_or("输入位置不能映射")?;
                    let typed_h = typed_projection
                        .source_position(typed_state.selection().head())
                        .ok_or("输入位置不能映射")?;
                    a = final_projection
                        .source_position(next.selection().anchor())
                        .ok_or("格式位置不能映射")?;
                    h = final_projection
                        .source_position(next.selection().head())
                        .ok_or("格式位置不能映射")?;
                    if current.revision > u64::MAX - 2 || current.view_epoch > u64::MAX - 2 {
                        return Err("输入版本计数耗尽".into());
                    }
                    let accepted = self.native.edit(
                        current.revision,
                        current.view_epoch,
                        input_edits,
                        ExternalSelections {
                            primary: 0,
                            ranges: vec![(typed_a, typed_h)],
                        },
                        ExternalEditKind::Typing,
                    )?;
                    current = accepted.snapshot;
                    self.native.fence();
                    typed_fallback = Some((typed_state, typed_applied, typed_projection));
                    projected = final_projection;
                    edits = vec![(range, text)];
                }
            }
            let update_result = self.native.edit(
                current.revision,
                current.view_epoch,
                edits,
                ExternalSelections {
                    primary: 0,
                    ranges: vec![(a, h)],
                },
                kind,
            );
            let update = match update_result {
                Ok(update) => update,
                Err(error) => {
                    if let Some((typed_state, typed_applied, typed_projection)) = typed_fallback {
                        self.revision.set(current.revision);
                        self.epoch.set(current.view_epoch);
                        drop(p);
                        *self.projection.borrow_mut() = typed_projection;
                        session.commit(typed_state, &typed_applied);
                        self.native.notify();
                        return Err(format!("已保留文字输入；自动格式未执行：{error}"));
                    }
                    return Err(error);
                }
            };
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
    Some((p.nearest_rich_position(a)?, p.nearest_rich_position(h)?))
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
        let origin = self.source.snapshot()?;
        let state = next.session.state();
        let selection = restored_selection(&next.projection.borrow(), &origin.selections)
            .ok_or("当前源码位置不能映射到正文，已保留源码模式")?;
        let mut tr = state.tr();
        tr.set_selection(TextSelection::create(
            state.doc(),
            selection.0,
            Some(selection.1),
        ));
        let restored = state.apply(tr).map_err(|e| e.to_string())?;
        next.session.update_state(restored);
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
        let transferred = (|| {
            let rich = self.rich.snapshot()?;
            let source = self.source.snapshot()?;
            if rich.revision != source.revision {
                return Err("视图版本不一致，已保留正文模式".into());
            }
            self.source
                .select(source.revision, source.view_epoch, rich.selections)?;
            Ok::<_, String>(())
        })();
        if let Err(e) = transferred {
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
    pub fn shell_history(&self, undo: bool) -> Result<(), String> {
        if self.busy() {
            return Err("请先完成或取消中文组合，再撤销或重做".into());
        }
        if self.rich_mode.get() {
            let active = self.active.borrow();
            let owner = active.as_ref().ok_or("正文视图尚未准备完成")?;
            owner.history(undo);
            let status = owner.status.get();
            if !status.is_empty() {
                return Err(status);
            }
        } else {
            let current = self.source.snapshot()?;
            self.source
                .history(current.revision, current.view_epoch, undo)?;
            self.source.notify();
        }
        Ok(())
    }
    pub fn find_literal(&self, query: &str, previous: bool) -> Result<(usize, usize), String> {
        if self.busy() {
            return Err("请先完成或取消中文组合".into());
        }
        if self.rich_mode.get() {
            self.active
                .borrow()
                .as_ref()
                .ok_or("正文视图未就绪")?
                .find_literal(query, previous)
        } else {
            self.source.find_literal(query, previous)
        }
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
    fn external_source_view_updates_live_rich_projection_without_a_second_history() {
        run(|| {
            let source = NativeView::fixture("apple\r\n\n<custom>keep</custom>\n");
            let other = source.sibling_source().unwrap();
            let rich = RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
            rich.activate().unwrap();
            let before = other.snapshot().unwrap();
            batch(|| {
                other
                    .edit(
                        before.revision,
                        before.view_epoch,
                        vec![(0..5, "pear".into())],
                        ExternalSelections {
                            primary: 0,
                            ranges: vec![(4, 4)],
                        },
                        ExternalEditKind::Other,
                    )
                    .unwrap();
                other.notify();
            });
            assert_eq!(
                rich.session.state().doc().text_content(),
                "pear<custom>keep</custom>"
            );
            assert_eq!(rich.revision.get(), source.snapshot().unwrap().revision);
            assert_eq!(bytes(&source), b"pear\r\n\n<custom>keep</custom>\n");
            assert_eq!(
                source.inspect(|s, id| s.history_depth(s.view(id).unwrap().doc).unwrap()),
                (1, 0)
            );
            other.undo_redo(true).unwrap();
            assert_eq!(bytes(&source), b"apple\r\n\n<custom>keep</custom>\n");
            assert_eq!(rich.revision.get(), source.snapshot().unwrap().revision);
        });
    }
    #[test]
    fn rich_find_uses_visible_text_without_mode_edits_or_url_matches() {
        run(|| {
            let raw =
                "\u{feff}**apple**\r\n\r\npear\n\n[label](https://example.invalid/hidden)\r\n";
            let source = NativeView::fixture(raw);
            let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
            let before = source.inspect(|store, id| {
                let doc = store.view(id).unwrap().doc;
                (
                    store.snapshot(doc).unwrap().token,
                    store.history_depth(doc).unwrap(),
                    store.dirty(doc).unwrap(),
                )
            });
            gate.to_rich();
            assert!(gate.rich_mode.get(), "{}", gate.status.get());
            assert_eq!(gate.find_literal("apple\npear", false).unwrap(), (1, 1));
            assert_eq!(gate.find_literal("example.invalid", false).unwrap(), (0, 0));
            assert_eq!(gate.find_literal("label", false).unwrap(), (1, 1));
            assert!(gate.rich_mode.get());
            assert_eq!(bytes(&source), raw.as_bytes());
            assert_eq!(
                before,
                source.inspect(|store, id| {
                    let doc = store.view(id).unwrap().doc;
                    (
                        store.snapshot(doc).unwrap().token,
                        store.history_depth(doc).unwrap(),
                        store.dirty(doc).unwrap(),
                    )
                })
            );
        });
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
    fn empty_container_input_escapes_locally_without_binding_another_code_leaf() {
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
                let mut tr = owner.session.state().tr();
                tr.insert_text("*a*", None, None);
                owner.session.dispatch(tr).unwrap();
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                let expected = if raw.starts_with('|') {
                    raw.replace("|  |  |", "| \\*a\\* |  |")
                } else {
                    raw.replacen("+ \r\n", "+ \\*a\\*\r\n", 1)
                };
                assert_eq!(bytes(&source), expected.as_bytes());
                assert_eq!(
                    owner
                        .session
                        .state()
                        .doc()
                        .text_content()
                        .matches("*a*")
                        .count(),
                    2
                );
                owner.history(true);
                assert!(owner.status.get().is_empty());
                assert_eq!(bytes(&source), raw.as_bytes());
            }
        });
    }
    #[test]
    fn adjacent_empty_fence_first_text_is_one_source_history_entry() {
        run(|| {
            // round-empty-code-01: two Source lines, no blank code-body line
            // and no final EOL; the failed Rich input must have its own undo.
            assert_empty_fence_input_source_roundtrip("~~~rust\r\n~~~", "~~~rust\r\n戊\r\n~~~");
        });
    }
    #[test]
    fn empty_fence_input_preserves_marker_eol_blank_body_and_terminal_eol_controls() {
        run(|| {
            for eol in ["\n", "\r\n"] {
                for fence in ["~~~", "```"] {
                    for body_separator in ["", eol] {
                        for ending in ["", eol] {
                            let raw = format!("{fence}rust{eol}{body_separator}{fence}{ending}");
                            let expected = format!("{fence}rust{eol}戊{eol}{fence}{ending}");
                            assert_empty_fence_input_source_roundtrip(&raw, &expected);
                        }
                    }
                }
            }
        });
    }
    fn assert_empty_fence_input_source_roundtrip(raw: &str, expected: &str) {
        let source = NativeView::fixture(raw);
        let rich = source.sibling().unwrap();
        let owner = RichOwner::new(rich.clone(), signal(String::new())).unwrap();
        owner.activate().unwrap();
        select(&owner, 1);
        let state = owner.session.state();
        assert_eq!(state.doc().child_count(), 1, "fixture {raw:?}");
        assert_eq!(state.doc().child(0).ty().name(), "codeBlock");
        assert_eq!(state.doc().child(0).text_content(), "");
        assert_eq!(state.selection().anchor(), 1);
        assert_eq!(state.selection().head(), 1);
        let body_at = source
            .snapshot()
            .unwrap()
            .rope
            .to_string()
            .find('\n')
            .unwrap()
            + 1;
        assert_eq!(owner.projection.borrow().source_position(1), Some(body_at));
        assert_eq!(
            rich.snapshot().unwrap().selections.ranges,
            vec![(body_at, body_at)]
        );
        // Start with an imported fixture, not Source typing history: one undo
        // below can only remove this first Rich input, never the old fences.
        assert_eq!(source.can_history(), (false, false));
        let identity = || {
            source.inspect(|store, id| {
                let snapshot = store.snapshot(store.view(id).unwrap().doc).unwrap();
                let origins = snapshot
                    .body
                    .origins()
                    .into_iter()
                    .map(|(range, origin)| {
                        (range, origin.map(|o| (o.source, o.raw, o.logical_len)))
                    })
                    .collect::<Vec<_>>();
                (snapshot.token, snapshot.body.eols(), origins)
            })
        };
        let original_identity = identity();
        let original_revision = source.snapshot().unwrap().revision;
        let mut tr = state.tr();
        // Match the native Text input transaction rather than an API-origin
        // edit, so the real Owner submits a single Typing edit to the Store.
        tr.insert_text("戊", None, None);
        tr.set_origin(TransactionOrigin::Input);
        tr.scroll_into_view();
        owner.session.dispatch(tr).unwrap();
        assert!(
            owner.status.get().is_empty(),
            "fixture {raw:?}: {}",
            owner.status.get()
        );
        assert_eq!(bytes(&source), expected.as_bytes(), "fixture {raw:?}");
        assert_eq!(bytes(&rich), expected.as_bytes());
        assert_eq!(source.snapshot().unwrap().revision, original_revision + 1);
        assert_eq!(owner.session.state().doc().child(0).text_content(), "戊");
        assert_eq!(owner.session.state().selection().anchor(), 2);
        assert_eq!(owner.session.state().selection().head(), 2);
        assert_eq!(
            rich.snapshot().unwrap().selections.ranges,
            vec![(body_at + "戊".len(), body_at + "戊".len())]
        );
        let text_identity = identity();
        assert_ne!(text_identity.0, original_identity.0);
        source.inspect(|store, id| {
            let snapshot = store.snapshot(store.view(id).unwrap().doc).unwrap();
            assert_eq!(store.history_depth(snapshot.doc).unwrap(), (1, 0));
            let mut kept_original = Vec::new();
            for (_, origin) in snapshot.body.origins() {
                if let Some(origin) = origin {
                    assert_eq!(origin.source.as_ref(), raw);
                    kept_original.extend_from_slice(&origin.source.as_bytes()[origin.raw]);
                }
            }
            assert_eq!(kept_original, raw.as_bytes());
        });
        assert!(
            nsh_base::rich_editor::extensions::history::history_state(&owner.session.state())
                .is_none()
        );
        let final_doc = owner.session.state().doc().to_json();
        // Source alone drives the same Store history; token, exact EOL vector
        // and every origin range must return with the corresponding bytes.
        for (undo, expected_bytes, expected_identity, depth) in [
            (true, raw, &original_identity, (0, 1)),
            (false, expected, &text_identity, (1, 0)),
        ] {
            let snapshot = source.snapshot().unwrap();
            source
                .history(snapshot.revision, snapshot.view_epoch, undo)
                .unwrap();
            assert_eq!(bytes(&source), expected_bytes.as_bytes());
            assert_eq!(bytes(&rich), expected_bytes.as_bytes());
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
        assert_eq!(reopened.session.state().doc().to_json(), final_doc);
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
    fn escaped_entity_and_duplicate_leaf_edits_use_exact_source_spans_and_shared_history() {
        run(|| {
            for (raw, find, offset, replacement, expected) in [
                ("a &amp; b\r\n", "&amp;", 0, "和", "a 和 b\r\n"),
                (
                    "a \\*b\\* &amp; end\r\n",
                    "b",
                    0,
                    "B",
                    "a \\*B\\* &amp; end\r\n",
                ),
                ("**foo** foo\n", "foo\n", 0, "F", "**foo** Foo\n"),
                (
                    "> + **foo &amp; 甲**\r\n>   + foo \\*字\\*\n",
                    "foo \\*",
                    0,
                    "F",
                    "> + **foo &amp; 甲**\r\n>   + Foo \\*字\\*\n",
                ),
                (
                    "[foo &amp; 甲][id]\r\n\r\n[id]: <https://example.invalid> \"原题\"\n",
                    "foo",
                    0,
                    "F",
                    "[Foo &amp; 甲][id]\r\n\r\n[id]: <https://example.invalid> \"原题\"\n",
                ),
            ] {
                let source = NativeView::fixture(raw);
                let rich = source.sibling().unwrap();
                let owner = RichOwner::new(rich.clone(), signal(String::new())).unwrap();
                owner.activate().unwrap();
                let logical = source.snapshot().unwrap().rope.to_string();
                let at = if find.ends_with('\n') {
                    logical.rfind(find.trim_end_matches('\n')).unwrap()
                } else {
                    logical.find(find).unwrap()
                } + offset;
                let pos = owner.projection.borrow().rich_position(at).unwrap();
                select(&owner, pos);
                let original_token =
                    source.inspect(|s, id| s.snapshot(s.view(id).unwrap().doc).unwrap().token);
                let mut tr = owner.session.state().tr();
                tr.insert_text(replacement, Some(pos), Some(pos + 1));
                owner.session.dispatch(tr).unwrap();
                assert!(
                    owner.status.get().is_empty(),
                    "{raw:?}: {}",
                    owner.status.get()
                );
                assert_eq!(bytes(&source), expected.as_bytes(), "{raw:?}");
                assert_eq!(bytes(&rich), expected.as_bytes());
                source.inspect(|s, id| {
                    assert!(s.dirty(s.view(id).unwrap().doc).unwrap());
                    assert_eq!(s.history_depth(s.view(id).unwrap().doc).unwrap(), (1, 0));
                });
                owner.history(true);
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                assert_eq!(bytes(&source), raw.as_bytes());
                source.inspect(|s, id| {
                    assert_eq!(
                        s.snapshot(s.view(id).unwrap().doc).unwrap().token,
                        original_token
                    );
                    assert!(!s.dirty(s.view(id).unwrap().doc).unwrap());
                });
                owner.history(false);
                assert!(owner.status.get().is_empty());
                assert_eq!(bytes(&source), expected.as_bytes());
            }
        });
    }
    fn type_text(owner: &RichOwner, text: &str) {
        let state = owner.session.state();
        let (from, to) = (state.selection().from(), state.selection().to());
        let mut fired = None;
        let handled = state.plugins().iter().any(|plugin| {
            plugin.handle_text_input(&state, from, to, text, &mut |tr| fired = Some(tr))
        });
        let mut tr = if handled {
            fired.unwrap()
        } else {
            let mut tr = state.tr();
            tr.insert_text(text, Some(from), Some(to));
            tr
        };
        if !handled {
            tr.set_origin(TransactionOrigin::Input);
        }
        owner.session.dispatch(tr).unwrap();
        assert!(
            owner.status.get().is_empty(),
            "input {text:?}: {}",
            owner.status.get()
        );
    }
    #[test]
    fn chinese_heading_and_next_paragraph_hashes_type_per_key_then_convert_and_undo() {
        run(|| {
            for imported in [false, true] {
                let raw = if imported {
                    "# 天命难违的眼镜框\r\n\r\n"
                } else {
                    ""
                };
                let source = NativeView::fixture(raw);
                let owner =
                    RichOwner::new(source.sibling().unwrap(), signal(String::new())).unwrap();
                owner.activate().unwrap();
                if !imported {
                    type_text(&owner, "#");
                    type_text(&owner, " ");
                    type_text(&owner, "天命难违的眼镜框");
                    split(&owner);
                } else {
                    let pos = owner.session.state().doc().child(0).node_size() + 1;
                    select(&owner, pos);
                }
                assert_eq!(owner.session.state().doc().child(0).ty().name(), "heading");
                assert_eq!(
                    owner.session.state().doc().child(0).text_content(),
                    "天命难违的眼镜框"
                );
                for count in 1..=3 {
                    type_text(&owner, "#");
                    assert_eq!(
                        owner.session.state().doc().child(1).ty().name(),
                        "paragraph"
                    );
                    assert_eq!(
                        owner.session.state().doc().child(1).text_content(),
                        "#".repeat(count)
                    );
                }
                let hashes = bytes(&source);
                type_text(&owner, " ");
                assert_eq!(owner.session.state().doc().child(1).ty().name(), "heading");
                assert_eq!(
                    owner.session.state().doc().child(1).attrs()["level"],
                    serde_json::json!(3)
                );
                owner.history(true);
                assert!(owner.status.get().is_empty(), "{}", owner.status.get());
                assert!(bytes(&source).starts_with(&hashes));
                assert!(
                    bytes(&source).len() > hashes.len(),
                    "Undo keeps the trigger space spelling"
                );
                assert_eq!(owner.session.state().doc().child(1).text_content(), "### ");
                owner.history(false);
                assert!(owner.status.get().is_empty());
                assert_eq!(owner.session.state().doc().child(1).ty().name(), "heading");
            }
        });
    }
    #[test]
    fn mode_switch_transfers_logical_selection_without_body_history_or_dirty_changes() {
        run(|| {
            let raw = "# 天命难违的眼镜框\r\n###\n\n末尾 &amp; \\*字\\*\r\n";
            let source = NativeView::fixture(raw);
            let rich = source.sibling().unwrap();
            let gate = ModeGate::new(source.clone(), rich.clone());
            let before = source.inspect(|s, id| {
                let d = s.view(id).unwrap().doc;
                (
                    s.snapshot(d).unwrap().token,
                    s.history_depth(d).unwrap(),
                    s.dirty(d).unwrap(),
                )
            });
            let logical = source.snapshot().unwrap().rope.to_string();
            let at = logical.find("末尾").unwrap();
            let snap = source.snapshot().unwrap();
            source
                .select(
                    snap.revision,
                    snap.view_epoch,
                    ExternalSelections {
                        primary: 0,
                        ranges: vec![(at, at)],
                    },
                )
                .unwrap();
            for _ in 0..100 {
                gate.to_rich();
                assert!(gate.rich_mode.get(), "{}", gate.status.get());
                assert_eq!(rich.snapshot().unwrap().selections.ranges, vec![(at, at)]);
                gate.to_source();
                assert!(!gate.rich_mode.get(), "{}", gate.status.get());
                assert_eq!(source.snapshot().unwrap().selections.ranges, vec![(at, at)]);
                gate.source_mounted();
            }
            let after = source.inspect(|s, id| {
                let d = s.view(id).unwrap().doc;
                (
                    s.snapshot(d).unwrap().token,
                    s.history_depth(d).unwrap(),
                    s.dirty(d).unwrap(),
                )
            });
            assert_eq!(before, after);
            assert_eq!(bytes(&source), raw.as_bytes());
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
