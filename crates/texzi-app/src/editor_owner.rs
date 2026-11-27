//! Native view adapter: one Store owns the body/history, NSH caches snapshots.
use nsh_base::editor::*;
use nsh_core::prelude::*;
use std::{cell::RefCell, ops::Range, rc::Rc, time::Instant};
use texzi_document as doc;

mod cold_history;
mod commands;
mod files;
mod search;
pub(crate) use files::SaveError;
pub(crate) use files::{MAX_EDIT_BYTES, PrepareError, PreparedFile, line_limit_message};
pub(crate) use search::SearchScope;

pub struct Owner {
    loading_history: std::cell::Cell<bool>,
    search_scopes: search::ScopeRegistry,
    store: RefCell<doc::Store>,
    changed: Signal<u64>,
    clock: Instant,
    files: RefCell<files::FileState>,
}
pub struct NativeView {
    history_target: RefCell<Option<Rc<cold_history::Target>>>,
    owner: Rc<Owner>,
    view: doc::ViewId,
    composing: Signal<bool>,
    selection_changed: Signal<u64>,
}
impl NativeView {
    #[cfg(test)]
    pub fn from_text(raw: &str) -> Rc<Self> {
        let mut store = doc::Store::new(doc::HistoryBudget::default());
        let s = store
            .open_utf8(raw, doc::Profile::default(), false)
            .expect("new document");
        Self::from_store(store, s, files::FileState::default()).expect("new logical view")
    }
    fn from_store(
        mut store: doc::Store,
        s: doc::Snapshot,
        file_state: files::FileState,
    ) -> Result<Rc<Self>, String> {
        let set = doc::SelectionSet::caret(&s, 0);
        let v = store
            .open_view(
                s.doc,
                doc::ModeTag::Source,
                set.clone(),
                set.selections[0].head,
                0,
            )
            .map_err(err)?;
        Ok(Rc::new(Self {
            owner: Rc::new(Owner {
                loading_history: std::cell::Cell::new(false),
                search_scopes: RefCell::default(),
                store: RefCell::new(store),
                changed: signal(0),
                clock: Instant::now(),
                files: RefCell::new(file_state),
            }),
            view: v.id,
            history_target: RefCell::default(),
            composing: signal(false),
            selection_changed: signal(0),
        }))
    }
    pub fn buffer(self: &Rc<Self>) -> ExternalBuffer {
        ExternalBuffer(self.clone())
    }
    pub(crate) fn current_body(&self) -> Option<std::sync::Arc<doc::Body>> {
        self.track();
        let store = self.owner.store.borrow();
        let view = store.view(self.view).ok()?;
        Some(store.snapshot(view.doc).ok()?.body)
    }
    pub fn is_composing(&self) -> bool {
        self.composing.get()
    }
    pub fn set_composing(&self, active: bool) {
        self.composing.set(active);
        if std::env::var_os("TEXZI_TRACE_IME").as_deref() == Some(std::ffi::OsStr::new("1")) {
            log::info!("TX05_COMPOSITION view={:?} active={}", self.view, active);
        }
    }
    #[cfg(test)]
    pub fn fixture(raw: &str) -> Rc<Self> {
        Self::from_text(raw)
    }
    #[cfg(test)]
    pub fn inspect<R>(&self, f: impl FnOnce(&doc::Store, doc::ViewId) -> R) -> R {
        f(&self.owner.store.borrow(), self.view)
    }
    pub(crate) fn has_single_caret(&self) -> bool {
        self.track();
        self.selection_changed.get();
        self.owner.store.borrow().view(self.view).is_ok_and(|view| {
            matches!(
                view.selection.selections.as_slice(),
                [selection] if selection.anchor.position.offset == selection.head.position.offset
            )
        })
    }
    pub(crate) fn cursor_status(&self) -> Option<String> {
        self.track();
        self.selection_changed.get();
        let store = self.owner.store.borrow();
        let view = store.view(self.view).ok()?;
        if view.selection.selections.len() > 1 {
            return Some(format!("{} 个光标", view.selection.selections.len()));
        }
        let head = view
            .selection
            .selections
            .get(view.selection.primary)?
            .head
            .position
            .offset;
        let (line, column) = store.snapshot(view.doc).ok()?.body.line_column(head).ok()?;
        Some(format!("行 {line}，列 {column}"))
    }
    pub fn trace_state(&self, action: &str) {
        if std::env::var_os("TEXZI_TRACE_IME").as_deref() != Some(std::ffi::OsStr::new("1")) {
            return;
        }
        let store = self.owner.store.borrow();
        if let Ok(v) = store.view(self.view) {
            if let (Ok(s), Ok((undo, redo)), Ok(dirty)) = (
                store.snapshot(v.doc),
                store.history_depth(v.doc),
                store.dirty(v.doc),
            ) {
                log::info!(
                    "TX05_VIEW action={} revision={} token={:?} bytes={} undo={} redo={} dirty={}",
                    action,
                    s.revision.0,
                    s.token,
                    s.body.len_bytes(),
                    undo,
                    redo,
                    dirty
                );
            }
        }
    }
    pub fn sibling(&self) -> Result<Rc<Self>, String> {
        self.sibling_mode(doc::ModeTag::Rich)
    }
    pub fn sibling_source(&self) -> Result<Rc<Self>, String> {
        self.sibling_mode(doc::ModeTag::Source)
    }
    fn sibling_mode(&self, mode: doc::ModeTag) -> Result<Rc<Self>, String> {
        let mut store = self.owner.store.borrow_mut();
        let source = store.view(self.view).map_err(err)?;
        let v = store
            .open_view(
                source.doc,
                mode,
                source.selection,
                source.scroll,
                source.pixel_bias,
            )
            .map_err(err)?;
        Ok(Rc::new(Self {
            owner: self.owner.clone(),
            view: v.id,
            history_target: RefCell::default(),
            composing: signal(false),
            selection_changed: signal(0),
        }))
    }
    fn read(&self, store: &doc::Store) -> Result<ExternalSnapshot, String> {
        let v = store.view(self.view).map_err(err)?;
        let s = store.snapshot(v.doc).map_err(err)?;
        Ok(ExternalSnapshot {
            revision: s.revision.0,
            view_epoch: v.epoch,
            rope: s.body.source().clone(),
            selections: ExternalSelections {
                primary: v.selection.primary,
                ranges: v
                    .selection
                    .selections
                    .iter()
                    .map(|s| (s.anchor.position.offset, s.head.position.offset))
                    .collect(),
            },
        })
    }
    fn update(&self, store: &doc::Store, r: doc::Receipt) -> Result<ExternalUpdate, String> {
        self.advance_search_scopes(&r);
        if r.is_bulk() {
            return Ok(ExternalUpdate {
                snapshot: self.read(store)?,
                edits: None,
            });
        }
        // Document deltas use the same old version; descending order is sequential.
        let edits = r
            .forward
            .into_iter()
            .rev()
            .map(|d| (d.range, d.text))
            .collect();
        Ok(ExternalUpdate {
            snapshot: self.read(store)?,
            edits: Some(edits),
        })
    }
    fn history_update(
        &self,
        store: &doc::Store,
        r: doc::Receipt,
        undo: bool,
    ) -> Result<ExternalUpdate, String> {
        if std::env::var_os("TEXZI_TRACE_IME").as_deref() == Some(std::ffi::OsStr::new("1")) {
            log::info!(
                "TX04_IME_OWNER action={} previous={} revision={} bytes={}",
                if undo { "undo" } else { "redo" },
                r.previous.0,
                r.snapshot.revision.0,
                r.snapshot.body.len_bytes()
            );
        }
        self.update(store, r)
    }
}
fn err(e: doc::Error) -> String {
    format!("{e:?}")
}
impl ExternalDocument for NativeView {
    fn request_history(
        self: Rc<Self>,
        revision: u64,
        epoch: u64,
        undo: bool,
    ) -> Result<Option<ExternalUpdate>, String> {
        self.request_owned_history(revision, epoch, undo)
    }
    fn composition_changed(&self, active: bool) {
        self.set_composing(active)
    }
    fn snapshot(&self) -> Result<ExternalSnapshot, String> {
        self.read(&self.owner.store.borrow())
    }
    fn edit(
        &self,
        revision: u64,
        epoch: u64,
        edits: Vec<(Range<usize>, String)>,
        after: ExternalSelections,
        kind: ExternalEditKind,
    ) -> Result<ExternalUpdate, String> {
        let mut store = self.owner.store.borrow_mut();
        let kind = match kind {
            ExternalEditKind::Typing => doc::InputKind::Typing,
            ExternalEditKind::ImeCommit => doc::InputKind::ImeCommit,
            _ => doc::InputKind::Structural,
        };
        let tick = self.owner.clock.elapsed().as_millis().min(u64::MAX as u128) as u64;
        let plan = store
            .plan_view_edits(
                self.view,
                doc::Revision(revision),
                epoch,
                edits,
                after.ranges,
                after.primary,
                kind,
                tick,
            )
            .map_err(err)?;
        let r = store.submit_view(plan).map_err(err)?;
        if std::env::var_os("TEXZI_TRACE_IME").as_deref() == Some(std::ffi::OsStr::new("1")) {
            log::info!(
                "TX04_IME_OWNER action=submit kind={:?} previous={} revision={} bytes={}",
                kind,
                r.previous.0,
                r.snapshot.revision.0,
                r.snapshot.body.len_bytes()
            );
        }
        self.update(&store, r)
    }
    fn history(&self, revision: u64, epoch: u64, undo: bool) -> Result<ExternalUpdate, String> {
        let mut store = self.owner.store.borrow_mut();
        let v = store.view(self.view).map_err(err)?;
        if store.snapshot(v.doc).map_err(err)?.revision.0 != revision {
            return Err("StaleRevision".into());
        }
        let r = if undo {
            store.undo_view(self.view, epoch)
        } else {
            store.redo_view(self.view, epoch)
        }
        .map_err(err)?;
        self.history_update(&store, r, undo)
    }
    fn select(
        &self,
        revision: u64,
        epoch: u64,
        selections: ExternalSelections,
    ) -> Result<ExternalSnapshot, String> {
        let mut store = self.owner.store.borrow_mut();
        let v = store.view(self.view).map_err(err)?;
        let s = store.snapshot(v.doc).map_err(err)?;
        if s.revision.0 != revision {
            return Err("StaleRevision".into());
        }
        if v.epoch != epoch {
            return Err("StaleView".into());
        }
        let set = doc::normalize_selections(
            &s,
            doc::SelectionSet {
                primary: selections.primary,
                selections: selections
                    .ranges
                    .into_iter()
                    .map(|(a, h)| doc::Selection {
                        anchor: doc::Anchor {
                            position: doc::SourceByte::at(&s, a),
                            affinity: doc::Affinity::After,
                        },
                        head: doc::Anchor {
                            position: doc::SourceByte::at(&s, h),
                            affinity: doc::Affinity::After,
                        },
                    })
                    .collect(),
            },
        )
        .map_err(err)?;
        let changed = set != v.selection;
        if changed {
            store.set_selection(v.id, v.epoch, set).map_err(err)?;
        }
        let snapshot = self.read(&store);
        drop(store);
        if changed {
            self.selection_changed
                .update(|value| *value = value.wrapping_add(1));
        }
        snapshot
    }
    fn can_history(&self) -> (bool, bool) {
        let s = self.owner.store.borrow();
        s.view(self.view)
            .and_then(|v| s.history_depth(v.doc))
            .map(|(u, r)| (u > 0, r > 0))
            .unwrap_or_default()
    }
    fn fence(&self) {
        let mut s = self.owner.store.borrow_mut();
        if let Ok(v) = s.view(self.view) {
            let _ = s.fence(v.doc, doc::FenceReason::Navigation);
        }
    }
    fn track(&self) {
        self.owner.changed.get();
    }
    fn notify(&self) {
        self.owner.changed.update(|x| *x = x.wrapping_add(1));
    }
}
