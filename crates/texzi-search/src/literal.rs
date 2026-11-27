mod preview;

use crate::{Options, Rope, Scope, scope::ScopeCursor, unicode::QueryFold, word::WordFilter};
use std::{
    fmt,
    ops::{ControlFlow, Range},
};

const PROGRESS_BYTES: usize = 64 * 1024;
const WORK_CHECK: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "search cancelled")
    }
}

impl std::error::Error for Cancelled {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanEvent {
    Match(Range<usize>),
    /// `read` is progress through the original subject. Restarting at
    /// `resume` reproduces all still-undecided matches, even when a partial
    /// needle spans the progress boundary.
    Progress {
        read: usize,
        resume: usize,
    },
}

#[derive(Debug)]
pub struct Literal(Needle);

#[derive(Debug)]
enum Needle {
    Bytes(Pattern<u8>),
    Scalars {
        pattern: Pattern<char>,
        fold: Option<QueryFold>,
        whole_word: bool,
    },
}

/// Knuth–Morris–Pratt state persists across rope chunks. Restarting after an
/// accepted match yields the same nonoverlapping global iteration as the regex
/// scanner. A rejected whole-word candidate may have an overlapping suffix.
#[derive(Debug)]
struct Pattern<T> {
    keys: Vec<T>,
    prefix: Vec<usize>,
}

impl<T: Eq> Pattern<T> {
    fn new(keys: Vec<T>, cancel: &mut impl FnMut() -> bool) -> Result<Self, Cancelled> {
        let mut prefix = Vec::with_capacity(keys.len());
        prefix.push(0);
        let mut matched = 0;
        for i in 1..keys.len() {
            if i % WORK_CHECK == 0 && cancel() {
                return Err(Cancelled);
            }
            while matched > 0 && keys[matched] != keys[i] {
                let next = prefix[matched - 1];
                if matched / WORK_CHECK != next / WORK_CHECK && cancel() {
                    return Err(Cancelled);
                }
                matched = next;
            }
            if keys[matched] == keys[i] {
                matched += 1;
            }
            prefix.push(matched);
        }
        Ok(Self { keys, prefix })
    }

    fn advance(
        &self,
        matched: &mut usize,
        key: T,
        cancel: &mut impl FnMut() -> bool,
    ) -> Result<bool, Cancelled> {
        while *matched > 0 && self.keys[*matched] != key {
            let next = self.prefix[*matched - 1];
            if *matched / WORK_CHECK != next / WORK_CHECK && cancel() {
                return Err(Cancelled);
            }
            *matched = next;
        }
        if self.keys[*matched] == key {
            *matched += 1;
        }
        Ok(*matched == self.keys.len())
    }
}

impl Literal {
    /// An empty query clears the panel without starting a search.
    pub fn new(query: &str, options: Options) -> Option<Self> {
        Self::try_new(query, options, || false).expect("infallible cancellation check")
    }

