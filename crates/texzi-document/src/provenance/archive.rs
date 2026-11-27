//! Disk representation of provenance topology. A shared source is written
//! once, even when many leaves or both history roots refer to it.
use super::*;
use crate::{
    Error,
    bulk::{OriginMap, check},
    files::history_file::{Backing, Temporary},
};
use std::{
    collections::HashMap,
    sync::{Weak, atomic::AtomicBool},
};

struct SavedSource {
    original: Weak<str>,
    bytes: Backing,
}
#[derive(Default)]
pub(crate) struct Sources {
    known: HashMap<usize, Arc<SavedSource>>,
}
impl Sources {
    fn save(
        &mut self,
        original: &Arc<str>,
        cancelled: &AtomicBool,
    ) -> Result<Arc<SavedSource>, Error> {
        let key = original.as_ptr() as usize;
        if let Some(found) = self.known.get(&key) {
            return Ok(found.clone());
        }
        let mut temp = Temporary::new()?;
        for chunk in original.as_bytes().chunks(64 * 1024) {
            check(cancelled)?;
            temp.append(chunk)?;
        }
        let source = Arc::new(SavedSource {
            original: Arc::downgrade(original),
            bytes: temp.seal()?,
        });
        self.known.insert(key, source.clone());
        Ok(source)
    }
}
#[derive(Default)]
pub(crate) struct Topology {
    sources: Vec<Arc<SavedSource>>,
    projections: Vec<Arc<OriginMap>>,
}
struct Encoder<'a> {
    output: &'a mut Temporary,
    sources: &'a mut Sources,
    ids: HashMap<usize, u32>,
    table: Topology,
    cancelled: &'a AtomicBool,
}
impl Encoder<'_> {
    fn emit(&mut self, tag: u8, fields: &[usize]) -> Result<(), Error> {
        let mut buf = [0; 33];
        buf[0] = tag;
        for (word, data) in fields.iter().zip(buf[1..].chunks_exact_mut(8)) {
            data.copy_from_slice(&(*word as u64).to_le_bytes());
        }
        self.output.append(&buf[..1 + 8 * fields.len()])
    }
    fn node(&mut self, tree: &Tree) -> Result<(), Error> {
        check(self.cancelled)?;
        let Some(node) = &tree.0 else {
            return self.emit(0, &[]);
        };
        match &node.kind {
            Kind::Leaf(Piece { len, origin: None }) => self.emit(1, &[*len]),
            Kind::Leaf(Piece {
                len,
                origin: Some(span),
            }) => {
                let key = span.source.as_ptr() as usize;
                let id = if let Some(id) = self.ids.get(&key) {
                    *id
                } else {
                    let id = self.table.sources.len() as u32;
                    self.table
                        .sources
                        .push(self.sources.save(&span.source, self.cancelled)?);
                    self.ids.insert(key, id);
                    id
                };
                self.emit(2, &[*len, id as usize, span.raw.start, span.raw.end])
            }
            Kind::Branch(left, right) => {
                self.emit(3, &[])?;
                self.node(left)?;
                self.node(right)
            }
            Kind::Projected { base, map, offset } => {
                let id = self.table.projections.len();
                self.table.projections.push(map.clone());
                self.emit(4, &[node.len, *offset, id])?;
                self.node(base)
            }
        }
    }
}
impl Topology {
    pub(crate) fn write(
        tree: &Tree,
        output: &mut Temporary,
        sources: &mut Sources,
        cancelled: &AtomicBool,
    ) -> Result<Self, Error> {
        let mut encoder = Encoder {
            output,
            sources,
            ids: HashMap::new(),
            table: Self::default(),
            cancelled,
        };
        encoder.node(tree)?;
        Ok(encoder.table)
    }
    pub(crate) fn allocation(&self) -> usize {
        self.projections
            .iter()
            .map(|v| v.allocation() + 64)
            .sum::<usize>()
            + self.sources.capacity() * 128
    }
    pub(crate) fn read(&self, bytes: &[u8]) -> Result<Tree, Error> {
        struct Reader<'a> {
            bytes: &'a [u8],
            table: &'a Topology,
            sources: Vec<Option<Arc<str>>>,
        }
        impl Reader<'_> {
            fn word(&mut self) -> Result<usize, Error> {
                let (word, rest) = self.bytes.split_at_checked(8).ok_or(Error::HistoryBudget)?;
                self.bytes = rest;
                usize::try_from(u64::from_le_bytes(word.try_into().unwrap()))
                    .map_err(|_| Error::HistoryBudget)
            }
            fn node(&mut self) -> Result<Tree, Error> {
                let (&tag, rest) = self.bytes.split_first().ok_or(Error::HistoryBudget)?;
                self.bytes = rest;
                Ok(match tag {
                    0 => Tree::default(),
                    1 => Tree::leaf(Piece {
                        len: self.word()?,
                        origin: None,
                    }),
                    2 => {
                        let len = self.word()?;
                        let id = self.word()?;
                        let raw = self.word()?..self.word()?;
                        let src = self.table.sources.get(id).ok_or(Error::HistoryBudget)?;
                        let data = &mut self.sources[id];
                        if data.is_none() {
                            *data = Some(match src.original.upgrade() {
                                Some(original) => original,
                                None => Arc::from(
                                    std::str::from_utf8(&src.bytes)
                                        .map_err(|_| Error::HistoryBudget)?,
                                ),
                            });
                        }
                        Tree::leaf(Piece {
                            len,
                            origin: Some(OriginSpan {
                                source: data.as_ref().unwrap().clone(),
                                raw,
                                logical_len: len,
                            }),
                        })
                    }
                    3 => Tree::branch(self.node()?, self.node()?),
                    4 => {
                        let len = self.word()?;
                        let offset = self.word()?;
                        let id = self.word()?;
                        let map = self
                            .table
                            .projections
                            .get(id)
                            .ok_or(Error::HistoryBudget)?
                            .clone();
                        Tree::window(self.node()?, map, offset, len)
                    }
                    _ => return Err(Error::HistoryBudget),
                })
            }
        }
        let mut reader = Reader {
            bytes,
            table: self,
            sources: vec![None; self.sources.len()],
        };
        let tree = reader.node()?;
        if !reader.bytes.is_empty() {
            return Err(Error::HistoryBudget);
        }
        Ok(tree)
    }
}
