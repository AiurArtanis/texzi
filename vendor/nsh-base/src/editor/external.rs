//! texzi fork: synchronous external document owner, no second editable history.
use ropey::Rope;
use std::{fmt, ops::Range, rc::Rc};

/// Source-byte positions, kept apart from the public UTF-16 line/column API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalSelections {
    pub ranges: Vec<(usize, usize)>,
    pub primary: usize,
}
#[derive(Clone)]
pub struct ExternalSnapshot {
    pub revision: u64,
    pub view_epoch: u64,
    pub rope: Rope,
    pub selections: ExternalSelections,
}
pub struct ExternalUpdate {
    pub snapshot: ExternalSnapshot,
    /// Sequential replacements of the old snapshot. None means refresh after loss.
    pub edits: Option<Vec<(Range<usize>, String)>>,
}
#[derive(Clone, Copy)]
pub enum ExternalEditKind {
    Typing,
    Deleting,
    ImeCommit,
    Other,
}
/// Implemented by the application. All writes validate the supplied version/epoch.
/// `track` reads an engine reactive notification, `notify` runs outside editor borrows.
pub trait ExternalDocument {
    /// Composition is transient view state, never a document transaction.
    fn composition_changed(&self, _active: bool) {}
    fn snapshot(&self) -> Result<ExternalSnapshot, String>;
    fn edit(
        &self,
        revision: u64,
        epoch: u64,
        edits: Vec<(Range<usize>, String)>,
        after: ExternalSelections,
        kind: ExternalEditKind,
    ) -> Result<ExternalUpdate, String>;
    fn history(&self, revision: u64, epoch: u64, undo: bool) -> Result<ExternalUpdate, String>;
    /// A disk-backed owner may complete this operation later and call
    /// `notify` with its new revision. Until then the editor keeps its cache,
    /// selection and undo state at their present version.
    fn request_history(
        self: Rc<Self>,
        revision: u64,
        epoch: u64,
        undo: bool,
    ) -> Result<Option<ExternalUpdate>, String> {
        self.history(revision, epoch, undo).map(Some)
    }
    fn select(
        &self,
        revision: u64,
        epoch: u64,
        selections: ExternalSelections,
    ) -> Result<ExternalSnapshot, String>;
    fn can_history(&self) -> (bool, bool);
    fn fence(&self);
    fn track(&self);
    fn notify(&self);
}
/// Provide one binding in each Editor's component context. Several bindings may
/// share the same owner and notification while naming independent logical views.
#[derive(Clone)]
pub struct ExternalBuffer(pub Rc<dyn ExternalDocument>);
impl fmt::Debug for ExternalBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ExternalBuffer")
    }
}
impl ExternalSelections {
    pub(crate) fn from_internal(s: &super::types::Selections) -> Self {
        Self {
            ranges: s.all().iter().map(|s| (s.anchor, s.head)).collect(),
            primary: s.primary_index(),
        }
    }
    pub(crate) fn internal(&self) -> super::types::Selections {
        super::types::Selections::new(
            self.ranges
                .iter()
                .map(|&(a, h)| super::types::Selection::new(a, h))
                .collect(),
            self.primary,
        )
    }
}
