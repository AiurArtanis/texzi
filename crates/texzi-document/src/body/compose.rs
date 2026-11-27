use super::*;
use crate::{
    MAX_EDIT_LINES,
    bulk::{MAX_BODY_BYTES, OriginBuilder, check},
};
use std::sync::atomic::AtomicBool;

const BLOCK: usize = 32 * 1024;

/// Monotone dense searches usually validate and read hundreds of matches
/// within one Rope leaf. Hold only that leaf's bytes; the source stays shared.
#[derive(Default)]
pub(crate) struct LeafCache {
    start: usize,
    text: String,
}
impl LeafCache {
    fn load(&mut self, rope: &Rope, at: usize) {
        if self.text.is_empty() || at < self.start || at >= self.start + self.text.len() {
            let (text, start, _, _) = rope.chunk_at_byte(at);
            self.text.clear();
            self.text.push_str(text);
            self.start = start;
        }
    }
    pub(crate) fn valid(&mut self, body: &Body, range: &Range<usize>) -> Result<(), Error> {
        if range.start > range.end || range.end > body.len_bytes() {
            return Err(Error::InvalidBoundary);
        }
        for at in [range.start, range.end] {
            if at == body.len_bytes() {
                continue;
            }
            self.load(&body.text, at);
            if !self.text.is_char_boundary(at - self.start) {
                return Err(Error::InvalidBoundary);
            }
        }
        Ok(())
    }
    pub(crate) fn small<'a>(&'a mut self, rope: &Rope, range: &Range<usize>) -> Option<&'a str> {
        if range.len() > 2048 {
            return None;
        }
        self.load(rope, range.start);
        self.text
            .get(range.start - self.start..range.end - self.start)
    }
}

pub(crate) fn equal_ropes(a: &Rope, b: &Rope, cancelled: &AtomicBool) -> Result<bool, Error> {
    if a.len_bytes() != b.len_bytes() {
        return Ok(false);
    }
    let mut right = b.chunks();
    let mut rest = &b""[..];
    for chunk in a.chunks() {
        check(cancelled)?;
        let mut left = chunk.as_bytes();
        while !left.is_empty() {
            while rest.is_empty() {
                rest = right.next().unwrap().as_bytes();
            }
            let take = rest.len().min(left.len());
            if rest[..take] != left[..take] {
                return Ok(false);
            }
            left = &left[take..];
            rest = &rest[take..];
        }
    }
    Ok(true)
}

// Split new literal text on scalar / CRLF boundaries. Large reused template
// text is never normalized into one more complete String.
pub(crate) fn literal_chunks(
    raw: &str,
    mut visit: impl FnMut(&str) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut rest = raw;
    while !rest.is_empty() {
        let mut end = rest.floor_char_boundary(BLOCK.min(rest.len()));
        if rest.as_bytes().get(end) == Some(&b'\n') && rest.as_bytes()[end - 1] == b'\r' {
            end += 1;
        }
        let (part, next) = rest.split_at(end);
        if part.contains('\r') {
            visit(&part.replace("\r\n", "\n").replace('\r', "\n"))?;
        } else {
            visit(part)?;
        }
        rest = next;
    }
    Ok(())
}

