use serde::{Deserialize, Serialize};
use std::ops::{ControlFlow, Range};

pub const MAX_VISIBLE: usize = 5000;
pub const MAX_SELECTIONS: usize = 5000;
pub const MAX_CHECKPOINTS: usize = 2048;
const MIN_STRIDE: usize = 64 * 1024;

/// Byte anchors are for an initial editor selection. Subsequent navigation
/// uses the 1-based ordinal of the precise match, independent of any UI
/// selection rounding to graphemes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Navigation {
    NextFrom(usize),
    PreviousFrom(usize),
    NextAfter(u64),
    PreviousBefore(u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    pub ordinal: u64,
    pub range: Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionLimit {
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Restart in the complete subject here, using the original query.
    pub byte: usize,
    /// Number of in-scope hits before this continuation.
    pub count: u64,
    pub previous: Option<Range<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoints {
    subject_len: usize,
    stride: usize,
    points: Vec<Checkpoint>,
}

impl Checkpoints {
    fn new(subject_len: usize) -> Self {
        Self {
            subject_len,
            stride: subject_len.div_ceil(MAX_CHECKPOINTS - 1).max(MIN_STRIDE),
            points: vec![Checkpoint {
                byte: 0,
                count: 0,
                previous: None,
            }],
        }
    }

    pub fn points(&self) -> &[Checkpoint] {
        &self.points
    }

    pub fn before_byte(&self, byte: usize) -> &Checkpoint {
        let end = self.points.partition_point(|p| p.byte <= byte);
        &self.points[end.saturating_sub(1)]
    }

    /// The ordinal being sought must still be discoverable after this point.
    pub fn before_ordinal(&self, ordinal: u64) -> &Checkpoint {
        let end = self.points.partition_point(|p| p.count < ordinal);
        &self.points[end.saturating_sub(1)]
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResults {
    pub total: u64,
    pub first: Option<Hit>,
    pub last: Option<Hit>,
    pub current: Option<Hit>,
    pub wrapped: bool,
    pub visible: Vec<Range<usize>>,
    /// When saturated, ranges are sampled across the entire visible band.
    /// The independently retained current match may always be drawn.
    pub visible_sampled: bool,
    pub selected: Option<Result<Vec<Range<usize>>, SelectionLimit>>,
    pub index: Checkpoints,
}

/// Feed only matches accepted by the full-subject search and frozen scope,
/// in their global iteration order. Stores no per-hit state outside the
/// explicitly requested viewport, selections, and sparse continuations.
pub struct Collector {
    nav: Navigation,
    total: u64,
    first: Option<Hit>,
    last: Option<Hit>,
    current: Option<Hit>,
    window: Visible,
    select_cap: usize,
    selected: Option<Result<Vec<Range<usize>>, SelectionLimit>>,
    index: Checkpoints,
    next_checkpoint: usize,
}

impl Collector {
    pub fn new(
        subject_len: usize,
        nav: Navigation,
        viewport: Range<usize>,
        select_cap: Option<usize>,
    ) -> Self {
        let index = Checkpoints::new(subject_len);
        Self {
            nav,
            total: 0,
            first: None,
            last: None,
            current: None,
            window: Visible::new(viewport, subject_len),
            select_cap: select_cap.unwrap_or(0).min(MAX_SELECTIONS),
            selected: select_cap.map(|_| Ok(Vec::new())),
            next_checkpoint: index.stride,
            index,
        }
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn first(&self) -> Option<&Hit> {
        self.first.as_ref()
    }

    /// Current navigation candidate during a partial scan. A backwards
    /// search or wrap may still require reaching the anchor or the end.
    pub fn current(&self) -> Option<&Hit> {
        self.current.as_ref()
    }

    /// `resume_after` is the global iterator's real next search offset. A
    /// terminal zero-width hit has none. Returns true on the first match, so
    /// the UI can display it before waiting for the complete count.
    pub fn record(&mut self, range: Range<usize>, resume_after: Option<usize>) -> bool {
        self.total += 1;
        let hit = Hit {
            ordinal: self.total,
            range,
        };
        let first = self.total == 1;
        if first {
            self.first = Some(hit.clone());
        }
        let choose = match self.nav {
            Navigation::NextFrom(at) => self.current.is_none() && hit.range.start >= at,
            Navigation::PreviousFrom(at) => hit.range.end <= at,
            Navigation::NextAfter(n) => self.current.is_none() && hit.ordinal > n,
            Navigation::PreviousBefore(n) => hit.ordinal < n,
        };
        if choose {
            self.current = Some(hit.clone());
        }
        self.window.add(&hit.range);
        if let Some(selected) = &mut self.selected
            && let Ok(ranges) = selected
        {
            if ranges.len() == self.select_cap {
                *selected = Err(SelectionLimit {
                    limit: self.select_cap,
                });
            } else {
                ranges.push(hit.range.clone());
            }
        }
        self.last = Some(hit);
        if let Some(resume) = resume_after {
            self.progress(resume);
        }
        first
    }

    /// Also accepts an engine-proven safe continuation in an unmatched span.
    /// An arbitrary byte bucket would break global overlap and regex anchors.
    pub fn progress(&mut self, resume: usize) {
        if resume < self.next_checkpoint || resume > self.index.subject_len {
            return;
        }
        self.index.points.push(Checkpoint {
            byte: resume,
            count: self.total,
            previous: self.last.as_ref().map(|hit| hit.range.clone()),
        });
        self.next_checkpoint = resume.saturating_add(self.index.stride);
    }

    /// Call only after the engine reports EOF for the same revision.
    pub fn finish(self) -> SearchResults {
        let wrapped = self.current.is_none() && self.first.is_some();
        let current = self.current.or_else(|| match self.nav {
            Navigation::NextFrom(_) | Navigation::NextAfter(_) => self.first.clone(),
            Navigation::PreviousFrom(_) | Navigation::PreviousBefore(_) => self.last.clone(),
        });
        SearchResults {
            total: self.total,
            first: self.first,
            last: self.last,
            current,
            wrapped,
            visible: self.window.hits,
            visible_sampled: self.window.sampled,
            selected: self.selected,
            index: self.index,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Visible {
    band: Range<usize>,
    subject_len: usize,
    bin_width: usize,
    sampled: bool,
    last_bin: Option<usize>,
    hits: Vec<Range<usize>>,
}

impl Visible {
    fn new(range: Range<usize>, subject_len: usize) -> Self {
        let start = range.start.min(subject_len);
        let end = range.end.clamp(start, subject_len);
        Self {
            band: start..end,
            subject_len,
            bin_width: (end - start).div_ceil(MAX_VISIBLE).max(1),
            sampled: false,
            last_bin: None,
            hits: Vec::new(),
        }
    }

    fn past(&self, byte: usize) -> bool {
        byte > self.band.end || (byte == self.band.end && byte != self.subject_len)
    }

    fn add(&mut self, hit: &Range<usize>) {
        let visible = if hit.is_empty() {
            self.band.start <= hit.start && !self.past(hit.start)
        } else {
            !self.band.is_empty() && hit.start < self.band.end && self.band.start < hit.end
        };
        if !visible {
            return;
        }
        if !self.sampled && self.hits.len() < MAX_VISIBLE {
            self.hits.push(hit.clone());
            return;
        }
        let bin = |byte: usize| {
            (byte.saturating_sub(self.band.start) / self.bin_width).min(MAX_VISIBLE - 1)
        };
        if !self.sampled {
            self.sampled = true;
            self.hits.retain(|r| {
                let current = bin(r.start);
                self.last_bin.replace(current) != Some(current)
            });
        }
        let current = bin(hit.start);
        if self.last_bin.replace(current) != Some(current) {
            self.hits.push(hit.clone());
        }
    }
}

impl SearchResults {
    pub fn navigation(&self, nav: Navigation) -> NavigationReplay {
        NavigationReplay::new(self, nav)
    }

    pub fn viewport(&self, range: Range<usize>) -> VisibleReplay {
        let start = self.index.before_byte(range.start).byte;
        VisibleReplay {
            start,
            window: Visible::new(range, self.index.subject_len),
        }
    }
}

#[derive(Serialize, Deserialize)]
enum Seek {
    Next(usize),
    Previous(usize),
    Ordinal(u64),
}

#[derive(Serialize, Deserialize)]
pub struct NavigationReplay {
    start: usize,
    count: u64,
    seek: Option<Seek>,
    hit: Option<Hit>,
    wrapped: bool,
}

impl NavigationReplay {
    fn new(results: &SearchResults, nav: Navigation) -> Self {
        let mut replay = Self {
            start: 0,
            count: 0,
            seek: None,
            hit: None,
            wrapped: false,
        };
        let (Some(first), Some(last)) = (&results.first, &results.last) else {
            return replay;
        };
        let seek = match nav {
            Navigation::NextFrom(at) if at <= first.range.start => {
                replay.hit = Some(first.clone());
                return replay;
            }
            Navigation::NextFrom(at) if at > last.range.start => {
                replay.hit = Some(first.clone());
                replay.wrapped = true;
                return replay;
            }
            Navigation::PreviousFrom(at) if at >= last.range.end => {
                replay.hit = Some(last.clone());
                return replay;
            }
            Navigation::PreviousFrom(at) if at < first.range.end => {
                replay.hit = Some(last.clone());
                replay.wrapped = true;
                return replay;
            }
            Navigation::NextFrom(at) => Seek::Next(at),
            Navigation::PreviousFrom(at) => Seek::Previous(at),
            Navigation::NextAfter(n) if n >= results.total => {
                replay.hit = Some(first.clone());
                replay.wrapped = true;
                return replay;
            }
            Navigation::PreviousBefore(n) if n <= 1 => {
                replay.hit = Some(last.clone());
                replay.wrapped = true;
                return replay;
            }
            Navigation::NextAfter(n) => Seek::Ordinal(n + 1),
            Navigation::PreviousBefore(n) => Seek::Ordinal((n - 1).min(results.total)),
        };
        let checkpoint = match seek {
            Seek::Next(at) | Seek::Previous(at) => results.index.before_byte(at),
            Seek::Ordinal(n) => results.index.before_ordinal(n),
        };
        replay.start = checkpoint.byte;
        replay.count = checkpoint.count;
        if matches!(seek, Seek::Previous(_)) {
            replay.hit = checkpoint.previous.as_ref().map(|range| Hit {
                ordinal: checkpoint.count,
                range: range.clone(),
            });
        }
        replay.seek = Some(seek);
        replay
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn done(&self) -> bool {
        self.seek.is_none()
    }

    pub fn record(&mut self, range: Range<usize>) -> ControlFlow<()> {
        self.count += 1;
        match self.seek {
            Some(Seek::Next(at)) if range.start < at => return ControlFlow::Continue(()),
            Some(Seek::Ordinal(n)) if self.count < n => return ControlFlow::Continue(()),
            Some(Seek::Previous(at)) if range.end <= at => {
                self.hit = Some(Hit {
                    ordinal: self.count,
                    range,
                });
                return ControlFlow::Continue(());
            }
            Some(Seek::Next(_) | Seek::Ordinal(_)) => {
                self.hit = Some(Hit {
                    ordinal: self.count,
                    range,
                });
            }
            Some(Seek::Previous(_)) | None => {}
        }
        self.seek = None;
        ControlFlow::Break(())
    }

    pub fn progress(&mut self, resume: usize) -> ControlFlow<()> {
        if matches!(self.seek, Some(Seek::Previous(at)) if resume > at) {
            self.seek = None;
        }
        if self.done() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    pub fn finish(self) -> (Option<Hit>, bool) {
        (self.hit, self.wrapped)
    }
}

#[derive(Serialize, Deserialize)]
pub struct VisibleReplay {
    start: usize,
    window: Visible,
}

impl VisibleReplay {
    pub fn start(&self) -> usize {
        self.start
    }

    pub fn record(&mut self, range: Range<usize>) -> ControlFlow<()> {
        self.progress(range.start)?;
        self.window.add(&range);
        ControlFlow::Continue(())
    }

    pub fn progress(&self, resume: usize) -> ControlFlow<()> {
        if self.window.past(resume) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    pub fn finish(self) -> Vec<Range<usize>> {
        self.window.hits
    }
}
