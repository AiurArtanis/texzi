//! Persistent document component; not connected to NSH yet.
#![forbid(unsafe_code)]

mod anchors;
mod body;
mod coordinates;
mod history;
mod observe;
mod observed_writes;
mod provenance;
mod view_store;
mod views;

pub use anchors::Affinity;
pub use body::{Body, Eol, Profile};
pub use coordinates::{Coordinate, GraphemeIndex, GraphemeOffset, SourceByte, Utf16Offset};
pub use history::*;
pub use observe::*;
pub use provenance::OriginSpan;
pub use view_store::DocumentViewSnapshot;
pub use views::*;

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
    CounterExhausted,
    HistoryBudget,
    NoHistory,
    Busy,
    InvalidSave,
    TooLate,
    InvalidView,
    StaleView,
    InvalidSelection,
}
