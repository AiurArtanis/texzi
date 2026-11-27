//! Persistent document component; not connected to NSH yet.
// Keep the document core forbidden; only the named file adapters may wrap
// native file replacement and exclusive read-only mappings.
#![deny(unsafe_code)]

#[forbid(unsafe_code)]
mod anchors;
#[forbid(unsafe_code)]
mod body;
#[forbid(unsafe_code)]
mod bulk;
#[forbid(unsafe_code)]
mod compact_table;
#[forbid(unsafe_code)]
mod coordinates;
pub mod files;
#[forbid(unsafe_code)]
mod history;
#[forbid(unsafe_code)]
mod observe;
#[forbid(unsafe_code)]
mod observed_writes;
#[forbid(unsafe_code)]
mod provenance;
#[forbid(unsafe_code)]
mod ui_plan;
#[forbid(unsafe_code)]
mod view_store;
#[forbid(unsafe_code)]
mod views;

pub use anchors::Affinity;
pub use body::{Body, Eol, Profile};
pub use bulk::{BulkMap, BulkPart, PreparedBulk, PreparedBulkBuilder, bulk_history_disk_usage};
pub use coordinates::{Coordinate, GraphemeIndex, GraphemeOffset, SourceByte, Utf16Offset};
pub use history::*;
pub use observe::*;
pub use provenance::OriginSpan;
pub use view_store::DocumentViewSnapshot;
pub use views::*;

/// Limits for the complete editable model (file import and bulk replacement output).
pub const MAX_EDIT_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_EDIT_LINES: usize = 2_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocId(pub(crate) u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Revision(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateToken(pub(crate) u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidDoc,
    StaleRevision,
    InvalidBoundary,
    InsideGrapheme,
    InvalidOverlap,
    InvalidEolPlan,
    InvalidProfile,
    Cancelled,
    LineLimit,
    CounterExhausted,
    HistoryBudget,
    NoHistory,
    Busy,
    InvalidSave,
    UnresolvedConflict,
    TooLate,
    InvalidView,
    StaleView,
    InvalidSelection,
}
