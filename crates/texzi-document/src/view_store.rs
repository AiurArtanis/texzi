use crate::views::{snap_anchor, validate_anchor, validate_selection};
use crate::*;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT_VIEW: AtomicU64 = AtomicU64::new(1);

/// One synchronous read of the component writer. NOT a C08 ObserverSnapshot.
#[derive(Clone, Debug)]
pub struct DocumentViewSnapshot {
    pub document: Snapshot,
    pub views: Vec<ViewState>,
}

impl Store {
    pub fn read_document(&self, doc: DocId) -> Result<DocumentViewSnapshot, Error> {
        let d = self.docs.get(&doc).ok_or(Error::InvalidDoc)?;
        let mut views: Vec<_> = d.views.values().cloned().collect();
        views.sort_by_key(|v| v.id.0);
        Ok(DocumentViewSnapshot {
            document: d.current.clone(),
            views,
        })
    }
    pub fn open_view(
        &mut self,
        doc: DocId,
        mode: ModeTag,
        selection: SelectionSet,
        scroll: Anchor,
        pixel_bias: i32,
    ) -> Result<ViewState, Error> {
        let s = self.snapshot(doc)?;
        validate_selection(&s, &selection)?;
        validate_anchor(&s, scroll)?;
        let id = ViewId(
            NEXT_VIEW
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |x| x.checked_add(1))
                .map_err(|_| Error::CounterExhausted)?,
        );
        let view = ViewState {
            id,
            doc,
            epoch: 0,
            mode,
            selection,
            scroll,
            pixel_bias,
            mapping_snaps: vec![],
        };
        self.view_docs.insert(id, doc);
        self.docs
            .get_mut(&doc)
            .unwrap()
            .views
            .insert(id, view.clone());
        Ok(view)
    }
    pub fn view(&self, id: ViewId) -> Result<ViewState, Error> {
        let doc = self.view_docs.get(&id).ok_or(Error::InvalidView)?;
        self.docs
            .get(doc)
            .and_then(|d| d.views.get(&id))
            .cloned()
            .ok_or(Error::InvalidView)
    }
    fn checked_view(&self, id: ViewId, epoch: u64) -> Result<ViewState, Error> {
        let v = self.view(id)?;
        if v.epoch != epoch {
            return Err(Error::StaleView);
        }
        Ok(v)
    }
    pub fn close_view(&mut self, id: ViewId, epoch: u64) -> Result<(), Error> {
        let v = self.checked_view(id, epoch)?;
        self.docs.get_mut(&v.doc).unwrap().views.remove(&id);
        self.view_docs.remove(&id);
        self.fence(v.doc, FenceReason::CrossView)
    }
    pub fn fence(&mut self, doc: DocId, reason: FenceReason) -> Result<(), Error> {
        let d = self.docs.get_mut(&doc).ok_or(Error::InvalidDoc)?;
        d.merge = None;
        d.last_fence = reason;
        Ok(())
    }
    pub fn last_fence(&self, doc: DocId) -> Result<FenceReason, Error> {
        Ok(self.docs.get(&doc).ok_or(Error::InvalidDoc)?.last_fence)
    }
    pub fn set_selection(
        &mut self,
        id: ViewId,
        epoch: u64,
        set: SelectionSet,
    ) -> Result<ViewState, Error> {
        let mut v = self.checked_view(id, epoch)?;
        let s = self.snapshot(v.doc)?;
        validate_selection(&s, &set)?;
        v.epoch = v.epoch.checked_add(1).ok_or(Error::CounterExhausted)?;
        v.selection = set;
        v.mapping_snaps.clear();
        self.docs
            .get_mut(&v.doc)
            .unwrap()
            .views
            .insert(id, v.clone());
        self.fence(v.doc, FenceReason::Navigation)?;
        Ok(v)
    }
    pub fn set_scroll(
        &mut self,
        id: ViewId,
        epoch: u64,
        scroll: Anchor,
        pixel_bias: i32,
    ) -> Result<ViewState, Error> {
        let mut v = self.checked_view(id, epoch)?;
        validate_anchor(&self.snapshot(v.doc)?, scroll)?;
        v.epoch = v.epoch.checked_add(1).ok_or(Error::CounterExhausted)?;
        v.scroll = scroll;
        v.pixel_bias = pixel_bias;
        v.mapping_snaps.clear();
        self.docs
            .get_mut(&v.doc)
            .unwrap()
            .views
            .insert(id, v.clone());
        self.fence(v.doc, FenceReason::Navigation)?;
        Ok(v)
    }
    /// Changes only a logical test/component label, not an NSH mode/projection.
    pub fn set_mode_tag(
        &mut self,
        id: ViewId,
        epoch: u64,
        mode: ModeTag,
    ) -> Result<ViewState, Error> {
        let mut v = self.checked_view(id, epoch)?;
        v.epoch = v.epoch.checked_add(1).ok_or(Error::CounterExhausted)?;
        v.mode = mode;
        self.docs
            .get_mut(&v.doc)
            .unwrap()
            .views
            .insert(id, v.clone());
        self.fence(v.doc, FenceReason::Mode)?;
        Ok(v)
    }
    pub fn submit_view(&mut self, intent: ViewTransaction) -> Result<Receipt, Error> {
        let v = self.checked_view(intent.view, intent.expected_epoch)?;
        if v.doc != intent.transaction.doc {
            return Err(Error::InvalidView);
        }
        if intent.transaction.boundary != BoundaryPolicy::Grapheme {
            return Err(Error::InvalidSelection);
        }
        self.submit_inner(intent.transaction.clone(), Some(&intent))
    }
    /// One replacement intent per normalized selection, with explicit post-edit
    /// collapsed carets. Exact invalid joins are rejected, never silently snapped.
    pub fn plan_replace(
        &self,
        id: ViewId,
        raw: &str,
        kind: InputKind,
        tick_ms: u64,
    ) -> Result<ViewTransaction, Error> {
        let v = self.view(id)?;
        let s = self.snapshot(v.doc)?;
        let mut body = (*s.body).clone();
        let mut edits = Vec::new();
        let mut changed = false;
        let mut shift = 0isize;
        let mut carets = Vec::new();
        for selection in &v.selection.selections {
            let range = selection.range();
            let edit = Edit::input(&s, range.clone(), raw);
            changed |= s.body.source().byte_slice(range.clone()).to_string() != edit.text
                || s.body.eols_in(range.clone()) != edit.eols;
            let at = range
                .start
                .checked_add_signed(shift)
                .ok_or(Error::InvalidBoundary)?
                + edit.text.len();
            carets.push(at);
            shift += edit.text.len() as isize - (range.end - range.start) as isize;
            edits.push(edit);
        }
        for e in edits.iter().rev() {
            body.replace(e.start.offset..e.end.offset, &e.text, &e.eols);
        }
        let preview = Snapshot {
            doc: s.doc,
            revision: Revision(if changed {
                s.revision.0.checked_add(1).ok_or(Error::CounterExhausted)?
            } else {
                s.revision.0
            }),
            token: s.token,
            body: std::sync::Arc::new(body),
        };
        let selection_after = normalize_selections(
            &preview,
            SelectionSet {
                selections: carets
                    .into_iter()
                    .map(|at| Selection::caret(&preview, at))
                    .collect(),
                primary: v.selection.primary,
            },
        )?;
        Ok(ViewTransaction {
            view: id,
            expected_epoch: v.epoch,
            transaction: Transaction {
                doc: s.doc,
                base: s.revision,
                edits,
                boundary: BoundaryPolicy::Grapheme,
                cancelled: false,
            },
            selection_after,
            kind,
            tick_ms,
        })
    }
    pub fn undo_view(&mut self, id: ViewId, epoch: u64) -> Result<Receipt, Error> {
        let v = self.checked_view(id, epoch)?;
        self.travel(v.doc, false, Some(id))
    }
    pub fn redo_view(&mut self, id: ViewId, epoch: u64) -> Result<Receipt, Error> {
        let v = self.checked_view(id, epoch)?;
        self.travel(v.doc, true, Some(id))
    }

    pub(crate) fn prepare_views(
        &self,
        r: &Receipt,
        override_set: Option<(ViewId, &SelectionSet)>,
    ) -> Result<HashMap<ViewId, ViewState>, Error> {
        let d = self.docs.get(&r.snapshot.doc).ok_or(Error::InvalidDoc)?;
        if let Some((id, set)) = override_set {
            if !d.views.contains_key(&id) {
                return Err(Error::InvalidView);
            }
            validate_selection(&r.snapshot, set)?;
        }
        let mut out = d.views.clone();
        for v in out.values_mut() {
            let overridden = override_set
                .filter(|(id, _)| *id == v.id)
                .map(|(_, set)| set);
            if r.changed {
                let mut snaps = Vec::new();
                let map = |a: Anchor, snaps: &mut Vec<AnchorSnap>| -> Result<Anchor, Error> {
                    let mapped = Anchor {
                        position: r.map_anchor(a.position, a.affinity)?,
                        affinity: a.affinity,
                    };
                    let (anchor, snap) = snap_anchor(&r.snapshot, mapped)?;
                    if let Some(s) = snap {
                        snaps.push(s);
                    }
                    Ok(anchor)
                };
                if overridden.is_none() {
                    let selections = v
                        .selection
                        .selections
                        .iter()
                        .map(|sel| {
                            Ok(Selection {
                                anchor: map(sel.anchor, &mut snaps)?,
                                head: map(sel.head, &mut snaps)?,
                            })
                        })
                        .collect::<Result<_, Error>>()?;
                    v.selection = normalize_selections(
                        &r.snapshot,
                        SelectionSet {
                            selections,
                            primary: v.selection.primary,
                        },
                    )?;
                }
                v.scroll = map(v.scroll, &mut snaps)?;
                v.mapping_snaps = snaps;
            }
            let selection_changed = overridden.is_some_and(|s| s != &v.selection);
            if let Some(set) = overridden {
                v.selection = set.clone();
            }
            if r.changed || selection_changed {
                v.epoch = v.epoch.checked_add(1).ok_or(Error::CounterExhausted)?;
            }
        }
        Ok(out)
    }
}
