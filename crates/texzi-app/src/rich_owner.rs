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
        state::{EditorState, EditorStateConfig, Transaction, TransactionOrigin},
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
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
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
        let state = EditorState::create(
            EditorStateConfig::new(m.schema().clone())
                .doc(p.doc.clone())
                .plugins(plugins),
        )
        .map_err(|e| e.to_string())?;
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
        let patch = p.patch(next.doc())?;
        if let Some((range, text)) = patch {
            let mut raw = p.source.clone();
            raw.replace_range(range.clone(), &text);
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
                vec![(range, text)],
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
        } else if let (Some(a), Some(h)) = (
            p.source_position(next.selection().anchor()),
            p.source_position(next.selection().head()),
        ) {
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
        match self
            .native
            .history(self.revision.get(), self.epoch.get(), undo)
        {
            Ok(u) => {
                let state = self.session.state();
                match Projection::parse(state.schema(), u.snapshot.rope.to_string()).and_then(|p| {
                    let new = EditorState::create(
                        EditorStateConfig::new(state.schema().clone())
                            .doc(p.doc.clone())
                            .plugins(state.plugins().to_vec()),
                    )
                    .map_err(|e| e.to_string())?;
                    Ok((p, new))
                }) {
                    Ok((p, new)) => {
                        self.revision.set(u.snapshot.revision);
                        self.epoch.set(u.snapshot.view_epoch);
                        *self.projection.borrow_mut() = p;
                        self.session.update_state(new);
                        self.native.notify();
                        self.status.set(String::new());
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
