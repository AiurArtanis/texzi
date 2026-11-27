use super::*;
use crate::files::{FileError, Format, encode, legacy_aliases::Variant};
use std::sync::atomic::{AtomicBool, Ordering};

fn normalized_len(raw: &str) -> usize {
    raw.len() - raw.as_bytes().windows(2).filter(|s| *s == b"\r\n").count()
}
impl Body {
    /// EOL conversion invalidates raw spans across line breaks. The surviving
    /// noncanonical GB scalars still need their own original spellings.
    pub(super) fn origins_after_eol_change(&self) -> Result<Tree, Error> {
        let mut aliases = crate::bulk::OriginBuilder::new();
        self.visit_aliases(0..self.len_bytes(), |at, variant| {
            aliases.copied(at, at..at + variant.utf8)
        })?;
        Ok(Tree::projected(
            &self.origins,
            aliases.finish()?,
            self.len_bytes(),
        ))
    }
    fn visit_aliases<E>(
        &self,
        range: Range<usize>,
        mut f: impl FnMut(usize, Variant) -> Result<(), E>,
    ) -> Result<(), E> {
        let Some(aliases) = &self.legacy else {
            return Ok(());
        };
        self.origins
            .visit_original(range.clone(), range.start, &mut |output, source, raw| {
                let mut logical = output;
                let mut previous = raw.start;
                aliases.visit(raw, |offset, variant| {
                    logical += normalized_len(&source[previous..offset]);
                    f(logical, variant)?;
                    previous = offset + variant.utf8;
                    logical += variant.utf8;
                    Ok(())
                })
            })
    }
    /// Same logical source and EOL can still denote two different GB byte
    /// spellings. Check both sparse sets exactly, even for reordered captures.
    pub(crate) fn same_alias_ranges(
        &self,
        range: Range<usize>,
        other: &Body,
        second: Range<usize>,
        stop: &AtomicBool,
    ) -> Result<bool, Error> {
        if self.legacy.is_none() && other.legacy.is_none() {
            return Ok(true);
        }
        // Projected text can contain millions of synthetic segments even if
        // only one original alias survived. Check cancellation between bounded
        // slices also when no alias callback runs at all.
        let mut cursor = range.start;
        while cursor < range.end {
            crate::bulk::check(stop)?;
            let end = if range.end - cursor <= 32 * 1024 {
                range.end
            } else {
                self.text
                    .char_to_byte(self.text.byte_to_char(cursor + 32 * 1024))
            };
            let target = second.start + (cursor - range.start);
            if !self.same_alias_chunk(cursor..end, other, target, stop)? {
                return Ok(false);
            }
            cursor = end;
        }
        Ok(true)
    }
    fn same_alias_chunk(
        &self,
        range: Range<usize>,
        other: &Body,
        target_start: usize,
        stop: &AtomicBool,
    ) -> Result<bool, Error> {
        enum Stop {
            Different,
            Cancelled,
        }
        let mut left_count = 0;
        let result = self.visit_aliases(range.clone(), |at, v| {
            if stop.load(Ordering::Relaxed) {
                return Err(Stop::Cancelled);
            }
            let target = target_start + (at - range.start);
            let mut same = false;
            other.visit_aliases(target..target + v.utf8, |pos, found| {
                same = pos == target && found == v;
                Ok(())
            })?;
            if !same {
                return Err(Stop::Different);
            }
            left_count += 1;
            Ok(())
        });
        match result {
            Err(Stop::Cancelled) => return Err(Error::Cancelled),
            Err(Stop::Different) => return Ok(false),
            Ok(()) => {}
        }
        let mut right_count = 0;
        other.visit_aliases(target_start..target_start + range.len(), |_, _| {
            crate::bulk::check(stop)?;
            right_count += 1;
            Ok(())
        })?;
        Ok(left_count == right_count)
    }
    pub(crate) fn encode_aliases(&self, format: Format) -> Result<Vec<u8>, FileError> {
        if format.bom {
            return Err(FileError::UnsupportedBom);
        }
        let mut out = Vec::with_capacity(self.len_bytes());
        let mut cursor = WireCursor {
            chunks: self.text.chunks(),
            rest: "",
            eols: self.eols.chars(),
            at: 0,
        };
        self.visit_aliases(
            0..self.len_bytes(),
            |offset, variant| -> Result<(), FileError> {
                cursor.through(offset, |part| append(&mut out, part, format))?;
                cursor.through(offset + variant.utf8, |_| Ok(()))?;
                out.extend_from_slice(&variant.bytes[..variant.len]);
                Ok(())
            },
        )?;
        cursor.through(self.len_bytes(), |part| append(&mut out, part, format))?;
        Ok(out)
    }
}
fn append(out: &mut Vec<u8>, text: &str, format: Format) -> Result<(), FileError> {
    if text.is_ascii() {
        out.extend_from_slice(text.as_bytes());
    } else {
        out.extend_from_slice(&encode(text, format)?);
    }
    Ok(())
}

struct WireCursor<'a> {
    chunks: ropey::iter::Chunks<'a>,
    rest: &'a str,
    eols: ropey::iter::Chars<'a>,
    at: usize,
}
impl WireCursor<'_> {
    fn through(
        &mut self,
        end: usize,
        mut emit: impl FnMut(&str) -> Result<(), FileError>,
    ) -> Result<(), FileError> {
        while self.at < end {
            while self.rest.is_empty() {
                self.rest = self.chunks.next().ok_or(FileError::InvalidBytes)?;
            }
            let len = (end - self.at).min(self.rest.len());
            let (part, next) = self
                .rest
                .split_at_checked(len)
                .ok_or(FileError::InvalidBytes)?;
            for line in part.split_inclusive('\n') {
                if let Some(text) = line.strip_suffix('\n') {
                    emit(text)?;
                    emit(Eol::from_code(self.eols.next().ok_or(FileError::InvalidBytes)?).raw())?;
                } else {
                    emit(line)?;
                }
            }
            self.rest = next;
            self.at += len;
        }
        Ok(())
    }
}
