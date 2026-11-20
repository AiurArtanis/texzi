//! Only public write entrypoints; nested writes publish once at the outer return.
use crate::*;
impl Store {
    /// Resolver-supplied dual domains; still memory coordination, not disk proof.
    pub fn begin_scoped_save(
        &mut self,
        doc: DocId,
        target: TargetKey,
        guard: String,
        scope: SaveScope,
    ) -> Result<SaveCapture, Error> {
        self.observed_write(doc, |s| {
            s.begin_scoped_save_unpublished(doc, target, guard, scope)
        })
    }
    pub fn finish_save_failure(
        &mut self,
        c: &SaveCapture,
        detail: SaveFailure,
    ) -> Result<(), Error> {
        self.observed_write(c.snapshot.doc, |s| {
            s.finish_save_failure_unpublished(c, detail)
        })
    }
    /// Explicit owner acceptance of recovery responsibility for an ended,
    /// non-conflicting failure before replacement. Do not call automatically
    /// from a generic error handler or treat this as a worker result.
    ///
    /// The owner must retain the returned recovery data or explicitly decide
    /// to discard it before closing the document. The Store retains its own
    /// failure record and captured snapshot until normal document close.
    /// This neither saves or cleans up files nor clears an unknown outcome,
    /// releases publication resources, or changes any target/scope protection.
    /// It does not replace the UI's save/discard decision for current dirty
    /// content and does not close the document.
    ///
    /// Eligibility, snapshot consistency, conflict and active-save checks run
    /// before any mutation. A repeat is read-only, including at the observer
    /// counter boundary; a first acknowledgement must pass observed-write
    /// counter preflight before its flag can change.
    pub fn acknowledge_pre_replace_failure(
        &mut self,
        doc: DocId,
        operation: u64,
    ) -> Result<PreReplaceAcknowledgement, Error> {
        let acknowledgement = self.pre_replace_acknowledgement(doc, operation)?;
        if !acknowledgement.newly_acknowledged {
            return Ok(acknowledgement);
        }
        self.observed_write(doc, |s| {
            s.acknowledge_pre_replace_failure_unpublished(doc, operation)
        })
    }
    pub fn submit(&mut self, txn: Transaction) -> Result<Receipt, Error> {
        self.observed_write(txn.doc, |s| s.submit_unpublished(txn))
    }
    pub fn submit_view(&mut self, intent: ViewTransaction) -> Result<Receipt, Error> {
        self.observed_write(intent.transaction.doc, |s| {
            s.submit_view_unpublished(intent)
        })
    }
    pub fn set_serialization(
        &mut self,
        doc: DocId,
        base: Revision,
        profile: Profile,
        eol: Option<Eol>,
    ) -> Result<Receipt, Error> {
        self.observed_write(doc, |s| {
            s.set_serialization_unpublished(doc, base, profile, eol)
        })
    }
    pub fn undo(&mut self, doc: DocId) -> Result<Receipt, Error> {
        self.observed_write(doc, |s| s.undo_unpublished(doc))
    }
    pub fn redo(&mut self, doc: DocId) -> Result<Receipt, Error> {
        self.observed_write(doc, |s| s.redo_unpublished(doc))
    }
    pub fn undo_view(&mut self, id: ViewId, epoch: u64) -> Result<Receipt, Error> {
        let doc = self.view(id)?.doc;
        self.observed_write(doc, |s| s.undo_view_unpublished(id, epoch))
    }
    pub fn redo_view(&mut self, id: ViewId, epoch: u64) -> Result<Receipt, Error> {
        let doc = self.view(id)?.doc;
        self.observed_write(doc, |s| s.redo_view_unpublished(id, epoch))
    }
    pub fn begin_save(
        &mut self,
        doc: DocId,
        target: TargetKey,
        guard: String,
    ) -> Result<SaveCapture, Error> {
        self.observed_write(doc, |s| s.begin_save_unpublished(doc, target, guard))
    }
    pub fn advance_save(&mut self, c: &SaveCapture, next: SaveStage) -> Result<(), Error> {
        self.observed_write(c.snapshot.doc, |s| s.advance_save_unpublished(c, next))
    }
    pub fn commit_permit(&mut self, c: &SaveCapture) -> Result<(), Error> {
        self.observed_write(c.snapshot.doc, |s| s.commit_permit_unpublished(c))
    }
    pub fn cancel_save(&mut self, c: &SaveCapture) -> Result<(), Error> {
        self.observed_write(c.snapshot.doc, |s| s.cancel_save_unpublished(c))
    }
    pub fn accept_verified_save(&mut self, c: &SaveCapture) -> Result<(), Error> {
        self.observed_write(c.snapshot.doc, |s| s.accept_verified_save_unpublished(c))
    }
    pub fn open_view(
        &mut self,
        doc: DocId,
        mode: ModeTag,
        set: SelectionSet,
        scroll: Anchor,
        bias: i32,
    ) -> Result<ViewState, Error> {
        self.observed_write(doc, |s| {
            s.open_view_unpublished(doc, mode, set, scroll, bias)
        })
    }
    pub fn close_view(&mut self, id: ViewId, epoch: u64) -> Result<(), Error> {
        let doc = self.view(id)?.doc;
        self.observed_write(doc, |s| s.close_view_unpublished(id, epoch))
    }
    pub fn fence(&mut self, doc: DocId, reason: FenceReason) -> Result<(), Error> {
        self.observed_write(doc, |s| s.fence_unpublished(doc, reason))
    }
    pub fn set_selection(
        &mut self,
        id: ViewId,
        epoch: u64,
        set: SelectionSet,
    ) -> Result<ViewState, Error> {
        let doc = self.view(id)?.doc;
        self.observed_write(doc, |s| s.set_selection_unpublished(id, epoch, set))
    }
    pub fn set_scroll(
        &mut self,
        id: ViewId,
        epoch: u64,
        scroll: Anchor,
        bias: i32,
    ) -> Result<ViewState, Error> {
        let doc = self.view(id)?.doc;
        self.observed_write(doc, |s| s.set_scroll_unpublished(id, epoch, scroll, bias))
    }
    pub fn set_mode_tag(
        &mut self,
        id: ViewId,
        epoch: u64,
        mode: ModeTag,
    ) -> Result<ViewState, Error> {
        let doc = self.view(id)?.doc;
        self.observed_write(doc, |s| s.set_mode_tag_unpublished(id, epoch, mode))
    }
}
