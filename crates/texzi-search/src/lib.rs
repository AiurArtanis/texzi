//! In-document search over an immutable revision. All ranges are UTF-8 byte
//! offsets in its complete, LF-normalized subject.

mod literal;
pub mod pcre;
mod scope;
mod summary;
mod unicode;
mod word;

pub use literal::{Cancelled, Literal, ScanEvent};
pub use ropey::{Rope, RopeBuilder};
pub use scope::Scope;
pub use summary::{
    Checkpoint, Checkpoints, Collector, Hit, MAX_CHECKPOINTS, MAX_SELECTIONS, MAX_VISIBLE,
    Navigation, NavigationReplay, SearchResults, SelectionLimit, VisibleReplay,
};
pub use texzi_word::is_word;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    pub case_sensitive: bool,
    pub whole_word: bool,
}
