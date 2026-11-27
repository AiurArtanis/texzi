use std::{
    io::{self, BufWriter, Read, Write},
    ops::Range,
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::RegexError;
use crate::{Navigation, NavigationReplay, Options, Rope, Scope, VisibleReplay};

// Only numbered source offsets travel per match. Patterns and templates are sent down
// stdin, never through command lines, diagnostics, or an externally named file.
pub const BATCH_BYTES: usize = 64 * 1024;
pub const CONTROL_BYTES: usize = 8 * 1024 * 1024;
pub const UNSET: u32 = u32::MAX;
pub const FIRST: u8 = 1;
pub const SUMMARY: u8 = 2;
pub const BATCH: u8 = 3;
pub const COUNT: u8 = 4;
pub const ERROR: u8 = 5;
pub const STATS: u8 = 6;
pub const SUBJECT_READY: u8 = 7;
// A UTF-8 subject cannot itself contain this byte. It separates the initial
// length-delimited byte stream from later control requests on the same pipe.
pub const SUBJECT_END: u8 = 0xff;

#[derive(Serialize, Deserialize)]
pub struct Request {
    pub subject_len: usize,
    pub pattern: String,
    pub options: Options,
    pub scope: Scope,
    pub operation: Operation,
}

#[derive(Serialize, Deserialize)]
pub enum Operation {
    Search {
        navigation: Navigation,
        viewport: Range<usize>,
        select_cap: Option<usize>,
    },
    Replace {
        template: String,
        only: Option<Range<usize>>,
    },
    Navigate(NavigationReplay),
    Viewport(VisibleReplay),
}

pub fn record_bytes(slots: usize) -> Result<usize, RegexError> {
    slots
        .checked_add(1)
        .and_then(|n| n.checked_mul(8))
        .ok_or(RegexError::Resource)
}

pub fn write_blob(writer: &mut impl Write, data: &[u8]) -> Result<(), RegexError> {
    if data.len() > CONTROL_BYTES {
        return Err(RegexError::Resource);
    }
    writer.write_all(&(data.len() as u32).to_le_bytes())?;
    writer.write_all(data)?;
    Ok(())
}

pub fn write_subject(writer: &mut impl Write, rope: &Rope) -> io::Result<()> {
    let mut out = BufWriter::with_capacity(BATCH_BYTES, writer);
    let result = (|| {
        for chunk in rope.chunks() {
            for bytes in chunk.as_bytes().chunks(BATCH_BYTES) {
                out.write_all(bytes)?;
            }
        }
        out.write_all(&[SUBJECT_END])?;
        out.flush()
    })();
    // Discard unwritten bytes after a closed pipe. BufWriter's Drop would
    // otherwise try to send the failed block again.
    let _ = out.into_parts();
    result
}

pub fn read_blob(
    reader: &mut impl Read,
    buf: &mut Vec<u8>,
    limit: usize,
) -> Result<(), RegexError> {
    let mut length = [0; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > limit {
        return Err(RegexError::Resource);
    }
    buf.clear();
    buf.try_reserve(length).map_err(|_| RegexError::Resource)?;
    buf.resize(length, 0);
    reader.read_exact(buf)?;
    Ok(())
}

pub fn json<T: Serialize>(value: &T) -> Result<Vec<u8>, RegexError> {
    serde_json::to_vec(value).map_err(|_| RegexError::Worker)
}

pub fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, RegexError> {
    serde_json::from_slice(bytes).map_err(|_| RegexError::Worker)
}

pub fn frame(writer: &mut impl Write, tag: u8, bytes: &[u8]) -> Result<(), RegexError> {
    writer.write_all(&[tag])?;
    write_blob(writer, bytes)
}

pub fn next_frame(reader: &mut impl Read, buf: &mut Vec<u8>) -> Result<u8, RegexError> {
    let mut tag = [0];
    reader.read_exact(&mut tag)?;
    let limit = match tag[0] {
        SUBJECT_READY => 0,
        FIRST | COUNT => 8,
        ERROR => 1024,
        BATCH => BATCH_BYTES,
        STATS => 256,
        SUMMARY => CONTROL_BYTES,
        _ => return Err(RegexError::Worker),
    };
    read_blob(reader, buf, limit)?;
    if tag[0] == ERROR {
        return Err(parse(buf)?);
    }
    Ok(tag[0])
}

pub fn push_range(out: &mut Vec<u8>, range: Option<Range<usize>>) {
    // Subject staging is capped below u32::MAX. Capture groups can be unset.
    let (a, b) = range.map_or((UNSET, UNSET), |r| (r.start as u32, r.end as u32));
    out.extend_from_slice(&a.to_le_bytes());
    out.extend_from_slice(&b.to_le_bytes());
}

pub fn get_range(pair: &[u8], len: usize) -> Result<Option<Range<usize>>, RegexError> {
    let pair: &[u8; 8] = pair.try_into().map_err(|_| RegexError::Worker)?;
    let a = u32::from_le_bytes(pair[..4].try_into().unwrap());
    let b = u32::from_le_bytes(pair[4..].try_into().unwrap());
    if a == UNSET && b == UNSET {
        return Ok(None);
    }
    if a > b || b as usize > len {
        return Err(RegexError::Worker);
    }
    Ok(Some(a as usize..b as usize))
}
