//! Prepare search replacements against an immutable root, off the Store writer.
//! Unchanged and captured pieces carry their original EOL and source metadata.
mod map;
mod origins;
use crate::{
    Body, Error, Snapshot,
    body::{
        compose::{Compose, LeafCache, literal_chunks},
        history_image::HistoryRoots,
    },
};
pub use map::BulkMap;
pub(crate) use origins::{OriginBuilder, OriginMap};
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) const MAX_BODY_BYTES: usize = crate::MAX_EDIT_BYTES as usize;
const MAX_TEMPLATE_PIECES: usize = 65_536;
/// Current and high-water backing-file bytes for this process, including
/// preparations, live history, and indexes retained by observer subscribers.
pub fn bulk_history_disk_usage() -> (u64, u64) {
    crate::files::history_file::usage()
}
pub(crate) fn check(cancelled: &AtomicBool) -> Result<(), Error> {
    if cancelled.load(Ordering::Acquire) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

pub enum BulkPart<'a> {
    /// An interval anywhere in the captured source, including a lookahead capture.
    Original(Range<usize>),
    Literal(&'a str),
}
pub struct PreparedBulkBuilder {
    base: Snapshot,
    cancelled: Arc<AtomicBool>,
    output: Compose,
    positions: map::Builder,
    cache: LeafCache,
    last_match: Option<Range<usize>>,
    copied_until: usize,
    removed: usize,
    changed: bool,
    failed: Option<Error>,
    dominant: crate::Eol,
}
pub struct PreparedBulk {
    pub(crate) base: Snapshot,
    pub(crate) cancelled: Arc<AtomicBool>,
    pub(crate) change: Option<PreparedChange>,
}
pub(crate) struct PreparedChange {
    pub(crate) body: Arc<Body>,
    pub(crate) roots: HistoryRoots,
    pub(crate) positions: BulkMap,
    pub(crate) charge: usize,
}
impl PreparedBulkBuilder {
    pub fn new(snapshot: Snapshot, cancelled: Arc<AtomicBool>) -> Self {
        let dominant = snapshot.body.dominant_eol();
        let output = Compose::new(&snapshot.body);
        Self {
            base: snapshot,
            cancelled,
            output,
            positions: map::Builder::default(),
            cache: LeafCache::default(),
            last_match: None,
            copied_until: 0,
            removed: 0,
            changed: false,
            failed: None,
            dominant,
        }
    }
    /// Accept a single non-overlapping match. Any invalid/unfinished input
    /// poisons this preparation, even if a caller ignores this result.
    pub fn replace<'a>(
        &mut self,
        range: Range<usize>,
        parts: impl IntoIterator<Item = BulkPart<'a>>,
    ) -> Result<(), Error> {
        if let Some(reason) = &self.failed {
            return Err(reason.clone());
        }
        let result = self.replace_inner(range, parts);
        if let Err(reason) = &result {
            self.failed = Some(reason.clone());
        }
        result
    }
    fn replace_inner<'a>(
        &mut self,
        range: Range<usize>,
        parts: impl IntoIterator<Item = BulkPart<'a>>,
    ) -> Result<(), Error> {
        check(&self.cancelled)?;
        // Captured-source strides are signed 32-bit words. The largest
        // supported complete file decodes well within this range.
        if self.base.body.len_bytes() > i32::MAX as usize {
            return Err(Error::HistoryBudget);
        }
        if self
            .last_match
            .as_ref()
            .is_some_and(|last| range.start < last.end || range.start == last.start)
        {
            return Err(Error::InvalidOverlap);
        }
        self.cache.valid(&self.base.body, &range)?;
        self.last_match = Some(range.clone());
        let mut parts = parts.into_iter();
        let Some(first) = parts.next() else {
            return self.emit(range, &[]);
        };
        let Some(second) = parts.next() else {
            return self.emit(range, std::slice::from_ref(&first));
        };
        let mut pieces = vec![first, second];
        for part in parts {
            if pieces.len() >= MAX_TEMPLATE_PIECES {
                return Err(Error::HistoryBudget);
            }
            pieces.push(part);
        }
        self.emit(range, &pieces)
    }
    fn emit(&mut self, range: Range<usize>, pieces: &[BulkPart<'_>]) -> Result<(), Error> {
        for part in pieces {
            if let BulkPart::Original(r) = part {
                self.cache.valid(&self.base.body, r)?;
            }
        }
        if self.identical(&range, pieces)? {
            return Ok(());
        }
        self.output.copy(
            &self.base.body,
            self.copied_until..range.start,
            &mut self.cache,
            &self.cancelled,
        )?;
        let start = self.output.written;
        for part in pieces {
            match part {
                BulkPart::Original(r) => self.output.copy(
                    &self.base.body,
                    r.clone(),
                    &mut self.cache,
                    &self.cancelled,
                )?,
                BulkPart::Literal(s) => literal_chunks(s, |chunk| {
                    check(&self.cancelled)?;
                    self.output.literal(chunk, self.dominant)
                })?,
            }
        }
        self.removed += range.len();
        self.copied_until = range.end;
        self.changed = true;
        self.positions
            .push(range, start, self.output.written - start)
    }
    fn identical(&mut self, range: &Range<usize>, pieces: &[BulkPart<'_>]) -> Result<bool, Error> {
        let mut cursor = range.start;
        for piece in pieces {
            check(&self.cancelled)?;
            match piece {
                BulkPart::Original(r) => {
                    if r.len() > range.end - cursor {
                        return Ok(false);
                    }
                    if r.start != cursor {
                        let mut old = cursor;
                        for chunk in self.base.body.text.byte_slice(r.clone()).chunks() {
                            check(&self.cancelled)?;
                            if !same_text(&self.base.body, &mut self.cache, old, chunk) {
                                return Ok(false);
                            }
                            old += chunk.len();
                        }
                        if self.base.body.eol_codes(r.clone())
                            != self.base.body.eol_codes(cursor..old)
                        {
                            return Ok(false);
                        }
                        if !self.base.body.same_alias_ranges(
                            r.clone(),
                            &self.base.body,
                            cursor..old,
                            &self.cancelled,
                        )? {
                            return Ok(false);
                        }
                    }
                    cursor += r.len();
                }
                BulkPart::Literal(raw) => {
                    if raw.len() > (range.end - cursor).saturating_mul(2) {
                        return Ok(false);
                    }
                    let mut same = true;
                    literal_chunks(raw, |chunk| {
                        check(&self.cancelled)?;
                        if same {
                            same = chunk.len() <= range.end - cursor
                                && same_text(&self.base.body, &mut self.cache, cursor, chunk)
                                && (!chunk.contains('\n')
                                    || self
                                        .base
                                        .body
                                        .eol_codes(cursor..cursor + chunk.len())
                                        .chars()
                                        .all(|c| crate::body::Eol::from_code(c) == self.dominant));
                            if same {
                                cursor += chunk.len();
                            }
                        }
                        Ok(())
                    })?;
                    if !same {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(cursor == range.end)
    }
    pub fn finish(mut self) -> Result<PreparedBulk, Error> {
        if let Some(reason) = self.failed {
            return Err(reason);
        }
        check(&self.cancelled)?;
        let change = if self.changed {
            self.output.copy(
                &self.base.body,
                self.copied_until..self.base.body.len_bytes(),
                &mut self.cache,
                &self.cancelled,
            )?;
            let (body, allocated) = self.output.finish(&self.base.body)?;
            if self.base.body.same_serialization(&body, &self.cancelled)? {
                None
            } else {
                let body = Arc::new(body);
                let positions = self.positions.finish()?;
                // Account discarded source content/metadata and newly materialized Rope leaves,
                // scaled for leaf/tree overhead. A larger transition is pre-backed on disk.
                let estimate = 8192 + 3 * (self.removed + allocated) + positions.allocation();
                let (roots, charge) = HistoryRoots::prepare(
                    &self.base.body,
                    &body,
                    estimate,
                    positions.allocation(),
                    &self.cancelled,
                )?;
                Some(PreparedChange {
                    body,
                    roots,
                    positions,
                    charge,
                })
            }
        } else {
            None
        };
        check(&self.cancelled)?;
        Ok(PreparedBulk {
            base: self.base,
            cancelled: self.cancelled,
            change,
        })
    }
}
fn same_text(body: &Body, cache: &mut LeafCache, at: usize, text: &str) -> bool {
    let range = at..at + text.len();
    // The comparison length comes from another string: its end can land
    // inside a source scalar. That is a mismatch rather than a slicing panic.
    if let Some(s) = cache.small(&body.text, &range) {
        return s == text;
    }
    body.text.get_byte_slice(range).is_some_and(|s| s == text)
}
