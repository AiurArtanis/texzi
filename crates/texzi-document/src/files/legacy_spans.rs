//! Same-format stateless legacy encoding. Body origins remain decoded UTF-8;
//! this save-local map never reinterprets their offsets as encoded-file offsets.
use super::{Encoding, FileBinding, FileError, Format, decode, encode};
use crate::{Body, Eol, Snapshot};
use std::{collections::BTreeMap, ops::Range};
#[cfg(test)]
mod tests;

struct Unit {
    decoded_end: usize,
    encoded: Range<usize>,
}

struct OriginUnit {
    decoded_len: usize,
    encoded: Range<usize>,
}

fn missing() -> FileError {
    FileError::NeedsLegacyByteMap
}

/// The named Web codecs are stateless. Validate each
/// complete encoded unit through the already selected decoder, never a guessed
/// standard table. In particular these are not Gbk936 or GB18030-2022 mappings.
fn units(
    original: &[u8],
    encoding: Encoding,
) -> Result<(String, BTreeMap<usize, Unit>, BTreeMap<usize, usize>), FileError> {
    let mut decoded = String::new();
    let mut units = BTreeMap::new();
    let mut offsets = BTreeMap::from([(0, 0)]);
    let mut at = 0;
    while at < original.len() {
        let width = match encoding {
            Encoding::GbkWebEncodingRs0844 | Encoding::Gb18030WebEncodingRs0844 => {
                if original[at] <= 0x80 {
                    1
                } else if encoding == Encoding::Gb18030WebEncodingRs0844
                    && original
                        .get(at + 1)
                        .is_some_and(|b| (0x30..=0x39).contains(b))
                {
                    4
                } else {
                    2
                }
            }
            _ => return Err(missing()),
        };
        let end = at.checked_add(width).ok_or_else(missing)?;
        let bytes = original.get(at..end).ok_or_else(missing)?;
        let scalar = decode(bytes, encoding)?;
        if scalar.chars().count() != 1 {
            return Err(missing());
        }
        let start = decoded.len();
        decoded.push_str(&scalar);
        units.insert(
            start,
            Unit {
                decoded_end: decoded.len(),
                encoded: at..end,
            },
        );
        offsets.insert(decoded.len(), end);
        at = end;
    }
    Ok((decoded, units, offsets))
}

fn serialized_span(body: &Body, range: Range<usize>) -> Result<String, FileError> {
    if range.start > range.end {
        return Err(missing());
    }
    body.scalar(range.start).map_err(|_| missing())?;
    body.scalar(range.end).map_err(|_| missing())?;
    let eols = body.eols_in(range.clone());
    let mut eols = eols.into_iter();
    let mut text = String::new();
    for scalar in body.source().byte_slice(range).chars() {
        if scalar == '\n' {
            text.push_str(match eols.next().ok_or_else(missing)? {
                Eol::Lf => "\n",
                Eol::CrLf => "\r\n",
                Eol::Cr => "\r",
            });
        } else {
            text.push(scalar);
        }
    }
    if eols.next().is_some() {
        return Err(missing());
    }
    Ok(text)
}

pub(super) fn encode_preserving(
    binding: &FileBinding,
    snapshot: &Snapshot,
    format: Format,
) -> Result<Vec<u8>, FileError> {
    if format != binding.format || format.bom {
        return Err(missing());
    }
    let (decoded, units, offsets) = units(&binding.original, format.encoding)?;
    if decoded.as_bytes() != binding.imported.body.to_utf8() {
        return Err(missing());
    }
    // Both immutable Bodies keep these Arcs alive for the whole call. The data
    // address is used solely for identity, never dereferenced or used as content.
    let mut origins = BTreeMap::<(usize, usize), OriginUnit>::new();
    let mut logical_end = 0;
    let mut decoded_at = 0;
    for (range, origin) in binding.imported.body.origins() {
        if range.start != logical_end {
            return Err(missing());
        }
        let text = serialized_span(&binding.imported.body, range.clone())?;
        let end = decoded_at.checked_add(text.len()).ok_or_else(missing)?;
        if decoded.get(decoded_at..end) != Some(text.as_str()) {
            return Err(missing());
        }
        let encoded_start = *offsets.get(&decoded_at).ok_or_else(missing)?;
        let encoded_end = *offsets.get(&end).ok_or_else(missing)?;
        if let Some(origin) = origin {
            if origin.logical_len != range.len()
                || origin.source.get(origin.raw.clone()) != Some(text.as_str())
            {
                return Err(missing());
            }
            let source = origin.source.as_ptr() as usize;
            let mut at = decoded_at;
            while at < end {
                let unit = units.get(&at).ok_or_else(missing)?;
                if unit.decoded_end > end {
                    return Err(missing());
                }
                let source_at = origin
                    .raw
                    .start
                    .checked_add(at - decoded_at)
                    .ok_or_else(missing)?;
                let key = (source, source_at);
                let entry = OriginUnit {
                    decoded_len: unit.decoded_end - at,
                    encoded: unit.encoded.clone(),
                };
                if let Some(previous) = origins.get(&key) {
                    if previous.decoded_len != entry.decoded_len
                        || binding.original[previous.encoded.clone()]
                            != binding.original[entry.encoded.clone()]
                    {
                        return Err(missing());
                    }
                } else {
                    origins.insert(key, entry);
                }
                at = unit.decoded_end;
            }
        } else if encode(&text, format)?.as_slice() != &binding.original[encoded_start..encoded_end]
        {
            // A previously inserted region has no origin. Only canonical bytes
            // can be reconstructed for it; never guess an alias from its text.
            return Err(missing());
        }
        logical_end = range.end;
        decoded_at = end;
    }
    if logical_end != binding.imported.body.len_bytes() || decoded_at != decoded.len() {
        return Err(missing());
    }
    let mut out = Vec::new();
    logical_end = 0;
    for (range, origin) in snapshot.body.origins() {
        if range.start != logical_end {
            return Err(missing());
        }
        let text = serialized_span(&snapshot.body, range.clone())?;
        if let Some(origin) = origin {
            if origin.logical_len != range.len()
                || origin.source.get(origin.raw.clone()) != Some(text.as_str())
            {
                return Err(missing());
            }
            let source = origin.source.as_ptr() as usize;
            for (at, scalar) in text.char_indices() {
                let unit = origins
                    .get(&(
                        source,
                        origin.raw.start.checked_add(at).ok_or_else(missing)?,
                    ))
                    .ok_or_else(missing)?;
                if unit.decoded_len != scalar.len_utf8() {
                    return Err(missing());
                }
                out.extend_from_slice(&binding.original[unit.encoded.clone()]);
            }
        } else {
            out.extend_from_slice(&encode(&text, format)?);
        }
        logical_end = range.end;
    }
    if logical_end != snapshot.body.len_bytes()
        || decode(&out, format.encoding)?.as_bytes() != snapshot.body.to_utf8()
    {
        return Err(missing());
    }
    Ok(out)
}