pub(crate) struct Compose {
    text: Rope,
    breaks: Rope,
    codes: Rope,
    counts: [usize; 3],
    small_text: String,
    small_codes: String,
    origins: OriginBuilder,
    pub(crate) written: usize,
    pub(crate) new_bytes: usize,
    byte_limit: usize,
    line_limit: usize,
}
impl Compose {
    pub(crate) fn new(base: &Body) -> Self {
        Self {
            text: Rope::new(),
            breaks: Rope::new(),
            codes: Rope::new(),
            counts: [0; 3],
            small_text: String::new(),
            small_codes: String::new(),
            origins: OriginBuilder::new(),
            written: 0,
            new_bytes: 0,
            // A supported legacy encoding can decode to more UTF-8 bytes than
            // the input file budget. A pre-existing larger model may shrink.
            byte_limit: MAX_BODY_BYTES.max(base.len_bytes()),
            line_limit: MAX_EDIT_LINES.max(base.text.len_lines()),
        }
    }
    fn reserve(&self, n: usize) -> Result<(), Error> {
        self.written
            .checked_add(n)
            .filter(|n| *n <= self.byte_limit)
            .map(|_| ())
            .ok_or(Error::HistoryBudget)
    }
    fn flush(&mut self) -> Result<(), Error> {
        if self.small_text.is_empty() {
            return Ok(());
        }
        let breaks = break_markers(RopeSlice::from(self.small_text.as_str()));
        self.breaks.append(Rope::from_str(&breaks));
        self.text.append(Rope::from_str(&self.small_text));
        self.codes.append(Rope::from_str(&self.small_codes));
        self.small_text.clear();
        self.small_codes.clear();
        self.check_lines()
    }
    fn check_lines(&self) -> Result<(), Error> {
        if self.text.len_lines() > self.line_limit {
            Err(Error::LineLimit)
        } else {
            Ok(())
        }
    }
    pub(crate) fn literal(&mut self, text: &str, eol: Eol) -> Result<(), Error> {
        self.reserve(text.len())?;
        self.small_text.push_str(text);
        let lines = text.as_bytes().iter().filter(|&&b| b == b'\n').count();
        self.small_codes
            .extend(std::iter::repeat_n(eol.code(), lines));
        self.counts[index(eol)] += lines;
        self.written += text.len();
        self.new_bytes += text.len();
        if self.small_text.len() >= BLOCK {
            self.flush()?;
        }
        Ok(())
    }
    pub(crate) fn copy(
        &mut self,
        base: &Body,
        range: Range<usize>,
        cache: &mut LeafCache,
        cancelled: &AtomicBool,
    ) -> Result<(), Error> {
        if range.is_empty() {
            return Ok(());
        }
        check(cancelled)?;
        self.reserve(range.len())?;
        self.origins.copied(self.written, range.clone())?;
        if range.len() >= BLOCK {
            self.flush()?;
            self.text
                .append(Rope::from(base.text.byte_slice(range.clone())));
            let start = base.text.byte_to_line(range.start);
            let end = base.text.byte_to_line(range.end);
            self.breaks
                .append(Rope::from(base.line_breaks.slice(start..end)));
            let codes = base.eol_codes(range.clone());
            for chunk in codes.chunks() {
                check(cancelled)?;
                for &c in chunk.as_bytes() {
                    self.counts[index(Eol::from_code(c as char))] += 1;
                }
            }
            self.codes.append(Rope::from(codes));
            self.check_lines()?;
        } else {
            let old_len = self.small_text.len();
            if let Some(s) = cache.small(&base.text, &range) {
                self.small_text.push_str(s);
            } else {
                for s in base.text.byte_slice(range.clone()).chunks() {
                    self.small_text.push_str(s);
                }
            }
            if self.small_text[old_len..].contains('\n') {
                for c in base.eol_codes(range.clone()).chars() {
                    self.small_codes.push(c);
                    self.counts[index(Eol::from_code(c))] += 1;
                }
            }
            self.new_bytes += range.len();
            if self.small_text.len() >= BLOCK {
                self.flush()?;
            }
        }
        self.written += range.len();
        Ok(())
    }
    pub(crate) fn finish(mut self, base: &Body) -> Result<(Body, usize), Error> {
        self.flush()?;
        let map = self.origins.finish()?;
        let resident =
            self.new_bytes + self.breaks.len_bytes() + self.codes.len_bytes() + map.allocation();
        Ok((
            Body {
                text: self.text,
                line_breaks: self.breaks,
                eols: self.codes,
                counts: self.counts,
                origins: Tree::projected(&base.origins, map, self.written),
                legacy: base.legacy.clone(),
                profile: base.profile,
            },
            resident,
        ))
    }
}
