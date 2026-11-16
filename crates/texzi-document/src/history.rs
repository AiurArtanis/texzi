use crate::{
    Body, DocId, Eol, Error, Profile, Revision, SourceByte, StateToken, coordinates::boundary,
};
use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

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
#[derive(Clone, Debug)]
pub struct Delta {
    pub range: Range<usize>,
    pub text: String,
    pub eols: Vec<Eol>,
}
#[derive(Clone, Debug)]
pub struct Receipt {
    pub snapshot: Snapshot,
    pub previous: Revision,
    pub previous_body: Arc<Body>,
    pub forward: Vec<Delta>,
    pub inverse: Vec<Delta>,
    pub changed: bool,
}
#[derive(Clone)]
struct Entry {
    before: Arc<Body>,
    after: Arc<Body>,
    old: StateToken,
    new: StateToken,
    forward: Vec<Delta>,
    inverse: Vec<Delta>,
    charge: usize,
}
struct Document {
    current: Snapshot,
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
    docs: HashMap<DocId, Document>,
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
struct ActiveSave {
    capture: SaveCapture,
    stage: SaveStage,
}

impl Store {
    /// Memory-only save coordination. This method does not write or verify a file.
    pub fn begin_save(
        &mut self,
        doc: DocId,
        target: TargetKey,
        guard: String,
    ) -> Result<SaveCapture, Error> {
        let s = self.snapshot(doc)?;
        if self.saves.contains_key(&doc) || self.targets.contains_key(&target) {
            return Err(Error::Busy);
        }
        let operation = self.next_save;
        let next = operation.checked_add(1).ok_or(Error::CounterExhausted)?;
        let capture = SaveCapture {
            operation,
            snapshot: s,
            binding_epoch: 0,
            target: target.clone(),
            guard,
        };
        self.saves.insert(
            doc,
            ActiveSave {
                capture: capture.clone(),
                stage: SaveStage::Encoding,
            },
        );
        self.targets.insert(target, doc);
        self.next_save = next;
        Ok(capture)
    }
    fn active_save(&self, c: &SaveCapture) -> Result<&ActiveSave, Error> {
        let a = self.saves.get(&c.snapshot.doc).ok_or(Error::InvalidSave)?;
        if a.capture.operation != c.operation
            || a.capture.binding_epoch != c.binding_epoch
            || a.capture.target != c.target
            || a.capture.guard != c.guard
            || a.capture.snapshot.revision != c.snapshot.revision
            || a.capture.snapshot.token != c.snapshot.token
        {
            return Err(Error::InvalidSave);
        }
        Ok(a)
    }
    pub fn advance_save(&mut self, c: &SaveCapture, next: SaveStage) -> Result<(), Error> {
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
    pub fn commit_permit(&mut self, c: &SaveCapture) -> Result<(), Error> {
        if self.active_save(c)?.stage != SaveStage::ReadyToReplace {
            return Err(Error::InvalidSave);
        }
        self.saves.get_mut(&c.snapshot.doc).unwrap().stage = SaveStage::Replacing;
        Ok(())
    }
    pub fn cancel_save(&mut self, c: &SaveCapture) -> Result<(), Error> {
        if matches!(
            self.active_save(c)?.stage,
            SaveStage::Replacing | SaveStage::Verifying
        ) {
            return Err(Error::TooLate);
        }
        self.release_save(c);
        Ok(())
    }
    fn release_save(&mut self, c: &SaveCapture) {
        self.saves.remove(&c.snapshot.doc);
        self.targets.remove(&c.target);
    }
    /// TRUST BOUNDARY: T07 IO owner must supply a truly verified durable result.
    /// Component tests simulate that result; they provide no disk-save evidence.
    pub fn accept_verified_save(&mut self, c: &SaveCapture) -> Result<(), Error> {
        if self.active_save(c)?.stage != SaveStage::Verifying {
            return Err(Error::InvalidSave);
        }
        let d = self
            .docs
            .get_mut(&c.snapshot.doc)
            .ok_or(Error::InvalidDoc)?;
        d.baseline = c.snapshot.token;
        d.backed = true;
        self.release_save(c);
        Ok(())
    }
    pub fn new(budget: HistoryBudget) -> Self {
        Self {
            docs: HashMap::new(),
            next_token: 1,
            budget,
            charge: 0,
            saves: HashMap::new(),
            targets: HashMap::new(),
            next_save: 1,
        }
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
        let id = NEXT_DOC
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |x| x.checked_add(1))
            .map_err(|_| Error::CounterExhausted)?;
        let token = self.allocate_token()?;
        let current = Snapshot {
            doc: DocId(id),
            revision: Revision(0),
            token,
            body: Arc::new(Body::import_utf8(raw, profile)),
        };
        self.docs.insert(
            current.doc,
            Document {
                current: current.clone(),
                baseline: token,
                backed: backed_by_disk,
                undo: VecDeque::new(),
                redo: Vec::new(),
                charge: 0,
            },
        );
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
    pub fn submit(&mut self, txn: Transaction) -> Result<Receipt, Error> {
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
            return Ok(Receipt {
                snapshot: s.clone(),
                previous: s.revision,
                previous_body: s.body.clone(),
                forward,
                inverse,
                changed: false,
            });
        }
        let mut body = (*s.body).clone();
        for d in forward.iter().rev() {
            body.replace(d.range.clone(), &d.text, &d.eols);
        }
        let charge = 4096usize
            .checked_add(bytes.checked_mul(8).ok_or(Error::HistoryBudget)?)
            .ok_or(Error::HistoryBudget)?;
        self.commit(s, Arc::new(body), forward, inverse, charge)
    }
    /// Metadata-only transaction; UTF-8/BOM and EOL policy changes are undoable.
    pub fn set_serialization(
        &mut self,
        doc: DocId,
        base: Revision,
        profile: Profile,
        eol: Option<Eol>,
    ) -> Result<Receipt, Error> {
        if profile.version != 1 {
            return Err(Error::InvalidProfile);
        }
        let s = self.snapshot(doc)?;
        if s.revision != base {
            return Err(Error::StaleRevision);
        }
        let eol_change = eol.is_some_and(|e| s.body.eols().iter().any(|old| *old != e));
        if profile == s.body.profile && !eol_change {
            return Ok(Receipt {
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
        self.commit(s, Arc::new(body), vec![], vec![], charge)
    }
    fn commit(
        &mut self,
        s: Snapshot,
        body: Arc<Body>,
        forward: Vec<Delta>,
        inverse: Vec<Delta>,
        charge: usize,
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
        let token = self.allocate_token()?;
        let next = Snapshot {
            doc: s.doc,
            revision,
            token,
            body: body.clone(),
        };
        let entry = Entry {
            before: s.body.clone(),
            after: body,
            old: s.token,
            new: token,
            forward: forward.clone(),
            inverse: inverse.clone(),
            charge,
        };
        let d = self.docs.get_mut(&s.doc).unwrap();
        d.redo.clear();
        d.undo.push_back(entry);
        d.charge = doc_charge + charge;
        self.charge = process_charge + charge;
        d.current = next.clone();
        Ok(Receipt {
            snapshot: next,
            previous: s.revision,
            previous_body: s.body.clone(),
            forward,
            inverse,
            changed: true,
        })
    }
    pub fn undo(&mut self, doc: DocId) -> Result<Receipt, Error> {
        self.travel(doc, false)
    }
    pub fn redo(&mut self, doc: DocId) -> Result<Receipt, Error> {
        self.travel(doc, true)
    }
    fn travel(&mut self, doc: DocId, redo: bool) -> Result<Receipt, Error> {
        let d = self.docs.get_mut(&doc).ok_or(Error::InvalidDoc)?;
        let revision = Revision(
            d.current
                .revision
                .0
                .checked_add(1)
                .ok_or(Error::CounterExhausted)?,
        );
        let entry = if redo {
            d.redo.pop()
        } else {
            d.undo.pop_back()
        }
        .ok_or(Error::NoHistory)?;
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
        d.current = next.clone();
        if redo {
            d.undo.push_back(entry);
        } else {
            d.redo.push(entry);
        }
        Ok(Receipt {
            snapshot: next,
            previous,
            previous_body,
            forward,
            inverse,
            changed: true,
        })
    }
}
