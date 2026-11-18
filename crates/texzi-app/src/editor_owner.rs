//! Native view adapter: one Store owns the body/history, NSH caches snapshots.
use nsh_base::editor::*;
use nsh_core::prelude::*;
use std::{cell::RefCell, ops::Range, rc::Rc, time::Instant};
use texzi_document as doc;

mod commands;

pub struct Owner {
    store: RefCell<doc::Store>,
    changed: Signal<u64>,
    clock: Instant,
}
pub struct NativeView {
    owner: Rc<Owner>,
    view: doc::ViewId,
    composing: Signal<bool>,
}
impl NativeView {
    pub fn new() -> Rc<Self> {
        Self::from_text("")
    }
    pub fn from_text(raw: &str) -> Rc<Self> {
        let mut store = doc::Store::new(doc::HistoryBudget::default());
        let s = store
            .open_utf8(raw, doc::Profile::default(), false)
            .expect("new document");
        let set = doc::SelectionSet::caret(&s, 0);
        let v = store
            .open_view(
                s.doc,
                doc::ModeTag::Source,
                set.clone(),
                set.selections[0].head,
                0,
            )
            .expect("new logical view");
        Rc::new(Self {
            owner: Rc::new(Owner {
                store: RefCell::new(store),
                changed: signal(0),
                clock: Instant::now(),
            }),
            view: v.id,
            composing: signal(false),
        })
    }
    pub fn buffer(self: &Rc<Self>) -> ExternalBuffer {
        ExternalBuffer(self.clone())
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
            composing: signal(false),
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
}
fn err(e: doc::Error) -> String {
    format!("{e:?}")
}
impl ExternalDocument for NativeView {
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
        if std::env::var_os("TEXZI_TRACE_IME").as_deref() == Some(std::ffi::OsStr::new("1")) {
            log::info!(
                "TX04_IME_OWNER action={} previous={} revision={} bytes={}",
                if undo { "undo" } else { "redo" },
                r.previous.0,
                r.snapshot.revision.0,
                r.snapshot.body.len_bytes()
            );
        }
        self.update(&store, r)
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
        if set != v.selection {
            store.set_selection(v.id, v.epoch, set).map_err(err)?;
        }
        self.read(&store)
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
