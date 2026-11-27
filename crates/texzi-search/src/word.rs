//! Check both ends of a match in the original subject. Each cursor remembers
//! one grapheme, including its word class across combining marks and Rope leaves.
use crate::{Cancelled, Rope};
use std::ops::Range;
use texzi_word::{Class, class_of};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

#[derive(Clone, Copy)]
enum Subject<'a> {
    Rope(&'a Rope),
    Text(&'a str),
}

impl<'a> Subject<'a> {
    fn len(self) -> usize {
        match self {
            Self::Rope(rope) => rope.len_bytes(),
            Self::Text(text) => text.len(),
        }
    }

    fn chunk(self, at: usize) -> (&'a str, usize) {
        match self {
            Self::Rope(rope) => {
                let (s, base, _, _) = rope.chunk_at_byte(at);
                (s, base)
            }
            Self::Text(text) => (text, 0),
        }
    }

    fn context(self, cursor: &mut GraphemeCursor, at: usize) {
        let (s, base) = self.chunk(at - 1);
        cursor.provide_context(&s[..at - base], base);
    }
}

struct Walker<'a> {
    text: Subject<'a>,
    len: usize,
    chunk: &'a str,
    base: usize,
    cursor: GraphemeCursor,
    end: usize,
    before: Option<Class>,
    last_query: usize,
}

impl<'a> Walker<'a> {
    fn new(text: Subject<'a>) -> Self {
        let (chunk, base) = text.chunk(0);
        let len = text.len();
        Self {
            text,
            len,
            chunk,
            base,
            cursor: GraphemeCursor::new(0, len, true),
            end: 0,
            before: None,
            last_query: 0,
        }
    }

    fn at(&mut self, byte: usize) -> Option<char> {
        if byte == self.len {
            return None;
        }
        if byte < self.base || byte >= self.base + self.chunk.len() {
            (self.chunk, self.base) = self.text.chunk(byte);
        }
        self.chunk[byte - self.base..].chars().next()
    }

    // Sparse replay starts near its checkpoint. Only the preceding grapheme
    // is needed, even when its base precedes the first available Rope leaf.
    fn restart(&mut self, byte: usize, cancel: &mut impl FnMut() -> bool) -> Result<(), Cancelled> {
        let mut back = GraphemeCursor::new(byte, self.len, true);
        let (mut chunk, mut base) = self.text.chunk(byte.saturating_sub(1));
        self.end = loop {
            if cancel() {
                return Err(Cancelled);
            }
            match back.prev_boundary(chunk, base) {
                Ok(previous) => break previous.unwrap_or(0),
                Err(GraphemeIncomplete::PrevChunk) => (chunk, base) = self.text.chunk(base - 1),
                Err(GraphemeIncomplete::PreContext(at)) => self.text.context(&mut back, at),
                Err(_) => unreachable!("previous grapheme of a valid UTF-8 offset"),
            }
        };
        self.cursor = GraphemeCursor::new(self.end, self.len, true);
        self.before = None;
        Ok(())
    }

    fn advance(&mut self, cancel: &mut impl FnMut() -> bool) -> Result<(), Cancelled> {
        let ch = self
            .at(self.end)
            .expect("a grapheme before the requested offset");
        self.before = Some(class_of(ch, ""));
        // A pair of ordinary ASCII scalars always breaks here. Keep common
        // dense whole-word searches out of the Unicode segmentation loop.
        let i = self.end - self.base;
        if ch.is_ascii() && ch != '\r' && self.chunk.as_bytes().get(i + 1).is_some_and(u8::is_ascii)
        {
            self.end += 1;
            self.cursor.set_cursor(self.end);
            return Ok(());
        }
        self.end = loop {
            match self.cursor.next_boundary(self.chunk, self.base) {
                Ok(next) => break next.unwrap_or(self.len),
                Err(GraphemeIncomplete::NextChunk) => {
                    (self.chunk, self.base) = self.text.chunk(self.base + self.chunk.len())
                }
                Err(GraphemeIncomplete::PreContext(at)) => self.text.context(&mut self.cursor, at),
                Err(_) => unreachable!("next grapheme of a valid UTF-8 offset"),
            }
            if cancel() {
                return Err(Cancelled);
            }
        };
        Ok(())
    }

    fn edge(
        &mut self,
        byte: usize,
        cancel: &mut impl FnMut() -> bool,
    ) -> Result<Option<(Option<Class>, Option<Class>)>, Cancelled> {
        if byte < self.last_query || byte.saturating_sub(self.end) > 4096 {
            self.restart(byte, cancel)?;
        }
        self.last_query = byte;
        while self.end < byte {
            self.advance(cancel)?;
        }
        if self.end != byte {
            return Ok(None); // inside one displayed grapheme
        }
        let after = self.at(byte).map(|ch| class_of(ch, ""));
        Ok(Some((self.before, after)))
    }
}

fn allows(outside: Option<Class>, inside: Option<Class>) -> bool {
    !outside.is_some_and(Class::is_word)
        || (inside.is_some_and(Class::is_word) && inside != outside)
}

pub(crate) struct WordFilter<'a> {
    start: Walker<'a>,
    end: Walker<'a>,
}

impl<'a> WordFilter<'a> {
    fn new(subject: Subject<'a>) -> Self {
        Self {
            start: Walker::new(subject),
            end: Walker::new(subject),
        }
    }

    pub(crate) fn rope(rope: &'a Rope) -> Self {
        Self::new(Subject::Rope(rope))
    }
    pub(crate) fn text(text: &'a str) -> Self {
        Self::new(Subject::Text(text))
    }

    pub(crate) fn accepts(
        &mut self,
        range: &Range<usize>,
        cancel: &mut impl FnMut() -> bool,
    ) -> Result<bool, Cancelled> {
        let Some((before, first)) = self.start.edge(range.start, cancel)? else {
            return Ok(false);
        };
        if !allows(before, first) {
            return Ok(false);
        }
        let Some((last, after)) = self.end.edge(range.end, cancel)? else {
            return Ok(false);
        };
        Ok(allows(after, last))
    }
}
