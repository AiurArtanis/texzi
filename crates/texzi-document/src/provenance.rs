//! Persistent AVL sequence of origin spans, measured in logical UTF-8 bytes.
//! Original leaves are bounded on import. Edits copy only the tree paths.
use std::{ops::Range, sync::Arc};

#[derive(Clone, Debug)]
pub struct OriginSpan {
    pub source: Arc<str>,
    pub raw: Range<usize>,
    pub logical_len: usize,
}

#[derive(Clone, Debug)]
struct Piece {
    len: usize,
    origin: Option<OriginSpan>,
}
#[derive(Debug)]
enum Kind {
    Leaf(Piece),
    Branch(Tree, Tree),
}
#[derive(Debug)]
struct Node {
    len: usize,
    height: u8,
    kind: Kind,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Tree(Option<Arc<Node>>);

impl Tree {
    fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |n| n.len)
    }
    fn height(&self) -> u8 {
        self.0.as_ref().map_or(0, |n| n.height)
    }
    fn leaf(piece: Piece) -> Self {
        if piece.len == 0 {
            return Self::default();
        }
        Self(Some(Arc::new(Node {
            len: piece.len,
            height: 1,
            kind: Kind::Leaf(piece),
        })))
    }
    fn branch(a: Self, b: Self) -> Self {
        if a.len() == 0 {
            return b;
        }
        if b.len() == 0 {
            return a;
        }
        Self(Some(Arc::new(Node {
            len: a.len() + b.len(),
            height: 1 + a.height().max(b.height()),
            kind: Kind::Branch(a, b),
        })))
    }
    fn children(&self) -> (Self, Self) {
        match &self.0.as_ref().unwrap().kind {
            Kind::Branch(a, b) => (a.clone(), b.clone()),
            _ => unreachable!(),
        }
    }
    fn balance(a: Self, b: Self) -> Self {
        if a.height() > b.height() + 1 {
            let (al, ar) = a.children();
            if al.height() >= ar.height() {
                Self::branch(al, Self::branch(ar, b))
            } else {
                let (arl, arr) = ar.children();
                Self::branch(Self::branch(al, arl), Self::branch(arr, b))
            }
        } else if b.height() > a.height() + 1 {
            let (bl, br) = b.children();
            if br.height() >= bl.height() {
                Self::branch(Self::branch(a, bl), br)
            } else {
                let (bll, blr) = bl.children();
                Self::branch(Self::branch(a, bll), Self::branch(blr, br))
            }
        } else {
            Self::branch(a, b)
        }
    }
    fn join(a: Self, b: Self) -> Self {
        if a.height() > b.height() + 1 {
            let (al, ar) = a.children();
            Self::balance(al, Self::join(ar, b))
        } else if b.height() > a.height() + 1 {
            let (bl, br) = b.children();
            Self::balance(Self::join(a, bl), br)
        } else {
            Self::branch(a, b)
        }
    }
    fn split(&self, at: usize) -> (Self, Self) {
        if at == 0 {
            return (Self::default(), self.clone());
        }
        if at == self.len() {
            return (self.clone(), Self::default());
        }
        match &self.0.as_ref().unwrap().kind {
            Kind::Branch(a, b) => {
                if at < a.len() {
                    let (l, r) = a.split(at);
                    (l, Self::join(r, b.clone()))
                } else {
                    let (l, r) = b.split(at - a.len());
                    (Self::join(a.clone(), l), r)
                }
            }
            Kind::Leaf(p) => {
                let (o1, o2) = if let Some(o) = &p.origin {
                    let raw = &o.source[o.raw.clone()];
                    let mut logical = 0;
                    let mut end = 0;
                    let mut chars = raw.char_indices().peekable();
                    while logical < at {
                        let (i, c) = chars.next().unwrap();
                        end = i + c.len_utf8();
                        if c == '\r' {
                            if chars.peek().is_some_and(|(_, c)| *c == '\n') {
                                let (i, _) = chars.next().unwrap();
                                end = i + 1;
                            }
                            logical += 1;
                        } else {
                            logical += c.len_utf8();
                        }
                    }
                    assert_eq!(logical, at, "caller must split at a scalar boundary");
                    (
                        Some(OriginSpan {
                            source: o.source.clone(),
                            raw: o.raw.start..o.raw.start + end,
                            logical_len: at,
                        }),
                        Some(OriginSpan {
                            source: o.source.clone(),
                            raw: o.raw.start + end..o.raw.end,
                            logical_len: p.len - at,
                        }),
                    )
                } else {
                    (None, None)
                };
                (
                    Self::leaf(Piece {
                        len: at,
                        origin: o1,
                    }),
                    Self::leaf(Piece {
                        len: p.len - at,
                        origin: o2,
                    }),
                )
            }
        }
    }
    pub(crate) fn imported(
        source: Arc<str>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, crate::Error> {
        if cancelled() {
            return Err(crate::Error::Cancelled);
        }
        let mut tree = Self::default();
        let mut start = 0;
        let mut len = 0;
        let mut chars = source.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            let mut end = i + c.len_utf8();
            if c == '\r' {
                if chars.peek().is_some_and(|(_, c)| *c == '\n') {
                    let (i, _) = chars.next().unwrap();
                    end = i + 1;
                }
                len += 1;
            } else {
                len += c.len_utf8();
            }
            if end - start >= 1024 || end == source.len() {
                if cancelled() {
                    return Err(crate::Error::Cancelled);
                }
                tree = Self::join(
                    tree,
                    Self::leaf(Piece {
                        len,
                        origin: Some(OriginSpan {
                            source: source.clone(),
                            raw: start..end,
                            logical_len: len,
                        }),
                    }),
                );
                start = end;
                len = 0;
            }
        }
        Ok(tree)
    }
    pub(crate) fn replace(&self, range: Range<usize>, inserted: usize) -> Self {
        let (left, tail) = self.split(range.start);
        let (_, right) = tail.split(range.end - range.start);
        Self::join(
            Self::join(
                left,
                Self::leaf(Piece {
                    len: inserted,
                    origin: None,
                }),
            ),
            right,
        )
    }
    pub(crate) fn spans(&self) -> Vec<(Range<usize>, Option<OriginSpan>)> {
        fn walk(t: &Tree, start: usize, out: &mut Vec<(Range<usize>, Option<OriginSpan>)>) {
            if let Some(n) = &t.0 {
                match &n.kind {
                    Kind::Leaf(p) => out.push((start..start + p.len, p.origin.clone())),
                    Kind::Branch(a, b) => {
                        walk(a, start, out);
                        walk(b, start + a.len(), out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(self, 0, &mut out);
        out
    }
}
