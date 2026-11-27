use crate::Error;
use crate::compact_table::{Row, Table, Writer};
use std::ops::Range;

// Output start, source start, copied length, intervening literal length,
// signed source step, repetitions. Captures may be reordered or duplicated.
pub(crate) struct OriginBuilder {
    completed: Writer,
    run: Option<Row>,
    pending: Option<(usize, Range<usize>)>,
}
impl OriginBuilder {
    pub(crate) fn new() -> Self {
        Self {
            completed: Writer::default(),
            run: None,
            pending: None,
        }
    }
    pub(crate) fn copied(&mut self, output: usize, input: Range<usize>) -> Result<(), Error> {
        if let Some((start, old)) = &mut self.pending
            && *start + old.len() == output
            && old.end == input.start
        {
            old.end = input.end;
            return Ok(());
        }
        if let Some(previous) = self.pending.replace((output, input)) {
            self.record(previous)?;
        }
        Ok(())
    }
    fn record(&mut self, (output, input): (usize, Range<usize>)) -> Result<(), Error> {
        if let Some(row) = &mut self.run {
            let gap = output - run_end(*row);
            let src = row[1] as i64 + (row[4] as i32 as i64) * (row[5] - 1) as i64;
            let step = input.start as i64 - src;
            if row[2] as usize == input.len()
                && (row[5] == 1 || (gap == row[3] as usize && step == row[4] as i32 as i64))
            {
                row[3] = gap as u32;
                row[4] = step as i32 as u32;
                row[5] += 1;
                return Ok(());
            }
            self.completed.push(*row)?;
        }
        self.run = Some([
            output as u32,
            input.start as u32,
            input.len() as u32,
            0,
            0,
            1,
        ]);
        Ok(())
    }
    pub(crate) fn finish(mut self) -> Result<OriginMap, Error> {
        if let Some(last) = self.pending.take() {
            self.record(last)?;
        }
        if let Some(row) = self.run {
            self.completed.push(row)?;
        }
        Ok(OriginMap(self.completed.finish()?))
    }
}
fn run_end(r: Row) -> usize {
    (r[0] + r[2] * r[5] + r[3] * (r[5] - 1)) as usize
}
#[derive(Debug)]
pub(crate) struct OriginMap(Table);
impl OriginMap {
    pub(crate) fn allocation(&self) -> usize {
        self.0.allocation()
    }
    pub(crate) fn empty(&self) -> bool {
        self.0.len() == 0
    }
    pub(crate) fn few_copies(&self) -> bool {
        self.0.len() <= 16
            && (0..self.0.len())
                .map(|n| self.0.get(n)[5] as usize)
                .sum::<usize>()
                <= 16
    }
    pub(crate) fn visit(
        &self,
        range: Range<usize>,
        mut segment: impl FnMut(Range<usize>, Option<Range<usize>>),
    ) {
        let Ok(()) = self.try_visit(range, |out, src| {
            Ok::<(), std::convert::Infallible>(segment(out, src))
        });
    }
    pub(crate) fn try_visit<E>(
        &self,
        range: Range<usize>,
        mut segment: impl FnMut(Range<usize>, Option<Range<usize>>) -> Result<(), E>,
    ) -> Result<(), E> {
        let mut cursor = range.start;
        let mut index = self.0.partition(|row| run_end(row) <= cursor);
        while cursor < range.end && index < self.0.len() {
            let [start, source, len, gap, step, count] = self.0.get(index);
            let stride = (len + gap) as usize;
            let first = cursor.saturating_sub(start as usize) / stride;
            for n in first..count as usize {
                let out = start as usize + stride * n;
                if out >= range.end {
                    break;
                }
                let end = (out + len as usize).min(range.end);
                if cursor >= end {
                    continue;
                }
                if cursor < out {
                    segment(cursor..out, None)?;
                    cursor = out;
                }
                let src =
                    (source as i64 + (step as i32 as i64) * n as i64) as usize + (cursor - out);
                segment(cursor..end, Some(src..src + (end - cursor)))?;
                cursor = end;
            }
            if start as usize >= range.end {
                break;
            }
            index += 1;
        }
        if cursor < range.end {
            segment(cursor..range.end, None)?;
        }
        Ok(())
    }
}
