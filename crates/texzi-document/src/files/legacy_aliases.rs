//! Retain only original byte sequences that differ from this codec's
//! canonical encoding. Keys are offsets into the unnormalized decoded source,
//! the same domain as OriginSpan::raw (including both bytes of a CRLF).
use super::{Encoding, LoadedBytes, legacy};
use crate::{
    Error,
    compact_table::{Row, Table, Writer},
};
use std::{ops::Range, sync::Arc};

#[derive(Debug)]
pub(crate) struct Aliases {
    pub(crate) encoding: Encoding,
    rows: Table,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Variant {
    pub(crate) utf8: usize,
    pub(crate) len: usize,
    pub(crate) bytes: [u8; 4],
}
// Source start, UTF-8 length, wire length, original bytes, source stride, repetitions.
#[derive(Default)]
struct Builder {
    table: Writer,
    run: Option<Row>,
}
impl Builder {
    fn push(&mut self, at: usize, ch: char, bytes: &[u8]) -> Result<(), Error> {
        let start = u32::try_from(at).map_err(|_| Error::HistoryBudget)?;
        let mut word = [0; 4];
        word[..bytes.len()].copy_from_slice(bytes);
        let value = u32::from_le_bytes(word);
        if let Some(run) = &mut self.run {
            let [first, utf8, len, original, stride, count] = *run;
            let delta = start - (first + stride * (count - 1));
            if utf8 == ch.len_utf8() as u32
                && len == bytes.len() as u32
                && original == value
                && (count == 1 || stride == delta)
            {
                run[4] = delta;
                run[5] += 1;
                return Ok(());
            }
            self.table.push(*run)?;
        }
        self.run = Some([start, ch.len_utf8() as u32, bytes.len() as u32, value, 0, 1]);
        Ok(())
    }
}
fn next_unit<'a>(data: &mut &'a [u8]) -> Result<&'a [u8], Error> {
    let first = *data.first().ok_or(Error::InvalidProfile)?;
    let len = if first <= 0x80 {
        1
    } else if data.get(1).is_some_and(u8::is_ascii_digit) {
        4
    } else {
        2
    };
    let (head, rest) = data.split_at_checked(len).ok_or(Error::InvalidProfile)?;
    *data = rest;
    Ok(head)
}
impl Aliases {
    pub(crate) fn capture(
        loaded: &LoadedBytes,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<Arc<Self>>, Error> {
        let encoding = loaded.format.encoding;
        if !matches!(
            encoding,
            Encoding::GbkWebEncodingRs0844 | Encoding::Gb18030WebEncodingRs0844
        ) {
            return Ok(None);
        }
        let codec = legacy(encoding).map_err(|_| Error::InvalidProfile)?;
        let mut builder = Builder::default();
        let mut text = loaded.text.as_str();
        let mut original = &loaded.original[..];
        let mut at = 0;
        while !text.is_empty() {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            let end = text.floor_char_boundary((32 * 1024).min(text.len()));
            let (part, next) = text.split_at(end);
            let (canonical, _, errors) = codec.encode(part);
            if !errors && original.starts_with(&canonical) {
                original = &original[canonical.len()..];
            } else {
                let mut normal = canonical.as_ref();
                for (i, ch) in part.char_indices() {
                    let raw = next_unit(&mut original)?;
                    let different = if errors {
                        let scalar = &part[i..i + ch.len_utf8()];
                        let (bytes, _, unencodable) = codec.encode(scalar);
                        unencodable || bytes.as_ref() != raw
                    } else {
                        next_unit(&mut normal)? != raw
                    };
                    if different {
                        builder.push(at + i, ch, raw)?;
                    }
                }
            }
            at += end;
            text = next;
        }
        if !original.is_empty() {
            return Err(Error::InvalidProfile);
        }
        let Some(last) = builder.run else {
            return Ok(None);
        };
        builder.table.push(last)?;
        Ok(Some(Arc::new(Self {
            encoding,
            rows: builder.table.finish()?,
        })))
    }
    pub(crate) fn visit<E>(
        &self,
        range: Range<usize>,
        mut f: impl FnMut(usize, Variant) -> Result<(), E>,
    ) -> Result<(), E> {
        let first = self.rows.partition(|[start, _, _, _, step, count]| {
            ((start + step * (count - 1)) as usize) < range.start
        });
        for index in first..self.rows.len() {
            let [start, utf8, len, word, stride, count] = self.rows.get(index);
            if start as usize >= range.end {
                break;
            }
            let offset = range.start.saturating_sub(start as usize);
            let n = if stride == 0 {
                0
            } else {
                offset.div_ceil(stride as usize)
            };
            let variant = Variant {
                utf8: utf8 as usize,
                len: len as usize,
                bytes: word.to_le_bytes(),
            };
            for n in n..count as usize {
                let at = start as usize + stride as usize * n;
                if at >= range.end {
                    break;
                }
                f(at, variant)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_import_has_no_catalog_and_uniform_aliases_use_one_run_across_chunks() {
        let encoding = Encoding::Gb18030WebEncodingRs0844;
        let clean =
            LoadedBytes::decode([0xa2, 0xe3].repeat(30_000).into(), Some(encoding)).unwrap();
        assert!(Aliases::capture(&clean, &|| false).unwrap().is_none());
        let alternate = LoadedBytes::decode(vec![0x80; 30_000].into(), Some(encoding)).unwrap();
        let meta = Aliases::capture(&alternate, &|| false).unwrap().unwrap();
        assert_eq!(meta.rows.len(), 1);
        assert!(meta.rows.allocation() <= 128);
        assert_eq!(meta.rows.disk_bytes(), 0);
        let mut offsets = Vec::new();
        meta.visit(32_766..32_778, |at, variant| {
            assert_eq!(&variant.bytes[..variant.len], &[0x80]);
            offsets.push(at);
            Ok::<(), ()>(())
        })
        .unwrap();
        assert_eq!(offsets, [32_766, 32_769, 32_772, 32_775]);
    }

    #[test]
    fn irregular_legacy_spellings_spill_under_the_fixed_ram_limit_and_allow_random_access() {
        let mut wire = Vec::new();
        let mut offsets = Vec::new();
        let mut decoded = 0;
        for i in 0..24_000 {
            if i % 2 == 0 {
                wire.extend(b"\r\n");
                decoded += 2;
            }
            offsets.push(decoded);
            wire.push(0x80);
            decoded += 3;
        }
        let loaded =
            LoadedBytes::decode(wire.into(), Some(Encoding::Gb18030WebEncodingRs0844)).unwrap();
        let m = Aliases::capture(&loaded, &|| false).unwrap().unwrap();
        assert_eq!(m.rows.allocation(), 0);
        assert!(m.rows.disk_bytes() > 128 * 1024);
        let range = offsets[19_998]..offsets[20_002];
        let mut found = Vec::new();
        m.visit(range, |at, _| {
            found.push(at);
            Ok::<(), ()>(())
        })
        .unwrap();
        assert_eq!(found, offsets[19_998..20_002]);
        assert!(matches!(
            Aliases::capture(&loaded, &|| true),
            Err(Error::Cancelled)
        ));
    }
}