    pub fn try_new(
        query: &str,
        options: Options,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Self>, Cancelled> {
        if cancelled() {
            return Err(Cancelled);
        }
        if query.is_empty() {
            return Ok(None);
        }
        let mut chars = query.chars().peekable();
        let mut normalized = std::iter::from_fn(|| {
            let ch = chars.next()?;
            if ch == '\r' {
                chars.next_if_eq(&'\n');
                Some('\n')
            } else {
                Some(ch)
            }
        });
        let needle = if options.case_sensitive && !options.whole_word {
            let mut bytes = Vec::with_capacity(query.len());
            for (index, ch) in normalized.enumerate() {
                if index % WORK_CHECK == 0 && cancelled() {
                    return Err(Cancelled);
                }
                bytes.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
            }
            Needle::Bytes(Pattern::new(bytes, &mut cancelled)?)
        } else {
            let mut keys = Vec::new();
            let mut fold = (!options.case_sensitive).then(QueryFold::default);
            for (index, ch) in normalized.by_ref().enumerate() {
                if index % WORK_CHECK == 0 && cancelled() {
                    return Err(Cancelled);
                }
                keys.push(fold.as_mut().map_or(ch, |table| table.add(ch)));
            }
            Needle::Scalars {
                pattern: Pattern::new(keys, &mut cancelled)?,
                fold,
                whole_word: options.whole_word,
            }
        };
        Ok(Some(Self(needle)))
    }

    /// Use 0 for a new global scan or a verified continuation for a replay.
    /// The callback can stop a completed lookup or return a preparation error
    /// without claiming that the entire document was searched.
    pub fn scan<B>(
        &self,
        rope: &Rope,
        start: usize,
        scope: &Scope,
        cancelled: impl FnMut() -> bool,
        mut visit: impl FnMut(Range<usize>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>, Cancelled> {
        self.scan_events(rope, start, scope, cancelled, |event| match event {
            ScanEvent::Match(range) => visit(range),
            ScanEvent::Progress { .. } => ControlFlow::Continue(()),
        })
    }

    pub fn scan_events<B>(
        &self,
        rope: &Rope,
        start: usize,
        scope: &Scope,
        cancelled: impl FnMut() -> bool,
        visit: impl FnMut(ScanEvent) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>, Cancelled> {
        let mut start = start.min(rope.len_bytes());
        while start < rope.len_bytes() && rope.byte(start) & 0xc0 == 0x80 {
            start += 1;
        }
        let mut scan = Scan {
            scope: scope.cursor(start),
            cancelled,
            visit,
            next_progress: start,
        };
        if let ControlFlow::Break(value) = scan.progress(start, start)? {
            return Ok(ControlFlow::Break(value));
        }
        match &self.0 {
            Needle::Bytes(pattern) => scan_bytes(rope, start, pattern, &mut scan),
            Needle::Scalars {
                pattern,
                fold,
                whole_word,
            } => scan_scalars(rope, start, pattern, fold.as_ref(), *whole_word, &mut scan),
        }
    }
}

struct Scan<'a, C, V> {
    scope: ScopeCursor<'a>,
    cancelled: C,
    visit: V,
    next_progress: usize,
}

impl<B, C: FnMut() -> bool, V: FnMut(ScanEvent) -> ControlFlow<B>> Scan<'_, C, V> {
    fn found(&mut self, hit: Range<usize>) -> ControlFlow<B> {
        if self.scope.contains(&hit) {
            (self.visit)(ScanEvent::Match(hit))
        } else {
            ControlFlow::Continue(())
        }
    }

    fn progress(&mut self, read: usize, resume: usize) -> Result<ControlFlow<B>, Cancelled> {
        if (self.cancelled)() {
            return Err(Cancelled);
        }
        if read < self.next_progress {
            return Ok(ControlFlow::Continue(()));
        }
        self.next_progress = read.saturating_add(PROGRESS_BYTES);
        Ok((self.visit)(ScanEvent::Progress { read, resume }))
    }

    fn finish(&mut self, end: usize) -> Result<ControlFlow<B>, Cancelled> {
        self.next_progress = end;
        self.progress(end, end)
    }
}

fn scan_bytes<B, C: FnMut() -> bool, V: FnMut(ScanEvent) -> ControlFlow<B>>(
    rope: &Rope,
    mut offset: usize,
    pattern: &Pattern<u8>,
    scan: &mut Scan<'_, C, V>,
) -> Result<ControlFlow<B>, Cancelled> {
    let first = pattern.keys[0];
    let mut matched = 0;
    for chunk in rope.byte_slice(offset..).chunks() {
        let bytes = chunk.as_bytes();
        if pattern.keys.len() == 1 {
            for i in memchr::memchr_iter(first, bytes) {
                if let ControlFlow::Break(value) = scan.found(offset + i..offset + i + 1) {
                    return Ok(ControlFlow::Break(value));
                }
            }
        } else {
            let mut i = 0;
            while i < bytes.len() {
                if matched == 0 {
                    let Some(skip) = memchr::memchr(first, &bytes[i..]) else {
                        break;
                    };
                    i += skip;
                }
                let complete = pattern.advance(&mut matched, bytes[i], &mut scan.cancelled)?;
                i += 1;
                if complete {
                    let end = offset + i;
                    matched = 0;
                    if let ControlFlow::Break(value) = scan.found(end - pattern.keys.len()..end) {
                        return Ok(ControlFlow::Break(value));
                    }
                }
            }
        }
        offset += bytes.len();
        if let ControlFlow::Break(value) = scan.progress(offset, offset - matched)? {
            return Ok(ControlFlow::Break(value));
        }
    }
    scan.finish(offset)
}

fn scan_scalars<B, C: FnMut() -> bool, V: FnMut(ScanEvent) -> ControlFlow<B>>(
    rope: &Rope,
    mut offset: usize,
    pattern: &Pattern<char>,
    fold: Option<&QueryFold>,
    whole_word: bool,
    scan: &mut Scan<'_, C, V>,
) -> Result<ControlFlow<B>, Cancelled> {
    let len = pattern.keys.len();
    let mut starts = vec![0usize; len];
    let mut slot = 0;
    let mut matched = 0;
    let mut words = whole_word.then(|| WordFilter::rope(rope));
    let slice = rope.byte_slice(offset..);
    for chunk in slice.chunks() {
        for (within, ch) in chunk.char_indices() {
            let at = offset + within;
            starts[slot] = at;
            slot += 1;
            if slot == len {
                slot = 0;
            }
            let key = fold.map_or(ch, |table| table.key(ch));
            if !pattern.advance(&mut matched, key, &mut scan.cancelled)? {
                continue;
            }
            let range = starts[slot]..at + ch.len_utf8();
            if let Some(words) = &mut words
                && !words.accepts(&range, &mut scan.cancelled)?
            {
                matched = pattern.prefix[len - 1];
                continue;
            }
            matched = 0;
            if let ControlFlow::Break(value) = scan.found(range) {
                return Ok(ControlFlow::Break(value));
            }
        }
        offset += chunk.len();
        let resume = if matched == 0 {
            offset
        } else {
            let first = if slot >= matched {
                slot - matched
            } else {
                len - (matched - slot)
            };
            starts[first]
        };
        if let ControlFlow::Break(value) = scan.progress(offset, resume)? {
            return Ok(ControlFlow::Break(value));
        }
    }
    scan.finish(offset)
}
