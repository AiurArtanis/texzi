use crate::{Error, OriginSpan, provenance::Tree};
use ropey::Rope;
use std::{ops::Range, sync::Arc};

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
    fn from_code(c: char) -> Self {
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
/// This component strictly supports UTF-8 only; other decoders belong to T07.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub utf8_bom: bool,
    pub version: u32,
    pub default_eol: Eol,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            utf8_bom: false,
            version: 1,
            default_eol: Eol::CrLf,
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
        let mut logical = String::with_capacity(raw.len());
        let mut codes = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            let e = match c {
                '\r' => Some(if chars.peek() == Some(&'\n') {
                    chars.next();
                    Eol::CrLf
                } else {
                    Eol::Cr
                }),
                '\n' => Some(Eol::Lf),
                _ => None,
            };
            if let Some(e) = e {
                logical.push('\n');
                codes.push(e.code());
            } else {
                logical.push(c);
            }
        }
        let mut counts = [0; 3];
        for c in codes.chars() {
            counts[index(Eol::from_code(c))] += 1;
        }
        let text = Rope::from_str(&logical);
        let line_breaks = Rope::from_str(&break_markers(&text));
        Self {
            text,
            line_breaks,
            eols: Rope::from_str(&codes),
            counts,
            origins: Tree::imported(Arc::from(raw)),
            profile,
        }
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
    pub fn eols(&self) -> Vec<Eol> {
        self.eols.chars().map(Eol::from_code).collect()
    }
    pub fn origins(&self) -> Vec<(Range<usize>, Option<OriginSpan>)> {
        self.origins.spans()
    }
    /// Explicit materialization for export/test, never used by submit/undo.
    pub fn to_utf8(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.profile.utf8_bom {
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
        let inserted_breaks = break_markers(&Rope::from_str(text));
        self.line_breaks.remove(first_break..last_break);
        self.line_breaks.insert(first_break, &inserted_breaks);
        self.text.remove(a..b);
        self.text.insert(a, text);
        self.origins = self.origins.replace(range, text.len());
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
    pub(crate) fn change_eols(&mut self, eol: Eol) {
        let n = self.eols.len_chars();
        self.eols = Rope::from_str(&eol.code().to_string().repeat(n));
        self.counts = [0; 3];
        self.counts[index(eol)] = n;
        // A changed serialization policy invalidates raw passthrough. Undo holds the old tree.
        self.origins = Tree::default().replace(0..0, self.len_bytes());
    }
}
fn break_markers(text: &Rope) -> String {
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
