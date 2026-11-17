use crate::{
    ActiveSaveState, BaselineKind, BaselineState, FileAttention, GlobalMetadata, HistoryGroupId,
    HistoryObservation, MergeObservation, SaveObservation, SaveResult, SaveTerminal, Unimplemented,
};
mod failure_ack;
#[cfg(windows)]
mod verified_open;
use crate::{
    Body, DocId, Eol, Error, Profile, Revision, SourceByte, StateToken, coordinates::boundary,
};
use crate::{FenceReason, InputKind, SelectionSet, ViewId, ViewState, ViewTransaction};
pub use failure_ack::PreReplaceAcknowledgement;
use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
#[cfg(windows)]
pub(crate) use verified_open::OpenDocument;

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub doc: DocId,
    pub revision: Revision,
    pub token: StateToken,
    pub body: Arc<Body>,
}
#[derive(Clone, Debug)]
pub struct Edit {
    pub start: SourceByte,
    pub end: SourceByte,
    pub text: String,
    pub eols: Vec<Eol>,
}
impl Edit {
    /// Paste/input preparation normalizes CRLF/CR and uses pre-submit dominant EOL.
    pub fn input(s: &Snapshot, range: Range<usize>, raw: &str) -> Self {
        let text = raw.replace("\r\n", "\n").replace('\r', "\n");
        let count = text.bytes().filter(|&b| b == b'\n').count();
        Self {
            start: SourceByte::at(s, range.start),
            end: SourceByte::at(s, range.end),
            text,
            eols: vec![s.body.dominant_eol(); count],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryPolicy {
    Grapheme,
    Scalar,
}
#[derive(Clone, Debug)]
pub struct Transaction {
    pub doc: DocId,
    pub base: Revision,
    pub edits: Vec<Edit>,
    pub boundary: BoundaryPolicy,
    pub cancelled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delta {
    pub range: Range<usize>,
    pub text: String,
    pub eols: Vec<Eol>,
}
#[derive(Clone, Debug)]
pub struct Receipt {
    pub history_group: Option<HistoryGroupId>,
    pub snapshot: Snapshot,
    pub previous: Revision,
    pub previous_body: Arc<Body>,
    pub forward: Vec<Delta>,
    pub inverse: Vec<Delta>,
    pub changed: bool,
}
#[derive(Clone)]
struct Entry {
    group: HistoryGroupId,
    selection_before: Option<SelectionSet>,
    selection_after: Option<SelectionSet>,
    before: Arc<Body>,
    after: Arc<Body>,
    old: StateToken,
    new: StateToken,
    forward: Vec<Delta>,
    inverse: Vec<Delta>,
    charge: usize,
}
pub(crate) struct Document {
    failures: Vec<SaveFailureRecord>,
    recovery_snapshots: Vec<(u64, Snapshot)>,
    save_blocked: bool,
    pub(crate) closed_views: std::collections::BTreeMap<ViewId, crate::ClosedView>,
    baseline_kind: BaselineKind,
    bound_target: Option<TargetKey>,
    binding_epoch: u64,
    save_terminal: Option<SaveTerminal>,
    pub(crate) views: HashMap<ViewId, ViewState>,
    pub(crate) merge: Option<MergeState>,
    pub(crate) last_fence: FenceReason,
    pub(crate) current: Snapshot,
    baseline: StateToken,
    backed: bool,
    undo: VecDeque<Entry>,
    redo: Vec<Entry>,
    charge: usize,
}

/// Accounting ceilings, not a measured process heap limit. No automatic disk spill.
#[derive(Clone, Copy, Debug)]
pub struct HistoryBudget {
    pub document: usize,
    pub process: usize,
}
impl Default for HistoryBudget {
    fn default() -> Self {
        Self {
            document: 64 * 1024 * 1024,
            process: 256 * 1024 * 1024,
        }
    }
}

/// All mutating APIs require &mut Store: one serial owner, no callback reentrancy.
pub struct Store {
    #[cfg(all(test, windows))]
    verified_open_doc_counter: Option<Arc<AtomicU64>>,
    #[cfg(windows)]
    opened_identities: HashMap<DocId, crate::files::windows_identity::OpenAnchor>,
    #[cfg(windows)]
    pub(crate) publication_recovery: Arc<crate::files::windows_save::recovery::Registry>,
    scope_parents: HashMap<TargetKey, ScopeLease>,
    scope_objects: HashMap<TargetKey, ScopeLease>,
    bound_targets: HashMap<TargetKey, DocId>,
    unresolved_targets: HashMap<TargetKey, DocId>,
    pub(crate) observer: crate::observe::ObserverHub,
    pub(crate) docs: HashMap<DocId, Document>,
    pub(crate) view_docs: HashMap<ViewId, DocId>,
    next_token: u64,
    budget: HistoryBudget,
    charge: usize,
    saves: HashMap<DocId, ActiveSave>,
    targets: HashMap<TargetKey, DocId>,
    next_save: u64,
}
static NEXT_DOC: AtomicU64 = AtomicU64::new(1);

/// Canonical identity supplied by a future T07 resolver; not a path normalizer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TargetKey(pub String);
/// Explicit, separate lock domains supplied by the filesystem resolver. Equality
/// of a file key and a directory key never stands in for a hierarchy check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveScope {
    pub parent: TargetKey,
    pub object: Option<TargetKey>,
}
#[derive(Clone, Debug)]
pub struct SaveCapture {
    pub operation: u64,
    pub snapshot: Snapshot,
    pub binding_epoch: u64,
    pub target: TargetKey,
    pub guard: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveStage {
    Encoding,
    WritingTemp,
    ReadyToReplace,
    Replacing,
    Verifying,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFailureOutcome {
    BeforeReplace,
    ReplaceFailed,
    OutcomeUnknown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveFailure {
    pub outcome: SaveFailureOutcome,
    pub conflict: bool,
    pub os_error: Option<i32>,
    pub message: String,
    pub recovery_paths: Vec<std::path::PathBuf>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveFailureRecord {
    pub scope: Option<SaveScope>,
    pub operation: u64,
    pub target: TargetKey,
    pub binding_epoch: u64,
    pub revision: Revision,
    pub token: StateToken,
    pub stage: SaveStage,
    pub detail: SaveFailure,
    /// Explicit owner acknowledgement of an eligible pre-replace failure only.
    /// This does not prove a disk save or release any filesystem protection.
    pub acknowledged_before_replace: bool,
}
struct ActiveSave {
    scope: Option<SaveScope>,
    capture: SaveCapture,
    stage: SaveStage,
}
#[derive(Clone, Copy)]
struct ScopeLease {
    operation: u64,
    held: bool,
}

impl Store {
    /// Memory-only save coordination. This method does not write or verify a file.
    pub(crate) fn begin_save_unpublished(
        &mut self,
        doc: DocId,
        target: TargetKey,
        guard: String,
    ) -> Result<SaveCapture, Error> {
        self.begin_save_with_scope(doc, target, guard, None)
    }
    pub(crate) fn begin_scoped_save_unpublished(
        &mut self,
        doc: DocId,
        target: TargetKey,
        guard: String,
        scope: SaveScope,
    ) -> Result<SaveCapture, Error> {
        self.begin_save_with_scope(doc, target, guard, Some(scope))
    }
    fn begin_save_with_scope(
        &mut self,
        doc: DocId,
        target: TargetKey,
        guard: String,
        scope: Option<SaveScope>,
    ) -> Result<SaveCapture, Error> {
        let s = self.snapshot(doc)?;
        if let Some(scope) = &scope {
            if scope
                .object
                .as_ref()
                .is_some_and(|object| object != &target)
            {
                return Err(Error::InvalidSave);
            }
            let parent = self.scope_parents.get(&scope.parent);
            let object = scope
                .object
                .as_ref()
                .and_then(|key| self.scope_objects.get(key));
            if parent.is_some_and(|lease| lease.held) || object.is_some_and(|lease| lease.held) {
                return Err(Error::UnresolvedConflict);
            }
            if parent.is_some() || object.is_some() {
                return Err(Error::Busy);
            }
        }
        if self.docs[&doc].save_blocked || self.unresolved_targets.contains_key(&target) {
            return Err(Error::UnresolvedConflict);
        }
        if self.saves.contains_key(&doc) || self.targets.contains_key(&target) {
            return Err(Error::Busy);
        }
        if self
            .bound_targets
            .get(&target)
            .is_some_and(|owner| *owner != doc)
        {
            return Err(Error::Busy);
        }
        let operation = self.next_save;
        let next = operation.checked_add(1).ok_or(Error::CounterExhausted)?;
        let capture = SaveCapture {
            operation,
            snapshot: s,
            binding_epoch: self.docs[&doc].binding_epoch,
            target: target.clone(),
            guard,
        };
        if let Some(scope) = &scope {
            let lease = ScopeLease {
                operation,
                held: false,
            };
            self.scope_parents.insert(scope.parent.clone(), lease);
            if let Some(object) = &scope.object {
                self.scope_objects.insert(object.clone(), lease);
            }
        }
        self.saves.insert(
            doc,
            ActiveSave {
                scope,
                capture: capture.clone(),
                stage: SaveStage::Encoding,
            },
        );
        self.targets.insert(target, doc);
        self.next_save = next;
        self.fence(doc, FenceReason::Save)?;
        Ok(capture)
    }
    fn active_save(&self, c: &SaveCapture) -> Result<&ActiveSave, Error> {
        let a = self.saves.get(&c.snapshot.doc).ok_or(Error::InvalidSave)?;
        if a.capture.operation != c.operation
            || a.capture.binding_epoch != c.binding_epoch
            || self.docs[&c.snapshot.doc].binding_epoch != c.binding_epoch
            || a.capture.target != c.target
            || a.capture.guard != c.guard
            || a.capture.snapshot.revision != c.snapshot.revision
            || a.capture.snapshot.token != c.snapshot.token
            || !Arc::ptr_eq(&a.capture.snapshot.body, &c.snapshot.body)
        {
            return Err(Error::InvalidSave);
        }
        Ok(a)
    }
    pub(crate) fn captured_save_stage(&self, c: &SaveCapture) -> Result<SaveStage, Error> {
        self.active_save(c).map(|a| a.stage)
    }
    pub(crate) fn permit_bound_new_unpublished(
        &mut self,
        c: &SaveCapture,
        scope: &SaveScope,
    ) -> Result<(), Error> {
        let active = self.active_save(c)?;
        if active.stage != SaveStage::WritingTemp
            || active.scope.as_ref() != Some(scope)
            || scope.object.is_some()
        {
            return Err(Error::InvalidSave);
        }
        self.docs[&c.snapshot.doc]
            .binding_epoch
            .checked_add(1)
            .ok_or(Error::CounterExhausted)?;
        self.advance_save_unpublished(c, SaveStage::ReadyToReplace)?;
        self.commit_permit_unpublished(c)
    }
    pub(crate) fn advance_save_unpublished(
        &mut self,
        c: &SaveCapture,
        next: SaveStage,
    ) -> Result<(), Error> {
        let stage = self.active_save(c)?.stage;
        if !matches!(
            (stage, next),
            (SaveStage::Encoding, SaveStage::WritingTemp)
                | (SaveStage::WritingTemp, SaveStage::ReadyToReplace)
                | (SaveStage::Replacing, SaveStage::Verifying)
        ) {
            return Err(Error::InvalidSave);
        }
        self.saves.get_mut(&c.snapshot.doc).unwrap().stage = next;
        Ok(())
    }
    /// The one-shot permit is represented by the transition; it cannot be reissued.
    pub(crate) fn commit_permit_unpublished(&mut self, c: &SaveCapture) -> Result<(), Error> {
        if self.active_save(c)?.stage != SaveStage::ReadyToReplace {
            return Err(Error::InvalidSave);
        }
        self.saves.get_mut(&c.snapshot.doc).unwrap().stage = SaveStage::Replacing;
        Ok(())
    }
    pub(crate) fn cancel_save_unpublished(&mut self, c: &SaveCapture) -> Result<(), Error> {
        if matches!(
            self.active_save(c)?.stage,
            SaveStage::Replacing | SaveStage::Verifying
        ) {
            return Err(Error::TooLate);
        }
        self.docs.get_mut(&c.snapshot.doc).unwrap().save_terminal = Some(SaveTerminal {
            operation: c.operation,
            result: SaveResult::Cancelled,
            error: None,
        });
        self.release_save(c, false);
        #[cfg(windows)]
        self.publication_recovery.recorded(c.operation);
        Ok(())
    }
    fn release_save(&mut self, c: &SaveCapture, hold_scope: bool) {
        if let Some(active) = self.saves.remove(&c.snapshot.doc) {
            if let Some(scope) = active.scope {
                if hold_scope {
                    if let Some(lease) = self.scope_parents.get_mut(&scope.parent) {
                        assert_eq!(lease.operation, c.operation);
                        lease.held = true;
                    }
                    if let Some(lease) = scope
                        .object
                        .as_ref()
                        .and_then(|key| self.scope_objects.get_mut(key))
                    {
                        assert_eq!(lease.operation, c.operation);
                        lease.held = true;
                    }
                } else {
                    self.scope_parents.remove(&scope.parent);
                    if let Some(object) = scope.object {
                        self.scope_objects.remove(&object);
                    }
                }
            }
        }
        self.targets.remove(&c.target);
    }
    pub(crate) fn finish_save_failure_unpublished(
        &mut self,
        c: &SaveCapture,
        detail: SaveFailure,
    ) -> Result<(), Error> {
        let stage = self.active_save(c)?.stage;
        let before = matches!(
            stage,
            SaveStage::Encoding | SaveStage::WritingTemp | SaveStage::ReadyToReplace
        );
        if (detail.outcome == SaveFailureOutcome::BeforeReplace) != before {
            return Err(Error::InvalidSave);
        }
        let held = detail.conflict || !before;
        let record = SaveFailureRecord {
            scope: self.active_save(c)?.scope.clone(),
            operation: c.operation,
            target: c.target.clone(),
            binding_epoch: c.binding_epoch,
            revision: c.snapshot.revision,
            token: c.snapshot.token,
            stage,
            detail,
            acknowledged_before_replace: false,
        };
        let d = self
            .docs
            .get_mut(&c.snapshot.doc)
            .ok_or(Error::InvalidDoc)?;
        d.save_terminal = Some(SaveTerminal {
            operation: c.operation,
            result: if record.detail.outcome == SaveFailureOutcome::OutcomeUnknown {
                SaveResult::OutcomeUnknown
            } else {
                SaveResult::Failed
            },
            error: Some(record.detail.message.clone()),
        });
        d.recovery_snapshots.push((c.operation, c.snapshot.clone()));
        d.failures.push(record);
        d.save_blocked |= held;
        if held {
            self.unresolved_targets
                .insert(c.target.clone(), c.snapshot.doc);
        }
        self.release_save(c, held);
        #[cfg(windows)]
        self.publication_recovery.recorded(c.operation);
        Ok(())
    }
    pub fn recovery_snapshot(&self, doc: DocId, operation: u64) -> Result<Snapshot, Error> {
        self.docs
            .get(&doc)
            .ok_or(Error::InvalidDoc)?
            .recovery_snapshots
            .iter()
            .find(|(op, _)| *op == operation)
            .map(|(_, s)| s.clone())
            .ok_or(Error::InvalidSave)
    }
    /// TRUST BOUNDARY: T07 IO owner must supply a truly verified durable result.
    /// Component tests simulate that result; they provide no disk-save evidence.
    pub(crate) fn accept_verified_save_unpublished(
        &mut self,
        c: &SaveCapture,
    ) -> Result<(), Error> {
        self.accept_result_unpublished(c, c.target.clone(), false)
            .map(|_| ())
    }
    pub(crate) fn accept_published_new_unpublished(
        &mut self,
        c: &SaveCapture,
        final_target: TargetKey,
    ) -> Result<u64, Error> {
        self.accept_result_unpublished(c, final_target, true)
    }
    fn accept_result_unpublished(
        &mut self,
        c: &SaveCapture,
        final_target: TargetKey,
        published_new: bool,
    ) -> Result<u64, Error> {
        let active = self.active_save(c)?;
        let required = if published_new {
            SaveStage::Replacing
        } else {
            SaveStage::Verifying
        };
        if active.stage != required
            || (published_new && active.scope.as_ref().is_none_or(|s| s.object.is_some()))
        {
            return Err(Error::InvalidSave);
        }
        if self
            .bound_targets
            .get(&final_target)
            .is_some_and(|owner| *owner != c.snapshot.doc)
        {
            return Err(Error::Busy);
        }
        // Check every fallible precondition before changing baseline or binding.
        let next_epoch = self.docs[&c.snapshot.doc]
            .binding_epoch
            .checked_add(1)
            .ok_or(Error::CounterExhausted)?;
        if published_new {
            self.saves.get_mut(&c.snapshot.doc).unwrap().stage = SaveStage::Verifying;
        }
        let d = self
            .docs
            .get_mut(&c.snapshot.doc)
            .ok_or(Error::InvalidDoc)?;
        d.baseline = c.snapshot.token;
        d.backed = true;
        d.baseline_kind = BaselineKind::SavedByCaller;
        let previous = d.bound_target.clone();
        d.bound_target = Some(final_target.clone());
        d.binding_epoch = next_epoch;
        d.save_terminal = Some(SaveTerminal {
            operation: c.operation,
            result: SaveResult::SucceededByCaller,
            error: None,
        });
        if let Some(previous) = previous {
            self.bound_targets.remove(&previous);
        }
        self.bound_targets.insert(final_target, c.snapshot.doc);
        // Generic/save publication receipts do not transfer a lifetime witness.
        #[cfg(windows)]
        self.opened_identities.remove(&c.snapshot.doc);
        self.release_save(c, false);
        #[cfg(windows)]
        self.publication_recovery.complete(c.operation);
        Ok(next_epoch)
    }
    pub fn new(budget: HistoryBudget) -> Self {
        Self {
            #[cfg(all(test, windows))]
            verified_open_doc_counter: None,
            #[cfg(windows)]
            opened_identities: HashMap::new(),
            #[cfg(windows)]
            publication_recovery: Arc::default(),
            scope_parents: HashMap::new(),
            scope_objects: HashMap::new(),
            bound_targets: HashMap::new(),
            unresolved_targets: HashMap::new(),
            observer: crate::observe::ObserverHub::new(),
            docs: HashMap::new(),
            view_docs: HashMap::new(),
            next_token: 1,
            budget,
            charge: 0,
            saves: HashMap::new(),
            targets: HashMap::new(),
            next_save: 1,
        }
    }
    /// Fresh-document counter boundary fixture; absent from normal product builds.
    #[cfg(all(test, windows))]
    pub(crate) fn test_set_unbound_binding_epoch(&mut self, doc: DocId, epoch: u64) {
        assert!(!self.saves.contains_key(&doc));
        let document = self.docs.get_mut(&doc).unwrap();
        assert!(document.bound_target.is_none());
        document.binding_epoch = epoch;
    }
    pub fn open_utf8(
        &mut self,
        raw: &str,
        profile: Profile,
        backed_by_disk: bool,
    ) -> Result<Snapshot, Error> {
        if profile.version != 1 {
            return Err(Error::InvalidProfile);
        }
        self.open_import(raw, profile, backed_by_disk, false)
    }
    pub fn open_decoded(
        &mut self,
        text: &str,
        format: crate::files::Format,
    ) -> Result<Snapshot, Error> {
        self.open_import(
            text,
            Profile {
                version: 2,
                file_format: Some(format),
                ..Profile::default()
            },
            false,
            true,
        )
    }
    fn open_import(
        &mut self,
        raw: &str,
        profile: Profile,
        backed_by_disk: bool,
        decoded: bool,
    ) -> Result<Snapshot, Error> {
        if !profile.valid() {
            return Err(Error::InvalidProfile);
        }
        let id = NEXT_DOC
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |x| x.checked_add(1))
            .map_err(|_| Error::CounterExhausted)?;
        let token = self.allocate_token()?;
        let current = Snapshot {
            doc: DocId(id),
            revision: Revision(0),
            token,
            body: Arc::new(if decoded {
                Body::import_decoded(raw, profile)
            } else {
                Body::import_utf8(raw, profile)
            }),
        };
        self.docs.insert(
            current.doc,
            Document {
                failures: Vec::new(),
                recovery_snapshots: Vec::new(),
                save_blocked: false,
                closed_views: std::collections::BTreeMap::new(),
                baseline_kind: if backed_by_disk {
                    BaselineKind::ExternalLoadClaim
                } else if raw.is_empty() {
                    BaselineKind::InitialEmpty
                } else {
                    BaselineKind::MemoryImport
                },
                bound_target: None,
                binding_epoch: 0,
                save_terminal: None,
                current: current.clone(),
                views: HashMap::new(),
                merge: None,
                last_fence: FenceReason::Explicit,
                baseline: token,
                backed: backed_by_disk,
                undo: VecDeque::new(),
                redo: Vec::new(),
                charge: 0,
            },
        );
        self.register_observer_document(current.doc);
        Ok(current)
    }
    fn allocate_token(&mut self) -> Result<StateToken, Error> {
        let n = self.next_token;
        self.next_token = n.checked_add(1).ok_or(Error::CounterExhausted)?;
        Ok(StateToken(n))
    }
    pub fn snapshot(&self, doc: DocId) -> Result<Snapshot, Error> {
        Ok(self
            .docs
            .get(&doc)
            .ok_or(Error::InvalidDoc)?
            .current
            .clone())
    }
    pub fn dirty(&self, doc: DocId) -> Result<bool, Error> {
        let d = self.docs.get(&doc).ok_or(Error::InvalidDoc)?;
        Ok(d.current.token != d.baseline)
    }
    pub fn backed_by_disk(&self, doc: DocId) -> Result<bool, Error> {
        Ok(self.docs.get(&doc).ok_or(Error::InvalidDoc)?.backed)
    }
    pub fn history_depth(&self, doc: DocId) -> Result<(usize, usize), Error> {
        let d = self.docs.get(&doc).ok_or(Error::InvalidDoc)?;
        Ok((d.undo.len(), d.redo.len()))
    }
    pub fn history_charge(&self) -> usize {
        self.charge
    }
    /// Trusted document-service entry; view adapters must use submit_view.
    pub(crate) fn submit_unpublished(&mut self, txn: Transaction) -> Result<Receipt, Error> {
        self.submit_inner(txn, None)
    }
    pub(crate) fn submit_inner(
        &mut self,
        txn: Transaction,
        context: Option<&ViewTransaction>,
    ) -> Result<Receipt, Error> {
        let s = self.snapshot(txn.doc)?;
        if s.revision != txn.base {
            return Err(Error::StaleRevision);
        }
        if txn.cancelled {
            return Err(Error::Cancelled);
        }
        let mut previous_end = None;
        let mut forward = Vec::new();
        let mut inverse = Vec::new();
        let mut shift: isize = 0;
        let mut bytes = 0usize;
        for edit in &txn.edits {
            s.check_position(edit.start.doc, edit.start.revision)?;
            s.check_position(edit.end.doc, edit.end.revision)?;
            let a = edit.start.offset;
            let b = edit.end.offset;
            if a > b {
                return Err(Error::InvalidBoundary);
            }
            s.body.scalar(a)?;
            s.body.scalar(b)?;
            if txn.boundary == BoundaryPolicy::Grapheme
                && (!boundary(&s.body, a)? || !boundary(&s.body, b)?)
            {
                return Err(Error::InsideGrapheme);
            }
            // Adjacent ranges are allowed; two insertions at one boundary are ambiguous.
            if let Some((end, start)) = previous_end {
                if a < end || a == start {
                    return Err(Error::InvalidOverlap);
                }
            }
            previous_end = Some((b, a));
            if edit.text.contains('\r')
                || edit.text.bytes().filter(|&b| b == b'\n').count() != edit.eols.len()
            {
                return Err(Error::InvalidEolPlan);
            }
            let old = s.body.source().byte_slice(a..b).to_string();
            let old_eols = s.body.eols_in(a..b);
            if old == edit.text && old_eols == edit.eols {
                continue;
            }
            bytes = bytes
                .checked_add(old.len())
                .and_then(|n| n.checked_add(edit.text.len()))
                .ok_or(Error::HistoryBudget)?;
            forward.push(Delta {
                range: a..b,
                text: edit.text.clone(),
                eols: edit.eols.clone(),
            });
            let new_a = a.checked_add_signed(shift).ok_or(Error::InvalidBoundary)?;
            inverse.push(Delta {
                range: new_a..new_a + edit.text.len(),
                text: old,
                eols: old_eols,
            });
            shift = shift
                .checked_add(edit.text.len() as isize - (b - a) as isize)
                .ok_or(Error::InvalidBoundary)?;
        }
        if forward.is_empty() {
            let receipt = Receipt {
                history_group: None,
                snapshot: s.clone(),
                previous: s.revision,
                previous_body: s.body.clone(),
                forward,
                inverse,
                changed: false,
            };
            let views =
                self.prepare_views(&receipt, context.map(|v| (v.view, &v.selection_after)))?;
            let d = self.docs.get_mut(&s.doc).unwrap();
            if let Some(v) = context {
                if v.kind != InputKind::Typing
                    || d.merge.as_ref().is_some_and(|m| m.view != v.view)
                    || d.views.get(&v.view) != views.get(&v.view)
                {
                    d.merge = None;
                    d.last_fence = FenceReason::Navigation;
                }
            } else {
                d.merge = None;
                d.last_fence = FenceReason::Service;
            }
            d.views = views;
            return Ok(receipt);
        }
        let mut body = (*s.body).clone();
        for d in forward.iter().rev() {
            body.replace(d.range.clone(), &d.text, &d.eols);
        }
        let charge = 4096usize
            .checked_add(bytes.checked_mul(8).ok_or(Error::HistoryBudget)?)
            .ok_or(Error::HistoryBudget)?;
        self.commit(s, Arc::new(body), forward, inverse, charge, context)
    }
    /// Metadata-only transaction; UTF-8/BOM and EOL policy changes are undoable.
    pub(crate) fn set_serialization_unpublished(
        &mut self,
        doc: DocId,
        base: Revision,
        profile: Profile,
        eol: Option<Eol>,
    ) -> Result<Receipt, Error> {
        if !profile.valid() {
            return Err(Error::InvalidProfile);
        }
        let s = self.snapshot(doc)?;
        if s.revision != base {
            return Err(Error::StaleRevision);
        }
        let eol_change = eol.is_some_and(|e| s.body.eols().iter().any(|old| *old != e));
        if profile == s.body.profile && !eol_change {
            return Ok(Receipt {
                history_group: None,
                snapshot: s.clone(),
                previous: base,
                previous_body: s.body.clone(),
                forward: vec![],
                inverse: vec![],
                changed: false,
            });
        }
        let mut body = (*s.body).clone();
        body.profile = profile;
        if let Some(e) = eol.filter(|_| eol_change) {
            body.change_eols(e);
        }
        let charge = 4096 + s.body.len_bytes().saturating_mul(2);
        self.commit(s, Arc::new(body), vec![], vec![], charge, None)
    }
    fn commit(
        &mut self,
        s: Snapshot,
        body: Arc<Body>,
        forward: Vec<Delta>,
        inverse: Vec<Delta>,
        charge: usize,
        context: Option<&ViewTransaction>,
    ) -> Result<Receipt, Error> {
        let d = self.docs.get(&s.doc).unwrap();
        let freed: usize = d.redo.iter().map(|e| e.charge).sum();
        let doc_charge = d.charge - freed;
        let process_charge = self.charge - freed;
        if charge > self.budget.document.saturating_sub(doc_charge)
            || charge > self.budget.process.saturating_sub(process_charge)
        {
            return Err(Error::HistoryBudget);
        }
        let revision = Revision(s.revision.0.checked_add(1).ok_or(Error::CounterExhausted)?);
        let preview = Receipt {
            history_group: None,
            snapshot: Snapshot {
                doc: s.doc,
                revision,
                token: s.token,
                body: body.clone(),
            },
            previous: s.revision,
            previous_body: s.body.clone(),
            forward: forward.clone(),
            inverse: inverse.clone(),
            changed: true,
        };
        let views = self.prepare_views(&preview, context.map(|v| (v.view, &v.selection_after)))?;
        let before_selection = context.map(|v| d.views[&v.view].selection.clone());
        let merge = merge_candidate(d, &s, &forward, context);
        let token = self.allocate_token()?;
        let next = Snapshot {
            doc: s.doc,
            revision,
            token,
            body: body.clone(),
        };
        let mut entry = Entry {
            group: HistoryGroupId(token.0),
            before: s.body.clone(),
            after: body,
            old: s.token,
            new: token,
            forward: forward.clone(),
            inverse: inverse.clone(),
            charge,
            selection_before: before_selection,
            selection_after: context.map(|v| v.selection_after.clone()),
        };
        let d = self.docs.get_mut(&s.doc).unwrap();
        if merge {
            let old = d.undo.back().unwrap();
            entry.group = old.group;
            entry.before = old.before.clone();
            entry.old = old.old;
            entry.selection_before = old.selection_before.clone();
            entry.forward = old.forward.clone();
            entry.forward[0].text.push_str(&forward[0].text);
            entry.inverse = old.inverse.clone();
            entry.inverse[0].range.end += forward[0].text.len();
            entry.charge += old.charge;
        }
        d.redo.clear();
        if merge {
            d.undo.pop_back();
        }
        let group = entry.group;
        d.undo.push_back(entry);
        d.merge = typing_cursor(&s, &forward, context, token);
        if d.merge.is_none() {
            d.last_fence = FenceReason::Service;
        }
        d.charge = doc_charge + charge;
        self.charge = process_charge + charge;
        d.current = next.clone();
        d.views = views;
        let receipt = Receipt {
            history_group: Some(group),
            snapshot: next,
            previous: s.revision,
            previous_body: s.body.clone(),
            forward,
            inverse,
            changed: true,
        };
        self.observer.pending_content = Some(receipt.clone());
        Ok(receipt)
    }
    pub(crate) fn undo_unpublished(&mut self, doc: DocId) -> Result<Receipt, Error> {
        self.travel(doc, false, None)
    }
    pub(crate) fn redo_unpublished(&mut self, doc: DocId) -> Result<Receipt, Error> {
        self.travel(doc, true, None)
    }
    pub(crate) fn travel(
        &mut self,
        doc: DocId,
        redo: bool,
        invoking: Option<ViewId>,
    ) -> Result<Receipt, Error> {
        let d = self.docs.get(&doc).ok_or(Error::InvalidDoc)?;
        let revision = Revision(
            d.current
                .revision
                .0
                .checked_add(1)
                .ok_or(Error::CounterExhausted)?,
        );
        let entry = if redo { d.redo.last() } else { d.undo.back() }
            .ok_or(Error::NoHistory)?
            .clone();
        let previous = d.current.revision;
        let previous_body = d.current.body.clone();
        let (body, token, forward, inverse) = if redo {
            (
                entry.after.clone(),
                entry.new,
                entry.forward.clone(),
                entry.inverse.clone(),
            )
        } else {
            (
                entry.before.clone(),
                entry.old,
                entry.inverse.clone(),
                entry.forward.clone(),
            )
        };
        let next = Snapshot {
            doc,
            revision,
            token,
            body,
        };
        let receipt = Receipt {
            history_group: Some(entry.group),
            snapshot: next.clone(),
            previous,
            previous_body,
            forward,
            inverse,
            changed: true,
        };
        let restored = invoking.map(|view| {
            let stored = if redo {
                &entry.selection_after
            } else {
                &entry.selection_before
            };
            let selections = stored
                .as_ref()
                .map(|s| s.reversion(&next))
                .unwrap_or_else(|| {
                    let offset = receipt
                        .forward
                        .first()
                        .map_or(0, |d| d.range.start.min(next.body.len_bytes()));
                    let a = crate::views::snap_anchor(
                        &next,
                        crate::Anchor {
                            position: SourceByte::at(&next, offset),
                            affinity: crate::Affinity::Before,
                        },
                    )
                    .unwrap()
                    .0;
                    SelectionSet::caret(&next, a.position.offset)
                });
            (view, selections)
        });
        let views = self.prepare_views(&receipt, restored.as_ref().map(|(v, s)| (*v, s)))?;
        let d = self.docs.get_mut(&doc).unwrap();
        d.current = next;
        d.views = views;
        d.merge = None;
        d.last_fence = FenceReason::UndoRedo;
        if redo {
            d.redo.pop();
            d.undo.push_back(entry);
        } else {
            d.undo.pop_back();
            d.redo.push(entry);
        }
        self.observer.pending_content = Some(receipt.clone());
        Ok(receipt)
    }
}

pub(crate) struct MergeState {
    view: ViewId,
    tick_ms: u64,
    end: usize,
    token: StateToken,
}
pub const INPUT_MERGE_WINDOW_MS: u64 = 750;
pub const INPUT_MERGE_MAX_BYTES: usize = 64 * 1024;
fn typing_cursor(
    _s: &Snapshot,
    forward: &[Delta],
    v: Option<&ViewTransaction>,
    token: StateToken,
) -> Option<MergeState> {
    let v = v?;
    if v.kind != InputKind::Typing || forward.len() != 1 || v.selection_after.selections.len() != 1
    {
        return None;
    }
    let d = &forward[0];
    let range = v.selection_after.selections[0].range();
    if !d.range.is_empty()
        || d.text.is_empty()
        || d.text.contains('\n')
        || !range.is_empty()
        || range.start != d.range.start + d.text.len()
    {
        return None;
    }
    if d.text.len() > INPUT_MERGE_MAX_BYTES {
        return None;
    }
    Some(MergeState {
        view: v.view,
        tick_ms: v.tick_ms,
        end: d.range.start + d.text.len(),
        token,
    })
}
fn merge_candidate(
    d: &Document,
    s: &Snapshot,
    forward: &[Delta],
    v: Option<&ViewTransaction>,
) -> bool {
    let Some(next) = typing_cursor(s, forward, v, s.token) else {
        return false;
    };
    let Some(previous) = &d.merge else {
        return false;
    };
    let Some(last) = d.undo.back() else {
        return false;
    };
    let v = v.unwrap();
    let before = &d.views[&v.view].selection;
    next.view == previous.view
        && s.token == previous.token
        && last.new == s.token
        && d.redo.is_empty()
        && v.tick_ms >= previous.tick_ms
        && v.tick_ms - previous.tick_ms <= INPUT_MERGE_WINDOW_MS
        && forward[0].range.start == previous.end
        && before.selections.len() == 1
        && before.selections[0].range() == (previous.end..previous.end)
        && last.forward[0].text.len() + forward[0].text.len() <= INPUT_MERGE_MAX_BYTES
}

impl Store {
    pub(crate) fn observer_metadata(&self, doc: DocId) -> GlobalMetadata {
        let d = &self.docs[&doc];
        let active = self.saves.get(&doc).map(|a| ActiveSaveState {
            operation: a.capture.operation,
            revision: a.capture.snapshot.revision,
            token: a.capture.snapshot.token,
            target: a.capture.target.clone(),
            guard: a.capture.guard.clone(),
            binding_epoch: a.capture.binding_epoch,
            stage: a.stage,
            cancellable: !matches!(a.stage, SaveStage::Replacing | SaveStage::Verifying),
        });
        GlobalMetadata {
            baseline: BaselineState {
                token: d.baseline,
                kind: d.baseline_kind.clone(),
                backed_by_disk: d.backed,
            },
            dirty: d.current.token != d.baseline,
            attention: FileAttention {
                recoveries: d.failures.clone(),
                needs_target: d.bound_target.is_none(),
                binding_epoch: d.binding_epoch,
                target: d.bound_target.clone(),
                known_fingerprint: None,
                conflict: if d.save_blocked {
                    let r = d.failures.last().unwrap();
                    Unimplemented::NeedsVerification {
                        operation: r.operation,
                        target: r.target.clone(),
                    }
                } else {
                    Unimplemented::NotImplemented
                },
                save_outcome_unknown: if let Some(r) = d
                    .failures
                    .iter()
                    .rev()
                    .find(|r| r.detail.outcome == SaveFailureOutcome::OutcomeUnknown)
                {
                    Unimplemented::NeedsVerification {
                        operation: r.operation,
                        target: r.target.clone(),
                    }
                } else {
                    Unimplemented::NotImplemented
                },
                display_error: d.save_terminal.as_ref().and_then(|t| t.error.clone()),
            },
            save: SaveObservation {
                active,
                last_terminal: d.save_terminal.clone(),
            },
            history: HistoryObservation {
                can_undo: !d.undo.is_empty(),
                can_redo: !d.redo.is_empty(),
                undo_groups: d.undo.len(),
                redo_groups: d.redo.len(),
                last_undo_group: d.undo.back().map(|e| e.group),
                last_redo_group: d.redo.last().map(|e| e.group),
                merge: d.merge.as_ref().map(|m| MergeObservation {
                    view: m.view,
                    tick_ms: m.tick_ms,
                    end: m.end,
                    token: m.token,
                }),
                last_fence: d.last_fence,
            },
        }
    }
    pub(crate) fn has_active_save(&self, doc: DocId) -> bool {
        self.saves.contains_key(&doc)
    }
    pub(crate) fn remove_memory_document(&mut self, doc: DocId) -> Result<(), Error> {
        let d = self.docs.get(&doc).ok_or(Error::InvalidDoc)?;
        if d.save_blocked
            || d.failures.iter().any(|failure| {
                !failure.acknowledged_before_replace
                    || !failure_ack::is_eligible_pre_replace_failure(failure)
            })
        {
            return Err(Error::UnresolvedConflict);
        }
        let d = self.docs.remove(&doc).ok_or(Error::InvalidDoc)?;
        #[cfg(windows)]
        self.opened_identities.remove(&doc);
        if let Some(target) = &d.bound_target {
            self.bound_targets.remove(target);
        }
        for id in d.views.keys() {
            self.view_docs.remove(id);
        }
        self.charge -= d.charge;
        Ok(())
    }
}
