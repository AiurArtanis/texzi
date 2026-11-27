use std::{
    io::{self, BufRead, BufWriter, Read, Write},
    ops::{ControlFlow, Range},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{
    RegexError,
    engine::{Match, Pattern},
    template::Parsed,
    wire::{self, Operation, Request},
};
use crate::{Checkpoints, Collector, Options, Scope, word::WordFilter};

static SUBJECT_BYTES_READ: AtomicU64 = AtomicU64::new(0);

/// Advance a zero-width result by exactly one scalar; EOF has no successor.
fn after(text: &str, offset: usize) -> Option<usize> {
    text[offset..]
        .chars()
        .next()
        .map(|ch| offset + ch.len_utf8())
}

fn enumerate(
    pattern: &mut Pattern,
    text: &str,
    options: Options,
    scope: &Scope,
    mut start: usize,
    mut hit: impl FnMut(Range<usize>, Option<usize>, &Match<'_>) -> Result<ControlFlow<()>, RegexError>,
) -> Result<(), RegexError> {
    if !text.is_char_boundary(start) {
        return Err(RegexError::Worker);
    }
    let mut scope = scope.cursor(start);
    let mut words = options.whole_word.then(|| WordFilter::text(text));
    let mut whole = |range: &Range<usize>| {
        words.as_mut().is_none_or(|filter| {
            filter
                .accepts(range, &mut || false)
                .expect("worker cancellation is enforced by its parent")
        })
    };
    if let Some(members) = pattern.ascii_table() {
        let bytes = text.as_bytes();
        for (offset, &byte) in bytes[start..].iter().enumerate() {
            if !members[byte as usize] {
                continue;
            }
            let at = start + offset;
            let end = at + 1;
            let range = at..end;
            if whole(&range)
                && scope.contains(&range)
                && hit(range, Some(end), &Match::single(&[at, end]))?.is_break()
            {
                break;
            }
        }
        return Ok(());
    }
    while let Some(found) = pattern.next(text, start)? {
        let range = found.group(0).ok_or(RegexError::Worker)?;
        if range.start < start || text.get(range.clone()).is_none() {
            return Err(RegexError::Worker);
        }
        let whole = whole(&range);
        // A rejected word boundary must not hide a valid overlapping whole word.
        // A rejected scope, in contrast, must not change the global match sequence.
        let resume = if !whole {
            after(text, range.start)
        } else if range.is_empty() {
            after(text, range.end)
        } else {
            Some(range.end)
        };
        if whole && scope.contains(&range) && hit(range, resume, &found)?.is_break() {
            break;
        }
        match resume {
            Some(next) if next > start => start = next,
            None => break,
            _ => return Err(RegexError::Worker),
        }
    }
    Ok(())
}

struct Query {
    text: String,
    scope: Scope,
    options: Options,
    empty: bool,
    compiled: Pattern,
    index: Option<Checkpoints>,
}

impl Query {
    fn open(
        request: Request,
        input: &mut impl Read,
        output: &mut impl Write,
    ) -> Result<(Self, Operation), RegexError> {
        let Request {
            subject_len,
            pattern,
            options,
            scope,
            operation,
        } = request;
        if subject_len > super::MAX_SUBJECT_BYTES || pattern.len() > super::MAX_EXPRESSION_BYTES {
            return Err(RegexError::Resource);
        }
        let mut compiled = Pattern::compile(&pattern, options.case_sensitive)?;
        // A compile error reaches the caller before it starts writing the subject.
        wire::frame(output, wire::SUBJECT_READY, &[])?;
        output.flush()?;
        let text = private_text(input, subject_len)?;
        if scope.ranges().is_some_and(|ranges| {
            ranges.len() > super::MAX_SCOPES
                || ranges.iter().any(|range| text.get(range.clone()).is_none())
        }) {
            return Err(RegexError::InvalidSubject);
        }
        compiled.prepare_ascii(&text)?;
        Ok((
            Self {
                text,
                scope,
                options,
                empty: pattern.is_empty(),
                compiled,
                index: None,
            },
            operation,
        ))
    }

    fn execute(&mut self, operation: Operation, output: &mut impl Write) -> Result<(), RegexError> {
        let text = self.text.as_str();
        let scope = &self.scope;
        let options = self.options;
        let compiled = &mut self.compiled;
        match operation {
            Operation::Search {
                navigation,
                viewport,
                select_cap,
            } => {
                let mut collector = Collector::new(text.len(), navigation, viewport, select_cap);
                let mut first = true;
                if !self.empty {
                    enumerate(compiled, text, options, scope, 0, |range, resume, _| {
                        if first {
                            let mut pair = Vec::with_capacity(8);
                            wire::push_range(&mut pair, Some(range.clone()));
                            wire::frame(output, wire::FIRST, &pair)?;
                            output.flush()?;
                            first = false;
                        }
                        collector.record(range, resume);
                        Ok(ControlFlow::Continue(()))
                    })?;
                }
                let result = collector.finish();
                wire::frame(output, wire::SUMMARY, &wire::json(&result)?)?;
                self.index = Some(result.index);
            }
            Operation::Navigate(mut replay) => {
                if !replay.done() {
                    enumerate(
                        compiled,
                        text,
                        options,
                        scope,
                        replay.start(),
                        |range, resume, _| {
                            if replay.record(range).is_break() {
                                return Ok(ControlFlow::Break(()));
                            }
                            Ok(resume.map_or(ControlFlow::Break(()), |at| replay.progress(at)))
                        },
                    )?;
                }
                wire::frame(output, wire::SUMMARY, &wire::json(&replay.finish())?)?;
            }
            Operation::Viewport(mut replay) => {
                if !self.empty {
                    enumerate(
                        compiled,
                        text,
                        options,
                        scope,
                        replay.start(),
                        |range, resume, _| {
                            if replay.record(range).is_break() {
                                return Ok(ControlFlow::Break(()));
                            }
                            Ok(resume.map_or(ControlFlow::Break(()), |at| replay.progress(at)))
                        },
                    )?;
                }
                wire::frame(output, wire::SUMMARY, &wire::json(&replay.finish())?)?;
            }
            Operation::Replace { template, only } => {
                let parsed = Parsed::new(&template)?;
                let slots = compiled.resolve(&parsed)?;
                let stride = wire::record_bytes(slots.len())?;
                let mut bytes = Vec::with_capacity(wire::BATCH_BYTES);
                let mut count = 0u64;
                if !self.empty {
                    let start = only
                        .as_ref()
                        .zip(self.index.as_ref())
                        .map_or(0, |(target, index)| index.before_byte(target.start).byte);
                    enumerate(compiled, text, options, scope, start, |range, _, found| {
                        if let Some(target) = &only {
                            if range.start > target.start {
                                return Ok(ControlFlow::Break(()));
                            }
                            if &range != target {
                                return Ok(ControlFlow::Continue(()));
                            }
                        }
                        if bytes.len() + stride > wire::BATCH_BYTES {
                            wire::frame(output, wire::BATCH, &bytes)?;
                            bytes.clear();
                        }
                        wire::push_range(&mut bytes, Some(range));
                        for alternatives in &slots {
                            let capture = alternatives.iter().find_map(|slot| found.group(*slot));
                            if capture
                                .as_ref()
                                .is_some_and(|r| text.get(r.clone()).is_none())
                            {
                                return Err(RegexError::Worker);
                            }
                            wire::push_range(&mut bytes, capture);
                        }
                        count += 1;
                        Ok(if only.is_some() {
                            ControlFlow::Break(())
                        } else {
                            ControlFlow::Continue(())
                        })
                    })?;
                }
                if !bytes.is_empty() {
                    wire::frame(output, wire::BATCH, &bytes)?;
                }
                wire::frame(output, wire::COUNT, &count.to_le_bytes())?;
            }
        }
        Ok(())
    }
}

fn private_text(input: &mut impl Read, len: usize) -> Result<String, RegexError> {
    if len > super::MAX_SUBJECT_BYTES {
        return Err(RegexError::Resource);
    }
    // Even a standalone invocation must provide an exact, valid UTF-8 payload.
    // String::from_utf8 reuses the worker's sole private contiguous allocation.
    let mut owned = Vec::new();
    owned
        .try_reserve_exact(len)
        .map_err(|_| RegexError::Resource)?;
    let mut chunk = [0u8; wire::BATCH_BYTES];
    while owned.len() < len {
        let take = chunk.len().min(len - owned.len());
        input
            .read_exact(&mut chunk[..take])
            .map_err(|_| RegexError::InvalidSubject)?;
        owned.extend_from_slice(&chunk[..take]);
        SUBJECT_BYTES_READ.fetch_add(take as u64, Ordering::Relaxed);
    }
    input
        .read_exact(&mut chunk[..1])
        .map_err(|_| RegexError::InvalidSubject)?;
    if chunk[0] != wire::SUBJECT_END {
        return Err(RegexError::InvalidSubject);
    }
    String::from_utf8(owned).map_err(|_| RegexError::InvalidSubject)
}

fn run(output: &mut impl Write) -> Result<(), RegexError> {
    let mut input = Vec::new();
    let mut reader = io::BufReader::with_capacity(wire::BATCH_BYTES, io::stdin().lock());
    wire::read_blob(&mut reader, &mut input, wire::CONTROL_BYTES)?;
    let (mut query, mut op) = Query::open(wire::parse(&input)?, &mut reader, output)?;
    let mut completed_operations = 0;
    loop {
        query.execute(op, output)?;
        completed_operations += 1;
        let receipt = super::WorkerStats {
            pid: std::process::id(),
            subject_bytes_read: SUBJECT_BYTES_READ.load(Ordering::Relaxed),
            pattern_compilations: super::engine::COMPILATIONS.load(Ordering::Relaxed),
            completed_operations,
        };
        wire::frame(output, wire::STATS, &wire::json(&receipt)?)?;
        output.flush()?;
        if reader.fill_buf()?.is_empty() {
            return Ok(());
        }
        wire::read_blob(&mut reader, &mut input, wire::CONTROL_BYTES)?;
        op = wire::parse(&input)?;
    }
}

pub fn worker_main() -> i32 {
    let mut output = BufWriter::with_capacity(wire::BATCH_BYTES, io::stdout().lock());
    let sent =
        run(&mut output).or_else(|err| wire::frame(&mut output, wire::ERROR, &wire::json(&err)?));
    if sent.is_ok() && output.flush().is_ok() {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_checks_declared_length_and_bytes_before_matching() {
        let mut stream = Vec::new();
        wire::write_subject(&mut stream, &crate::Rope::from_str("a猫\nb")).unwrap();
        let mut input = stream.as_slice();
        assert_eq!(private_text(&mut input, 6).unwrap(), "a猫\nb");
        assert!(input.is_empty());
        for (mut bytes, len) in [
            (stream.as_slice(), 5),
            (stream.as_slice(), 7),
            (&stream[..6], 6),
            (&[0xe4, 0xb8, wire::SUBJECT_END][..], 2),
        ] {
            assert_eq!(
                private_text(&mut bytes, len),
                Err(RegexError::InvalidSubject)
            );
        }
    }
}
