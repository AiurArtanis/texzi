use crate::compact_table::{Row, Table, Writer};
use crate::{Affinity, Error};
use std::{ops::Range, sync::Arc};

// old start, new start, consumed, produced, unchanged bytes between hits, repetitions.
#[derive(Default)]
pub(super) struct Builder {
    completed: Writer,
    last: Option<Row>,
}
impl Builder {
    pub(super) fn push(
        &mut self,
        range: Range<usize>,
        new: usize,
        inserted: usize,
    ) -> Result<(), Error> {
        let row = [
            range.start as u32,
            new as u32,
            range.len() as u32,
            inserted as u32,
            0,
            1,
        ];
        if let Some(last) = &mut self.last {
            let gap = row[0] - end(*last, false);
            if row[2..4] == last[2..4] && (last[5] == 1 || last[4] == gap) {
                last[4] = gap;
                last[5] += 1;
                return Ok(());
            }
            self.completed.push(*last)?;
        }
        self.last = Some(row);
        Ok(())
    }
    pub(super) fn finish(mut self) -> Result<BulkMap, Error> {
        if let Some(row) = self.last {
            self.completed.push(row)?;
        }
        Ok(BulkMap {
            rows: Arc::new(self.completed.finish()?),
            reverse: false,
        })
    }
}
fn end(row: Row, reverse: bool) -> u32 {
    let i = usize::from(reverse);
    row[i] + row[i + 2] * row[5] + row[4] * (row[5] - 1)
}
/// Directional compact map for a prepared root transition. Each receipt has
/// its own direction, including receipts for Undo and Redo.
#[derive(Clone, Debug)]
pub struct BulkMap {
    rows: Arc<Table>,
    reverse: bool,
}
impl PartialEq for BulkMap {
    fn eq(&self, other: &Self) -> bool {
        self.reverse == other.reverse && Arc::ptr_eq(&self.rows, &other.rows)
    }
}
impl Eq for BulkMap {}
impl BulkMap {
    pub(crate) fn inverse(&self) -> Self {
        Self {
            rows: self.rows.clone(),
            reverse: !self.reverse,
        }
    }
    pub(crate) fn allocation(&self) -> usize {
        self.rows.allocation()
    }
    pub fn spill_bytes(&self) -> usize {
        self.rows.disk_bytes()
    }
    pub fn run_count(&self) -> usize {
        self.rows.len()
    }
    /// Accepts a byte position in the map's input version. Store receipts add
    /// the document, revision and scalar checks before exposing a SourceByte.
    pub fn map_offset(&self, byte: usize, affinity: Affinity) -> usize {
        let at = self.rows.partition(|row| {
            let end = end(row, self.reverse) as usize;
            end < byte || (end == byte && affinity == Affinity::After)
        });
        if at == self.rows.len() {
            return if at == 0 {
                byte
            } else {
                let last = self.rows.get(at - 1);
                end(last, !self.reverse) as usize + (byte - end(last, self.reverse) as usize)
            };
        }
        let row = self.rows.get(at);
        let i = usize::from(self.reverse);
        let (start, output) = (row[i] as usize, row[1 - i] as usize);
        if byte < start {
            return output - (start - byte);
        }
        let (old, new, gap) = (row[2 + i] as usize, row[3 - i] as usize, row[4] as usize);
        let stride = old + gap;
        let relative = byte - start;
        let which = if stride == 0 {
            0
        } else {
            (relative / stride).min(row[5] as usize - 1)
        };
        let inside = relative - which * stride;
        let result = output + which * (new + gap);
        if inside < old || (inside == 0 && old == 0 && affinity == Affinity::Before) {
            result + if affinity == Affinity::After { new } else { 0 }
        } else {
            result + new + (inside - old)
        }
    }
}
