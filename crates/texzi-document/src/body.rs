use crate::{Error, OriginSpan, provenance::Tree};
use ropey::{Rope, RopeBuilder, RopeSlice};
use std::{ops::Range, sync::Arc};
pub(crate) mod compose;
pub(crate) mod history_image;
mod legacy;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eol {
    Lf,
    CrLf,
    Cr,
}
impl Eol {
    fn code(self) -> char {
        match self {
            Self::Lf => 'l',
            Self::CrLf => 'c',
            Self::Cr => 'r',
        }
    }
    pub(crate) fn from_code(c: char) -> Self {
        match c {
            'l' => Self::Lf,
            'c' => Self::CrLf,
            'r' => Self::Cr,
            _ => unreachable!(),
        }
    }
    fn raw(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }
}
/// Body coordinates/provenance are decoded UTF-8; disk codecs are separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    /// Version 2 disk format. Version 1 retains the legacy UTF-8/BOM contract.
    pub file_format: Option<crate::files::Format>,
    pub utf8_bom: bool,
    pub version: u32,
    pub default_eol: Eol,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            file_format: None,
            utf8_bom: false,
            version: 1,
            default_eol: Eol::CrLf,
        }
    }
}
impl Profile {
    pub(crate) fn valid(self) -> bool {
        match (self.version, self.file_format) {
            (1, None) => true,
            (2, Some(f)) => {
                !self.utf8_bom
                    && (!f.bom
                        || matches!(
                            f.encoding,
                            crate::files::Encoding::Utf8
                                | crate::files::Encoding::Utf16Le
                                | crate::files::Encoding::Utf16Be
                        ))
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Body {
    pub(crate) text: Rope,
    // One ASCII marker per line break recognized by this Rope build: LF for
    // a canonical LF, x for a Unicode-only break. This persistent rank index
    // isolates the EOL table from Cargo feature unification of unicode_lines.
    line_breaks: Rope,
    eols: Rope,
    counts: [usize; 3],
    origins: Tree,
    pub(crate) legacy: Option<Arc<crate::files::legacy_aliases::Aliases>>,
    pub profile: Profile,
}
impl Body {
    pub fn import_utf8(raw: &str, mut profile: Profile) -> Self {
        let raw = if let Some(s) = raw.strip_prefix('\u{feff}') {
            profile.utf8_bom = true;
            s
        } else {
            raw
        };
        Self::import_decoded(raw, profile)
    }
    /// Text is already decoded: a leading FEFF is content, never a BOM signature.
    pub fn import_decoded(raw: &str, profile: Profile) -> Self {
        Self::import_cancellable(Arc::from(raw), profile, usize::MAX, &|| false)
            .expect("uninterrupted document import")
    }
    pub(crate) fn import_cancellable(
        raw: Arc<str>,
        profile: Profile,
        max_lines: usize,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, Error> {
        if max_lines == 0 {
            return Err(Error::LineLimit);
        }
        let mut lines = 1;
        let mut text = RopeBuilder::new();
        let mut line_breaks = RopeBuilder::new();
        let mut eols = RopeBuilder::new();
        let mut counts = [0; 3];
        let mut logical = String::new();
        let mut codes = String::new();
        let mut rest: &str = &raw;
        while !rest.is_empty() {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            let mut end = rest.floor_char_boundary(crate::files::IMPORT_CHUNK.min(rest.len()));
            // Keep both halves of a CRLF in the same normalization block.
            if rest.as_bytes().get(end) == Some(&b'\n') && rest.as_bytes()[end - 1] == b'\r' {
                end += 1;
            }
            let (chunk, remaining) = rest.split_at(end);
            logical.clear();
            codes.clear();
            let mut chars = chunk.chars().peekable();
            while let Some(c) = chars.next() {
                let eol = match c {
                    '\r' => Some(if chars.next_if_eq(&'\n').is_some() {
                        Eol::CrLf
                    } else {
                        Eol::Cr
                    }),
                    '\n' => Some(Eol::Lf),
                    _ => None,
                };
                if let Some(eol) = eol {
                    logical.push('\n');
                    codes.push(eol.code());
                    counts[index(eol)] += 1;
                } else {
                    logical.push(c);
                }
            }
            let breaks = break_markers(RopeSlice::from(logical.as_str()));
            lines += breaks.len();
            if lines > max_lines {
                return Err(Error::LineLimit);
            }
            text.append(&logical);
            eols.append(&codes);
            line_breaks.append(&breaks);
            rest = remaining;
        }
        Ok(Self {
            text: text.finish(),
            line_breaks: line_breaks.finish(),
            eols: eols.finish(),
            counts,
            origins: Tree::imported(raw, cancelled)?,
            legacy: None,
            profile,
        })
    }
    pub fn len_bytes(&self) -> usize {
        self.text.len_bytes()
    }
    pub fn len_utf16(&self) -> usize {
        self.text.len_utf16_cu()
    }
    pub fn source(&self) -> &Rope {
        &self.text
    }
    pub fn default_eol(&self) -> Eol {
        self.profile.default_eol
    }
    pub fn logical_line_count(&self) -> usize {
        self.eols.len_chars() + 1
    }
    /// The source bytes of a zero-based LF-delimited line, including its LF if present.
    pub fn logical_line_bytes(&self, line: usize) -> Option<Range<usize>> {
        let count = self.logical_line_count();
        if line >= count {
            return None;
        }
        let start = self.text.line_to_byte(self.line_breaks.line_to_char(line));
        let end = if line + 1 == count {
            self.len_bytes()
        } else {
            self.text
                .line_to_byte(self.line_breaks.line_to_char(line + 1))
        };
        Some(start..end)
    }
    pub fn eols(&self) -> Vec<Eol> {
        self.eols.chars().map(Eol::from_code).collect()
    }
    pub fn origins(&self) -> Vec<(Range<usize>, Option<OriginSpan>)> {
        self.origins.spans()
    }
    /// Explicit materialization for export/test, never used by submit/undo.
    pub fn to_utf8(&self) -> Vec<u8> {
        self.serialized_utf8(self.profile.utf8_bom)
    }
    pub(crate) fn serialized_utf8(&self, bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if bom {
            out.extend_from_slice(b"\xef\xbb\xbf");
        }
        let mut eols = self.eols.chars();
        for chunk in self.text.chunks() {
            for c in chunk.chars() {
                if c == '\n' {
                    out.extend_from_slice(Eol::from_code(eols.next().unwrap()).raw().as_bytes());
                } else {
                    let mut b = [0; 4];
                    out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
                }
            }
        }
        out
    }
    pub fn dominant_eol(&self) -> Eol {
        let max = *self.counts.iter().max().unwrap();
        if max == 0 {
            return self.profile.default_eol;
        }
        if self.counts.iter().filter(|&&n| n == max).count() > 1 {
            Eol::from_code(self.eols.char(0))
        } else {
            [Eol::Lf, Eol::CrLf, Eol::Cr][self.counts.iter().position(|&n| n == max).unwrap()]
        }
    }
    pub(crate) fn scalar(&self, b: usize) -> Result<usize, Error> {
        if b > self.len_bytes() {
            return Err(Error::InvalidBoundary);
        }
        let c = self.text.byte_to_char(b);
        if self.text.char_to_byte(c) != b {
            Err(Error::InvalidBoundary)
        } else {
            Ok(c)
        }
    }
    pub(crate) fn replace(&mut self, range: Range<usize>, text: &str, eols: &[Eol]) {
        let a = self.text.byte_to_char(range.start);
        let b = self.text.byte_to_char(range.end);
        let first_break = self.text.char_to_line(a);
        let last_break = self.text.char_to_line(b);
        let el = self.line_breaks.char_to_line(first_break);
        let er = self.line_breaks.char_to_line(last_break);
        for c in self.eols.slice(el..er).chars() {
            self.counts[index(Eol::from_code(c))] -= 1;
        }
        self.eols.remove(el..er);
        let codes: String = eols.iter().map(|e| e.code()).collect();
        self.eols.insert(el, &codes);
        for &e in eols {
            self.counts[index(e)] += 1;
        }
        // Import and Edit::input normalize CR; submit_inner rejects raw CR,
        // and history inverses retain canonical text. Thus no CR/LF pair can
        // merge across these edit boundaries and invalidate break ordinals.
        let inserted_breaks = break_markers(RopeSlice::from(text));
        self.line_breaks.remove(first_break..last_break);
        self.line_breaks.insert(first_break, &inserted_breaks);
        self.text.remove(a..b);
        self.text.insert(a, text);
        self.origins = self.origins.replace(range, text.len());
    }
    /// The one-based logical position of a validated source byte.
    pub fn line_column(&self, byte: usize) -> Result<(usize, usize), Error> {
        let scalar = self.scalar(byte)?;
        let line = self.line_breaks.char_to_line(self.text.byte_to_line(byte));
        let start = self.text.line_to_char(self.line_breaks.line_to_char(line));
        Ok((line + 1, scalar - start + 1))
    }

    pub(crate) fn eols_in(&self, range: Range<usize>) -> Vec<Eol> {
        let a = self
            .line_breaks
            .char_to_line(self.text.byte_to_line(range.start));
        let b = self
            .line_breaks
            .char_to_line(self.text.byte_to_line(range.end));
        self.eols.slice(a..b).chars().map(Eol::from_code).collect()
    }
    pub(crate) fn eol_codes(&self, range: Range<usize>) -> RopeSlice<'_> {
        let first = self
            .line_breaks
            .char_to_line(self.text.byte_to_line(range.start));
        let last = self
            .line_breaks
            .char_to_line(self.text.byte_to_line(range.end));
        self.eols.slice(first..last)
    }
    pub(crate) fn same_serialization(
        &self,
        other: &Self,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<bool, Error> {
        Ok(self.counts == other.counts
            && compose::equal_ropes(&self.text, &other.text, cancelled)?
            && compose::equal_ropes(&self.eols, &other.eols, cancelled)?
            && self.same_alias_ranges(
                0..self.len_bytes(),
                other,
                0..other.len_bytes(),
                cancelled,
            )?)
    }
    pub(crate) fn change_eols(&mut self, eol: Eol) -> Result<(), Error> {
        let n = self.eols.len_chars();
        self.eols = Rope::from_str(&eol.code().to_string().repeat(n));
        self.counts = [0; 3];
        self.counts[index(eol)] = n;
        self.origins = self.origins_after_eol_change()?;
        Ok(())
    }
}
fn break_markers(text: RopeSlice<'_>) -> String {
    // Rope's own iterator keeps the index correct both with and without
    // unicode_lines. Building it inspects only imported/inserted text; later
    // rank lookups and persistent edits use the ropes' logarithmic indices.
    text.lines()
        .take(text.len_lines() - 1)
        .map(|line| {
            if line.char(line.len_chars() - 1) == '\n' {
                '\n'
            } else {
                'x'
            }
        })
        .collect()
}
fn index(e: Eol) -> usize {
    match e {
        Eol::Lf => 0,
        Eol::CrLf => 1,
        Eol::Cr => 2,
    }
}
