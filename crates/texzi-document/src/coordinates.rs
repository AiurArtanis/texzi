use crate::{Body, DocId, Error, Revision, Snapshot};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};

macro_rules! coordinate {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct $name {
            pub doc: DocId,
            pub revision: Revision,
            pub offset: usize,
        }
        impl $name {
            pub fn at(snapshot: &Snapshot, offset: usize) -> Self {
                Self {
                    doc: snapshot.doc,
                    revision: snapshot.revision,
                    offset,
                }
            }
        }
    };
}
coordinate!(SourceByte);
coordinate!(Utf16Offset);
coordinate!(GraphemeOffset);

#[derive(Clone, Copy, Debug)]
pub enum Coordinate {
    Byte(SourceByte),
    Utf16(Utf16Offset),
    Grapheme(GraphemeOffset),
}

impl Snapshot {
    pub(crate) fn check_position(&self, doc: DocId, revision: Revision) -> Result<(), Error> {
        if doc != self.doc {
            Err(Error::InvalidDoc)
        } else if revision != self.revision {
            Err(Error::StaleRevision)
        } else {
            Ok(())
        }
    }
    pub fn byte_to_utf16(&self, p: SourceByte) -> Result<Utf16Offset, Error> {
        self.check_position(p.doc, p.revision)?;
        let c = self.body.scalar(p.offset)?;
        Ok(Utf16Offset::at(
            self,
            self.body.source().char_to_utf16_cu(c),
        ))
    }
    pub fn utf16_to_byte(&self, p: Utf16Offset) -> Result<SourceByte, Error> {
        self.check_position(p.doc, p.revision)?;
        let r = self.body.source();
        if p.offset > r.len_utf16_cu() {
            return Err(Error::InvalidBoundary);
        }
        let c = r.utf16_cu_to_char(p.offset);
        if r.char_to_utf16_cu(c) != p.offset {
            return Err(Error::InvalidBoundary);
        }
        Ok(SourceByte::at(self, r.char_to_byte(c)))
    }
    /// Local UAX29 check. Handles rope chunk boundaries and unbounded RI/ZWJ context.
    pub fn is_grapheme_boundary(&self, p: SourceByte) -> Result<bool, Error> {
        self.check_position(p.doc, p.revision)?;
        self.body.scalar(p.offset)?;
        boundary(&self.body, p.offset)
    }
}
pub(crate) fn boundary(body: &Body, at: usize) -> Result<bool, Error> {
    let r = body.source();
    let (chunk, start, _, _) = r.chunk_at_byte(at);
    let mut cursor = GraphemeCursor::new(at, r.len_bytes(), true);
    loop {
        match cursor.is_boundary(chunk, start) {
            Ok(b) => return Ok(b),
            Err(GraphemeIncomplete::PreContext(n)) => {
                let (c, s, _, _) = r.chunk_at_byte(n - 1);
                cursor.provide_context(&c[..n - s], s);
            }
            _ => return Err(Error::InvalidBoundary),
        }
    }
}

/// Explicit batch index for global grapheme ordinals. Build once per snapshot,
/// retain it for repeated conversions. Not a per-keystroke/hit-test interface.
/// UI transactions validate only local boundaries, without constructing this index.
pub struct GraphemeIndex {
    doc: DocId,
    revision: Revision,
    bytes: Vec<usize>,
    body: std::sync::Arc<Body>,
}
impl GraphemeIndex {
    pub fn new(s: &Snapshot) -> Self {
        let r = s.body.source();
        let mut bytes = Vec::new();
        let mut base = 0;
        // Canonical LF is a UAX29 control boundary on both sides. A complete
        // logical line therefore has all required context, including arbitrary
        // RI/ZWJ/InCB runs. This explicit batch operation materializes one line,
        // not one complete document per edit. Local validation remains chunked.
        for line in r.lines() {
            let text = line.to_string();
            bytes.extend(text.grapheme_indices(true).map(|(i, _)| base + i));
            base += line.len_bytes();
        }
        bytes.push(r.len_bytes());
        Self {
            doc: s.doc,
            revision: s.revision,
            bytes,
            body: s.body.clone(),
        }
    }
    pub fn len(&self) -> usize {
        self.bytes.len() - 1
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn to_byte(&self, p: GraphemeOffset) -> Result<SourceByte, Error> {
        self.check(p.doc, p.revision)?;
        let offset = *self.bytes.get(p.offset).ok_or(Error::InvalidBoundary)?;
        Ok(SourceByte {
            doc: self.doc,
            revision: self.revision,
            offset,
        })
    }
    pub fn from_byte(&self, p: SourceByte) -> Result<GraphemeOffset, Error> {
        self.check(p.doc, p.revision)?;
        self.body.scalar(p.offset)?;
        let offset = self
            .bytes
            .binary_search(&p.offset)
            .map_err(|_| Error::InsideGrapheme)?;
        Ok(GraphemeOffset {
            doc: self.doc,
            revision: self.revision,
            offset,
        })
    }
    fn check(&self, doc: DocId, r: Revision) -> Result<(), Error> {
        if doc != self.doc {
            Err(Error::InvalidDoc)
        } else if r != self.revision {
            Err(Error::StaleRevision)
        } else {
            Ok(())
        }
    }
}
