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
/// The decoded String is transferred to the Body's shared provenance. Only
/// GB inputs also need the original wire bytes after that transfer.
pub(crate) struct LegacyBytes {
    encoding: Encoding,
    wire: Arc<[u8]>,
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
impl LegacyBytes {
    pub(crate) fn from_loaded(loaded: &LoadedBytes) -> Option<Self> {
        let encoding = loaded.format.encoding;
        matches!(
            encoding,
            Encoding::GbkWebEncodingRs0844 | Encoding::Gb18030WebEncodingRs0844
        )
        .then(|| Self {
            encoding,
            wire: loaded.original.clone(),
        })
    }
    pub(crate) fn capture(
        &self,
        mut text: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<Arc<Aliases>>, Error> {
        #[cfg(test)]
        tests::CAPTURE_STARTS.set(tests::CAPTURE_STARTS.get() + 1);
        let encoding = self.encoding;
        let codec = legacy(encoding).map_err(|_| Error::InvalidProfile)?;
        let mut builder = Builder::default();
        let mut original = &self.wire[..];
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
        Ok(Some(Arc::new(Aliases {
            encoding,
            rows: builder.table.finish()?,
        })))
    }
}
impl Aliases {
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
    use crate::files::encode_body;
    use crate::{HistoryBudget, Store};
    use std::cell::Cell;

    thread_local! {
        pub(super) static CAPTURE_STARTS: Cell<usize> = const { Cell::new(0) };
    }

    fn capture(
        loaded: &LoadedBytes,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Arc<Aliases>>, Error> {
        LegacyBytes::from_loaded(loaded)
            .unwrap()
            .capture(loaded.text(), &cancelled)
    }

    #[test]
    fn over_line_limit_gb18030_never_starts_collecting_aliases() {
        let encoding = Encoding::Gb18030WebEncodingRs0844;
        // 0x80 is a valid noncanonical euro in this codec. Irregularly spaced
        // originals would spill to disk when this same fixture is admitted.
        let wire: Arc<[u8]> = [0x80, b'\r', b'\n', 0x80].repeat(24_000).into();
        let load = || LoadedBytes::decode(wire.clone(), Some(encoding)).unwrap();
        let before = CAPTURE_STARTS.get();
        let mut store = Store::new(HistoryBudget::default());
        let rejected = store.open_loaded_with_line_limit(load(), 100, || false);
        assert!(matches!(rejected, Err(Error::LineLimit)));
        assert_eq!(CAPTURE_STARTS.get(), before);

        let cancelled =
            store.open_loaded_with_line_limit(load(), 24_001, || CAPTURE_STARTS.get() > before);
        assert!(matches!(cancelled, Err(Error::Cancelled)));
        assert_eq!(CAPTURE_STARTS.get(), before + 1);

        let accepted = store
            .open_loaded_with_line_limit(load(), 24_001, || false)
            .unwrap();
        assert_eq!(CAPTURE_STARTS.get(), before + 2);
        assert!(accepted.body.legacy.as_ref().unwrap().rows.disk_bytes() > 128 * 1024);
        let format = accepted.body.profile.file_format.unwrap();
        assert_eq!(encode_body(&accepted.body, format).unwrap(), wire.as_ref());
    }

    #[test]
    fn canonical_import_has_no_catalog_and_uniform_aliases_use_one_run_across_chunks() {
        let encoding = Encoding::Gb18030WebEncodingRs0844;
        let clean =
            LoadedBytes::decode([0xa2, 0xe3].repeat(30_000).into(), Some(encoding)).unwrap();
        assert!(capture(&clean, || false).unwrap().is_none());
        let alternate = LoadedBytes::decode(vec![0x80; 30_000].into(), Some(encoding)).unwrap();
        let meta = capture(&alternate, || false).unwrap().unwrap();
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
        let m = capture(&loaded, || false).unwrap().unwrap();
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
        assert!(matches!(capture(&loaded, || true), Err(Error::Cancelled)));
    }
}
