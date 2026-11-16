//! C08 memory publication, bounded replay, complete-batch client recovery.
use crate::*;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub const OBSERVER_SCHEMA: u32 = 1;
pub const MAX_SCOPE_VIEWS: usize = 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StoreEpoch {
    pub session: [u64; 2],
    pub instance: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EventSeq(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SubscriptionId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HistoryGroupId(pub u64);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverScope {
    pub schema: u32,
    pub views: Vec<ViewId>,
}
impl ObserverScope {
    pub fn new(mut views: Vec<ViewId>) -> Self {
        views.sort();
        views.dedup();
        Self {
            schema: OBSERVER_SCHEMA,
            views,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub store: StoreEpoch,
    pub doc: DocId,
    pub scope: ObserverScope,
    pub seq: EventSeq,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BaselineKind {
    InitialEmpty,
    MemoryImport,
    ExternalLoadClaim,
    SavedByCaller,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unimplemented {
    NotImplemented,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaselineState {
    pub token: StateToken,
    pub kind: BaselineKind,
    pub backed_by_disk: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileAttention {
    pub needs_target: bool,
    pub binding_epoch: u64,
    pub target: Option<TargetKey>,
    pub known_fingerprint: Option<String>,
    pub conflict: Unimplemented,
    pub save_outcome_unknown: Unimplemented,
    pub display_error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveSaveState {
    pub operation: u64,
    pub revision: Revision,
    pub token: StateToken,
    pub target: TargetKey,
    pub guard: String,
    pub binding_epoch: u64,
    pub stage: SaveStage,
    pub cancellable: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveResult {
    SucceededByCaller,
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveTerminal {
    pub operation: u64,
    pub result: SaveResult,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveObservation {
    pub active: Option<ActiveSaveState>,
    pub last_terminal: Option<SaveTerminal>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeObservation {
    pub view: ViewId,
    pub tick_ms: u64,
    pub end: usize,
    pub token: StateToken,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryObservation {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_groups: usize,
    pub redo_groups: usize,
    pub last_undo_group: Option<HistoryGroupId>,
    pub last_redo_group: Option<HistoryGroupId>,
    pub merge: Option<MergeObservation>,
    pub last_fence: FenceReason,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalMetadata {
    pub baseline: BaselineState,
    pub dirty: bool,
    pub attention: FileAttention,
    pub save: SaveObservation,
    pub history: HistoryObservation,
}
#[derive(Clone, Debug)]
pub struct ObservedGlobal {
    pub document: Snapshot,
    pub metadata: GlobalMetadata,
}
impl PartialEq for ObservedGlobal {
    fn eq(&self, r: &Self) -> bool {
        self.document.doc == r.document.doc
            && self.document.revision == r.document.revision
            && self.document.token == r.document.token
            && Arc::ptr_eq(&self.document.body, &r.document.body)
            && self.metadata == r.metadata
    }
}
impl Eq for ObservedGlobal {}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedView {
    pub id: ViewId,
    pub doc: DocId,
    pub epoch: u64,
    pub at_revision: Revision,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObservedView {
    Open {
        state: ViewState,
        folds: Unimplemented,
        logical_focus: Unimplemented,
        composition: Unimplemented,
        pending_mode: Unimplemented,
    },
    Closed(ClosedView),
}
impl ObservedView {
    pub fn id(&self) -> ViewId {
        match self {
            Self::Open { state, .. } => state.id,
            Self::Closed(v) => v.id,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverState {
    pub global: Arc<ObservedGlobal>,
    pub views: BTreeMap<ViewId, ObservedView>,
}
#[derive(Clone, Debug)]
pub struct ObserverSnapshot {
    pub cursor: Cursor,
    pub subscription: SubscriptionId,
    pub state: ObserverState,
}
#[derive(Clone, Debug)]
pub struct SubscriptionHandle {
    pub id: SubscriptionId,
    pub baseline: Cursor,
    pub capacity: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicationId {
    pub store: StoreEpoch,
    pub doc: DocId,
    pub seq: EventSeq,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentChange {
    pub transaction: PublicationId,
    pub group: HistoryGroupId,
    pub previous: Revision,
    pub revision: Revision,
    pub token: StateToken,
    pub forward: Arc<[Delta]>,
    pub inverse: Arc<[Delta]>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObserverEvent {
    Content(Arc<ContentChange>),
    Global(Arc<ObservedGlobal>),
    View(ObservedView),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventBatch {
    pub cursor: Cursor,
    pub subscription: SubscriptionId,
    pub events: Vec<ObserverEvent>,
}
impl EventBatch {
    pub fn frames(&self) -> Vec<ObserverFrame> {
        let mut out = vec![ObserverFrame::Begin {
            cursor: self.cursor.clone(),
            subscription: self.subscription,
            parts: self.events.len(),
        }];
        out.extend(
            self.events
                .iter()
                .enumerate()
                .map(|(index, event)| ObserverFrame::Part {
                    cursor: self.cursor.clone(),
                    subscription: self.subscription,
                    index,
                    event: event.clone(),
                }),
        );
        out.push(ObserverFrame::End {
            cursor: self.cursor.clone(),
            subscription: self.subscription,
            parts: self.events.len(),
        });
        out
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaggedInfo {
    pub cursor: Cursor,
    pub subscription: SubscriptionId,
    pub registered_at: EventSeq,
    pub earliest: EventSeq,
    pub head: EventSeq,
    pub capacity: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObserverError {
    DocGone,
    WrongStore,
    WrongDoc,
    ScopeMismatch,
    UnsupportedSchema,
    InvalidScope,
    InvalidCapacity,
    SubscriptionLimit,
    StaleSubscription,
    CursorAhead,
    CursorTooOld { earliest: EventSeq, head: EventSeq },
    Lagged(LaggedInfo),
    CounterExhausted,
    Protocol,
    NeedsObserve,
    BufferOverflow,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObserverPoll {
    Batch(EventBatch),
    Pending,
    Lagged(LaggedInfo),
    DocGone {
        cursor: Cursor,
        subscription: SubscriptionId,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObserverFrame {
    Begin {
        cursor: Cursor,
        subscription: SubscriptionId,
        parts: usize,
    },
    Part {
        cursor: Cursor,
        subscription: SubscriptionId,
        index: usize,
        event: ObserverEvent,
    },
    End {
        cursor: Cursor,
        subscription: SubscriptionId,
        parts: usize,
    },
    Unknown {
        cursor: Cursor,
        subscription: SubscriptionId,
    },
}
impl ObserverFrame {
    pub fn identity(&self) -> (&Cursor, SubscriptionId) {
        match self {
            Self::Begin {
                cursor,
                subscription,
                ..
            }
            | Self::Part {
                cursor,
                subscription,
                ..
            }
            | Self::End {
                cursor,
                subscription,
                ..
            }
            | Self::Unknown {
                cursor,
                subscription,
            } => (cursor, *subscription),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ObserverLimits {
    pub journal_batches: usize,
    pub max_queue_batches: usize,
    pub max_subscriptions: usize,
}
impl Default for ObserverLimits {
    fn default() -> Self {
        Self {
            journal_batches: 64,
            max_queue_batches: 64,
            max_subscriptions: 1024,
        }
    }
}
#[derive(Clone)]
struct Record {
    seq: EventSeq,
    global: Option<Arc<ObservedGlobal>>,
    views: Vec<ObservedView>,
    content: Option<Arc<ContentChange>>,
}
struct Journal {
    head: EventSeq,
    last: ObserverState,
    log: VecDeque<Arc<Record>>,
}
struct Subscription {
    handle: SubscriptionHandle,
    queue: VecDeque<Arc<Record>>,
    lagged: Option<LaggedInfo>,
}
pub(crate) struct ObserverHub {
    epoch: StoreEpoch,
    limits: ObserverLimits,
    next_subscription: u64,
    journals: HashMap<DocId, Journal>,
    subscriptions: HashMap<SubscriptionId, Subscription>,
    active_doc: Option<DocId>,
    pub(crate) pending_content: Option<Receipt>,
}
static NEXT_STORE: AtomicU64 = AtomicU64::new(1);
fn session_nonce() -> [u64; 2] {
    use std::hash::{BuildHasher, Hash, Hasher};
    static NONCE: std::sync::OnceLock<[u64; 2]> = std::sync::OnceLock::new();
    *NONCE.get_or_init(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        [0u64, 1u64].map(|salt| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            now.hash(&mut h);
            std::process::id().hash(&mut h);
            salt.hash(&mut h);
            h.finish()
        })
    })
}
impl ObserverHub {
    pub(crate) fn new() -> Self {
        Self {
            epoch: StoreEpoch {
                session: session_nonce(),
                instance: NEXT_STORE
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |x| x.checked_add(1))
                    .expect("StoreEpoch exhausted; refusing a reused identity"),
            },
            limits: ObserverLimits::default(),
            next_subscription: 1,
            journals: HashMap::new(),
            subscriptions: HashMap::new(),
            active_doc: None,
            pending_content: None,
        }
    }
}

impl Store {
    pub fn set_observer_limits(&mut self, limits: ObserverLimits) -> Result<(), ObserverError> {
        if !self.observer.journals.is_empty() || !self.observer.subscriptions.is_empty() {
            return Err(ObserverError::InvalidCapacity);
        }
        if limits.journal_batches > 4096
            || limits.max_queue_batches == 0
            || limits.max_queue_batches > 4096
            || limits.max_subscriptions == 0
            || limits.max_subscriptions > 4096
        {
            return Err(ObserverError::InvalidCapacity);
        }
        self.observer.limits = limits;
        Ok(())
    }
    fn full_observer_state(&self, doc: DocId) -> ObserverState {
        let d = &self.docs[&doc];
        let mut views: BTreeMap<_, _> = d
            .views
            .values()
            .map(|v| {
                (
                    v.id,
                    ObservedView::Open {
                        state: v.clone(),
                        folds: Unimplemented::NotImplemented,
                        logical_focus: Unimplemented::NotImplemented,
                        composition: Unimplemented::NotImplemented,
                        pending_mode: Unimplemented::NotImplemented,
                    },
                )
            })
            .collect();
        views.extend(
            d.closed_views
                .iter()
                .map(|(id, v)| (*id, ObservedView::Closed(v.clone()))),
        );
        ObserverState {
            global: Arc::new(ObservedGlobal {
                document: d.current.clone(),
                metadata: self.observer_metadata(doc),
            }),
            views,
        }
    }
    pub(crate) fn register_observer_document(&mut self, doc: DocId) {
        let state = self.full_observer_state(doc);
        self.observer.journals.insert(
            doc,
            Journal {
                head: EventSeq(0),
                last: state,
                log: VecDeque::new(),
            },
        );
    }
    pub(crate) fn observed_write<T>(
        &mut self,
        doc: DocId,
        f: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        if let Some(active) = self.observer.active_doc {
            assert_eq!(active, doc, "cross-document nested mutation");
            return f(self);
        }
        let journal = self.observer.journals.get(&doc).ok_or(Error::InvalidDoc)?;
        journal
            .head
            .0
            .checked_add(2)
            .ok_or(Error::CounterExhausted)?;
        self.observer.active_doc = Some(doc);
        self.observer.pending_content = None;
        let result = f(self);
        self.observer.active_doc = None;
        if result.is_ok() {
            self.publish_observer_document(doc);
        } else {
            assert!(
                self.observer.pending_content.is_none(),
                "error after content publication"
            );
        }
        result
    }
    fn publish_observer_document(&mut self, doc: DocId) {
        let after = self.full_observer_state(doc);
        let j = self.observer.journals.get_mut(&doc).unwrap();
        let global = (after.global != j.last.global).then(|| after.global.clone());
        let views: Vec<_> = after
            .views
            .iter()
            .filter(|(id, v)| j.last.views.get(id) != Some(v))
            .map(|(_, v)| v.clone())
            .collect();
        let pending = self.observer.pending_content.take();
        if global.is_none() && views.is_empty() {
            assert!(pending.is_none());
            return;
        }
        let seq = EventSeq(
            j.head
                .0
                .checked_add(1)
                .expect("publication was preflighted"),
        );
        let content = pending.map(|r| {
            Arc::new(ContentChange {
                transaction: PublicationId {
                    store: self.observer.epoch,
                    doc,
                    seq,
                },
                group: r.history_group.expect("content commit group"),
                previous: r.previous,
                revision: r.snapshot.revision,
                token: r.snapshot.token,
                forward: r.forward.into(),
                inverse: r.inverse.into(),
            })
        });
        let record = Arc::new(Record {
            seq,
            global,
            views,
            content,
        });
        j.head = seq;
        j.last = after;
        j.log.push_back(record.clone());
        while j.log.len() > self.observer.limits.journal_batches {
            j.log.pop_front();
        }
        let earliest = j.log.front().map_or(EventSeq(seq.0 + 1), |r| r.seq);
        for sub in self
            .observer
            .subscriptions
            .values_mut()
            .filter(|s| s.handle.baseline.doc == doc && s.lagged.is_none())
        {
            if sub.queue.len() == sub.handle.capacity {
                sub.queue.clear();
                let mut cursor = sub.handle.baseline.clone();
                cursor.seq = seq;
                sub.lagged = Some(LaggedInfo {
                    cursor,
                    subscription: sub.handle.id,
                    registered_at: sub.handle.baseline.seq,
                    earliest,
                    head: seq,
                    capacity: sub.handle.capacity,
                });
            } else {
                sub.queue.push_back(record.clone());
            }
        }
    }
    fn validate_scope(&self, doc: DocId, scope: &ObserverScope) -> Result<(), ObserverError> {
        if scope.schema != OBSERVER_SCHEMA {
            return Err(ObserverError::UnsupportedSchema);
        }
        if scope.views.len() > MAX_SCOPE_VIEWS || ObserverScope::new(scope.views.clone()) != *scope
        {
            return Err(ObserverError::InvalidScope);
        }
        let d = self.docs.get(&doc).ok_or(ObserverError::DocGone)?;
        if scope
            .views
            .iter()
            .any(|id| !d.views.contains_key(id) && !d.closed_views.contains_key(id))
        {
            return Err(ObserverError::InvalidScope);
        }
        Ok(())
    }
    fn register_subscription(
        &mut self,
        baseline: Cursor,
        capacity: usize,
        replay: VecDeque<Arc<Record>>,
    ) -> Result<SubscriptionHandle, ObserverError> {
        if capacity == 0 || capacity > self.observer.limits.max_queue_batches {
            return Err(ObserverError::InvalidCapacity);
        }
        if self.observer.subscriptions.len() >= self.observer.limits.max_subscriptions {
            return Err(ObserverError::SubscriptionLimit);
        }
        let id = SubscriptionId(self.observer.next_subscription);
        let next = id.0.checked_add(1).ok_or(ObserverError::CounterExhausted)?;
        let handle = SubscriptionHandle {
            id,
            baseline,
            capacity,
        };
        self.observer.subscriptions.insert(
            id,
            Subscription {
                handle: handle.clone(),
                queue: replay,
                lagged: None,
            },
        );
        self.observer.next_subscription = next;
        Ok(handle)
    }
    pub fn observe_and_subscribe(
        &mut self,
        doc: DocId,
        scope: ObserverScope,
        capacity: usize,
    ) -> Result<(ObserverSnapshot, SubscriptionHandle), ObserverError> {
        self.validate_scope(doc, &scope)?;
        let j = &self.observer.journals[&doc];
        let cursor = Cursor {
            store: self.observer.epoch,
            doc,
            scope: scope.clone(),
            seq: j.head,
        };
        let state = ObserverState {
            global: j.last.global.clone(),
            views: scope
                .views
                .iter()
                .map(|id| (*id, j.last.views[id].clone()))
                .collect(),
        };
        let handle = self.register_subscription(cursor.clone(), capacity, VecDeque::new())?;
        Ok((
            ObserverSnapshot {
                cursor,
                subscription: handle.id,
                state,
            },
            handle,
        ))
    }
    pub fn resume(
        &mut self,
        doc: DocId,
        scope: ObserverScope,
        cached: &Cursor,
        capacity: usize,
    ) -> Result<SubscriptionHandle, ObserverError> {
        if cached.store != self.observer.epoch {
            return Err(ObserverError::WrongStore);
        }
        if cached.doc != doc {
            return Err(ObserverError::WrongDoc);
        }
        if cached.scope != scope {
            return Err(ObserverError::ScopeMismatch);
        }
        self.validate_scope(doc, &scope)?;
        if capacity == 0 || capacity > self.observer.limits.max_queue_batches {
            return Err(ObserverError::InvalidCapacity);
        }
        let j = &self.observer.journals[&doc];
        let head = j.head;
        let earliest = j
            .log
            .front()
            .map_or(EventSeq(head.0.saturating_add(1)), |r| r.seq);
        if cached.seq > head {
            return Err(ObserverError::CursorAhead);
        }
        if cached.seq.0 < earliest.0.saturating_sub(1) {
            return Err(ObserverError::CursorTooOld { earliest, head });
        }
        let replay: VecDeque<_> = j
            .log
            .iter()
            .filter(|r| r.seq > cached.seq)
            .cloned()
            .collect();
        if replay.len() as u64 != head.0 - cached.seq.0
            || replay
                .iter()
                .enumerate()
                .any(|(i, r)| r.seq.0 != cached.seq.0 + i as u64 + 1)
        {
            return Err(ObserverError::CursorTooOld { earliest, head });
        }
        if replay.len() > capacity {
            let mut cursor = cached.clone();
            cursor.seq = head;
            return Err(ObserverError::Lagged(LaggedInfo {
                cursor,
                subscription: SubscriptionId(0),
                registered_at: cached.seq,
                earliest,
                head,
                capacity,
            }));
        }
        self.register_subscription(cached.clone(), capacity, replay)
    }
    pub fn poll_observer(
        &mut self,
        handle: &SubscriptionHandle,
    ) -> Result<ObserverPoll, ObserverError> {
        let sub = self
            .observer
            .subscriptions
            .get_mut(&handle.id)
            .ok_or(ObserverError::StaleSubscription)?;
        if sub.handle.baseline != handle.baseline || sub.handle.capacity != handle.capacity {
            return Err(ObserverError::StaleSubscription);
        }
        if !self.docs.contains_key(&handle.baseline.doc) {
            self.observer.subscriptions.remove(&handle.id);
            return Ok(ObserverPoll::DocGone {
                cursor: handle.baseline.clone(),
                subscription: handle.id,
            });
        }
        if let Some(mut info) = sub.lagged.take() {
            let j = &self.observer.journals[&handle.baseline.doc];
            info.head = j.head;
            info.cursor.seq = j.head;
            info.earliest = j.log.front().map_or(EventSeq(j.head.0 + 1), |r| r.seq);
            self.observer.subscriptions.remove(&handle.id);
            return Ok(ObserverPoll::Lagged(info));
        }
        let Some(record) = sub.queue.pop_front() else {
            return Ok(ObserverPoll::Pending);
        };
        let mut events = Vec::new();
        if let Some(c) = &record.content {
            events.push(ObserverEvent::Content(c.clone()));
        }
        if let Some(g) = &record.global {
            events.push(ObserverEvent::Global(g.clone()));
        }
        events.extend(
            record
                .views
                .iter()
                .filter(|v| handle.baseline.scope.views.contains(&v.id()))
                .cloned()
                .map(ObserverEvent::View),
        );
        let mut cursor = handle.baseline.clone();
        cursor.seq = record.seq;
        Ok(ObserverPoll::Batch(EventBatch {
            cursor,
            subscription: handle.id,
            events,
        }))
    }
    pub fn unsubscribe(&mut self, handle: &SubscriptionHandle) -> Result<(), ObserverError> {
        let sub = self
            .observer
            .subscriptions
            .get(&handle.id)
            .ok_or(ObserverError::StaleSubscription)?;
        if sub.handle.baseline != handle.baseline {
            return Err(ObserverError::StaleSubscription);
        }
        self.observer.subscriptions.remove(&handle.id);
        Ok(())
    }
    pub fn close_document(&mut self, doc: DocId) -> Result<(), Error> {
        if self.has_active_save(doc) {
            return Err(Error::Busy);
        }
        self.remove_memory_document(doc)?;
        self.observer.journals.remove(&doc);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheOutcome {
    Buffered,
    Applied,
    IgnoredDuplicate,
    IgnoredOldHandle,
    Stale,
}
struct Pending {
    cursor: Cursor,
    parts: usize,
    events: BTreeMap<usize, ObserverEvent>,
}
pub struct ObserverCache {
    baseline: ObserverSnapshot,
    state: Option<ObserverState>,
    cursor: Cursor,
    stale: bool,
    installed: bool,
    buffer_limit: usize,
    buffer: Vec<ObserverFrame>,
    pending: Option<Pending>,
    last_batch: Option<EventBatch>,
}
impl ObserverCache {
    /// A resume rejection is a control result, not a new stream or a new cursor.
    /// Clients must call this for the response to their own complete cursor.
    pub fn resume_rejected(&mut self, error: &ObserverError) -> Result<(), ObserverError> {
        if let ObserverError::Lagged(info) = error {
            if info.cursor.store != self.cursor.store
                || info.cursor.doc != self.cursor.doc
                || info.cursor.scope != self.cursor.scope
                || info.registered_at != self.cursor.seq
            {
                return self.invalidate(ObserverError::Protocol).map(|_| ());
            }
        }
        if matches!(
            error,
            ObserverError::Lagged(_)
                | ObserverError::CursorTooOld { .. }
                | ObserverError::CursorAhead
                | ObserverError::WrongStore
                | ObserverError::DocGone
        ) {
            self.stale = true;
            self.pending = None;
            self.buffer.clear();
        }
        Ok(())
    }
    pub fn receive_doc_gone(
        &mut self,
        cursor: &Cursor,
        subscription: SubscriptionId,
    ) -> Result<CacheOutcome, ObserverError> {
        if subscription != self.baseline.subscription || cursor.store != self.cursor.store {
            return Ok(CacheOutcome::IgnoredOldHandle);
        }
        if cursor.doc != self.cursor.doc || cursor.scope != self.cursor.scope {
            return self.invalidate(ObserverError::Protocol);
        }
        self.stale = true;
        self.pending = None;
        self.buffer.clear();
        Ok(CacheOutcome::Stale)
    }
    pub fn prepare(snapshot: ObserverSnapshot, buffer_limit: usize) -> Result<Self, ObserverError> {
        if buffer_limit == 0 || buffer_limit > 65536 {
            return Err(ObserverError::InvalidCapacity);
        }
        validate_state(&snapshot.cursor, &snapshot.state)?;
        Ok(Self {
            cursor: snapshot.cursor.clone(),
            baseline: snapshot,
            state: None,
            stale: false,
            installed: false,
            buffer_limit,
            buffer: vec![],
            pending: None,
            last_batch: None,
        })
    }
    pub fn state(&self) -> Option<&ObserverState> {
        self.state.as_ref()
    }
    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }
    pub fn is_stale(&self) -> bool {
        self.stale
    }
    pub fn is_installed(&self) -> bool {
        self.installed
    }
    pub fn is_partial(&self) -> bool {
        self.pending.is_some() || !self.buffer.is_empty()
    }
    pub fn cursor_for_resume(&self) -> Result<Cursor, ObserverError> {
        if self.stale || !self.installed || self.is_partial() {
            Err(ObserverError::NeedsObserve)
        } else {
            Ok(self.cursor.clone())
        }
    }
    fn invalidate(&mut self, error: ObserverError) -> Result<CacheOutcome, ObserverError> {
        self.stale = true;
        self.pending = None;
        self.buffer.clear();
        Err(error)
    }
    pub fn install(&mut self) -> Result<(), ObserverError> {
        if self.stale || self.installed {
            return Err(ObserverError::NeedsObserve);
        }
        self.state = Some(self.baseline.state.clone());
        self.installed = true;
        let buffered = std::mem::take(&mut self.buffer);
        for frame in buffered {
            self.receive(frame)?;
        }
        Ok(())
    }
    pub fn rebind_resume(&mut self, handle: &SubscriptionHandle) -> Result<(), ObserverError> {
        if self.cursor_for_resume()? != handle.baseline {
            return Err(ObserverError::Protocol);
        }
        self.baseline.subscription = handle.id;
        self.last_batch = None;
        Ok(())
    }
    pub fn receive_lagged(&mut self, info: &LaggedInfo) -> Result<CacheOutcome, ObserverError> {
        if info.subscription != self.baseline.subscription || info.cursor.store != self.cursor.store
        {
            return Ok(CacheOutcome::IgnoredOldHandle);
        }
        if info.cursor.doc != self.cursor.doc || info.cursor.scope != self.cursor.scope {
            return self.invalidate(ObserverError::Protocol);
        }
        self.stale = true;
        self.pending = None;
        self.buffer.clear();
        Ok(CacheOutcome::Stale)
    }
    pub fn receive(&mut self, frame: ObserverFrame) -> Result<CacheOutcome, ObserverError> {
        let (cursor, subscription) = frame.identity();
        if subscription != self.baseline.subscription || cursor.store != self.cursor.store {
            return Ok(CacheOutcome::IgnoredOldHandle);
        }
        if cursor.doc != self.cursor.doc
            || cursor.scope != self.cursor.scope
            || cursor.scope.schema != OBSERVER_SCHEMA
            || matches!(frame, ObserverFrame::Unknown { .. })
        {
            return self.invalidate(ObserverError::Protocol);
        }
        if self.stale {
            return Err(ObserverError::NeedsObserve);
        }
        if !self.installed {
            if self.buffer.len() == self.buffer_limit {
                return self.invalidate(ObserverError::BufferOverflow);
            }
            self.buffer.push(frame);
            return Ok(CacheOutcome::Buffered);
        }
        if cursor.seq <= self.cursor.seq {
            if let Some(last) = self
                .last_batch
                .as_ref()
                .filter(|b| b.cursor.seq == cursor.seq)
            {
                if !last.frames().contains(&frame) {
                    return self.invalidate(ObserverError::Protocol);
                }
            }
            return Ok(CacheOutcome::IgnoredDuplicate);
        }
        if cursor.seq.0
            != self
                .cursor
                .seq
                .0
                .checked_add(1)
                .ok_or(ObserverError::CounterExhausted)?
        {
            return self.invalidate(ObserverError::Protocol);
        }
        match frame {
            ObserverFrame::Begin { cursor, parts, .. } => {
                if parts > cursor.scope.views.len() + 2 {
                    return self.invalidate(ObserverError::Protocol);
                }
                if let Some(p) = &self.pending {
                    if p.cursor != cursor || p.parts != parts {
                        return self.invalidate(ObserverError::Protocol);
                    }
                    return Ok(CacheOutcome::IgnoredDuplicate);
                }
                self.pending = Some(Pending {
                    cursor,
                    parts,
                    events: BTreeMap::new(),
                });
                Ok(CacheOutcome::Buffered)
            }
            ObserverFrame::Part {
                cursor,
                index,
                event,
                ..
            } => {
                let Some(p) = &mut self.pending else {
                    return self.invalidate(ObserverError::Protocol);
                };
                if p.cursor != cursor || index >= p.parts {
                    return self.invalidate(ObserverError::Protocol);
                }
                if let Some(old) = p.events.get(&index) {
                    if old != &event {
                        return self.invalidate(ObserverError::Protocol);
                    }
                    return Ok(CacheOutcome::IgnoredDuplicate);
                }
                p.events.insert(index, event);
                Ok(CacheOutcome::Buffered)
            }
            ObserverFrame::End { cursor, parts, .. } => {
                let Some(p) = self.pending.take() else {
                    return self.invalidate(ObserverError::Protocol);
                };
                if p.cursor != cursor || p.parts != parts || p.events.len() != parts {
                    return self.invalidate(ObserverError::Protocol);
                }
                let events: Vec<_> = p.events.into_values().collect();
                let mut next = self.state.as_ref().unwrap().clone();
                let mut content = None;
                let mut global = false;
                let mut changed_views = std::collections::BTreeSet::new();
                for event in &events {
                    match event {
                        ObserverEvent::Content(c) => {
                            if content.is_some()
                                || c.transaction
                                    != (PublicationId {
                                        store: cursor.store,
                                        doc: cursor.doc,
                                        seq: cursor.seq,
                                    })
                            {
                                return self.invalidate(ObserverError::Protocol);
                            }
                            content = Some(c.clone());
                        }
                        ObserverEvent::Global(g) => {
                            if global {
                                return self.invalidate(ObserverError::Protocol);
                            }
                            global = true;
                            next.global = g.clone();
                        }
                        ObserverEvent::View(v) => {
                            if !cursor.scope.views.contains(&v.id())
                                || !changed_views.insert(v.id())
                            {
                                return self.invalidate(ObserverError::Protocol);
                            }
                            let increasing = match (next.views.get(&v.id()), v) {
                                (
                                    Some(ObservedView::Open { state: old, .. }),
                                    ObservedView::Open { state: new, .. },
                                ) => new.epoch > old.epoch,
                                (
                                    Some(ObservedView::Open { state: old, .. }),
                                    ObservedView::Closed(new),
                                ) => new.epoch > old.epoch,
                                _ => false,
                            };
                            if !increasing {
                                return self.invalidate(ObserverError::Protocol);
                            }
                            next.views.insert(v.id(), v.clone());
                        }
                    }
                }
                let old = &self.state.as_ref().unwrap().global.document;
                let new = &next.global.document;
                if let Some(c) = content {
                    if !global
                        || c.previous != old.revision
                        || c.revision != new.revision
                        || c.token != new.token
                        || new.revision.0
                            != old
                                .revision
                                .0
                                .checked_add(1)
                                .ok_or(ObserverError::CounterExhausted)?
                    {
                        return self.invalidate(ObserverError::Protocol);
                    }
                } else if new.revision != old.revision || new.token != old.token {
                    return self.invalidate(ObserverError::Protocol);
                }
                if validate_state(&cursor, &next).is_err() {
                    return self.invalidate(ObserverError::Protocol);
                }
                self.state = Some(next);
                self.cursor = cursor.clone();
                self.last_batch = Some(EventBatch {
                    cursor,
                    subscription: self.baseline.subscription,
                    events,
                });
                Ok(CacheOutcome::Applied)
            }
            ObserverFrame::Unknown { .. } => unreachable!(),
        }
    }
}
fn validate_state(cursor: &Cursor, state: &ObserverState) -> Result<(), ObserverError> {
    if cursor.scope.schema != OBSERVER_SCHEMA
        || ObserverScope::new(cursor.scope.views.clone()) != cursor.scope
        || cursor.scope.views.len() > MAX_SCOPE_VIEWS
    {
        return Err(ObserverError::Protocol);
    }
    let g = &state.global;
    if g.document.doc != cursor.doc
        || g.metadata.dirty != (g.document.token != g.metadata.baseline.token)
        || state.views.keys().copied().collect::<Vec<_>>() != cursor.scope.views
    {
        return Err(ObserverError::Protocol);
    }
    for (id, v) in &state.views {
        match v {
            ObservedView::Open { state, .. } => {
                if state.id != *id
                    || state.doc != cursor.doc
                    || crate::views::validate_selection(&g.document, &state.selection).is_err()
                    || crate::views::validate_anchor(&g.document, state.scroll).is_err()
                {
                    return Err(ObserverError::Protocol);
                }
            }
            ObservedView::Closed(v) => {
                if v.id != *id || v.doc != cursor.doc {
                    return Err(ObserverError::Protocol);
                }
            }
        }
    }
    Ok(())
}
