//! Random-access, read-only text windows. No line index or full-file allocation.
use super::{Encoding, decode};
use same_file::Handle;
use std::{
    collections::BTreeSet,
    fs::{File, Metadata},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};

pub const PAGE_BYTES: usize = 16 * 1024;
const SYNC_BYTES: usize = 4096;
const SAMPLE_BYTES: usize = 4096;
pub const SCAN_BYTES: usize = 256 * 1024;

pub enum Calibration {
    More(u64),
    Ready(Page),
}

#[derive(Debug, PartialEq)]
pub struct Page {
    pub text: String,
    /// Absolute byte positions; end is exclusive and is also a valid next-page start.
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Position {
    /// From this reader's BOM or a previously verified page end.
    Boundary(u64),
    /// A caller-supplied byte location. Decode only after finding character alignment.
    Near(u64),
    /// The page immediately preceding a known byte boundary (including EOF).
    Before(u64),
}

pub struct FileSource {
    file: Mutex<File>,
    identity: Handle,
    pub len: u64,
    modified: SystemTime,
}

pub struct PagedFile {
    pub source: Arc<FileSource>,
    pub len: u64,
    pub first: u64,
    pub encoding: Encoding,
    anchors: Mutex<BTreeSet<u64>>,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "无法按当前编码解码此页")
}

impl FileSource {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::other("只能分页查看普通文件"));
        }
        Ok(Self {
            identity: Handle::from_file(file.try_clone()?)?,
            file: Mutex::new(file),
            len: metadata.len(),
            modified: metadata.modified()?,
        })
    }

    pub fn verify_path(&self, path: &Path) -> io::Result<()> {
        if Handle::from_path(path)? != self.identity {
            return Err(io::Error::other("该路径已指向另一个文件，请重新打开"));
        }
        self.check(path.metadata()?)
    }

    fn check(&self, metadata: Metadata) -> io::Result<()> {
        if metadata.len() == self.len && metadata.modified()? == self.modified {
            Ok(())
        } else {
            Err(io::Error::other("文件已更改，请关闭后重新打开"))
        }
    }

    fn lock(&self) -> io::Result<std::sync::MutexGuard<'_, File>> {
        self.file.lock().map_err(|_| io::Error::other("读取已中断"))
    }

    pub fn complete_bytes(&self, limit: u64, cancel: &AtomicBool) -> io::Result<Vec<u8>> {
        if self.len > limit {
            return Err(io::Error::other("文件超出此次完整编辑预算，请选择分页查看"));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(usize::try_from(self.len).map_err(io::Error::other)?)
            .map_err(io::Error::other)?;
        self.check(self.lock()?.metadata()?)?;
        let mut chunk = [0; 64 * 1024];
        while (bytes.len() as u64) < self.len {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::other("已取消加载"));
            }
            let count = (self.len - bytes.len() as u64).min(chunk.len() as u64) as usize;
            // A duplicated Windows handle shares the seek pointer. Keep seek+read
            // atomic but let previews and independent page requests run between chunks.
            {
                let mut file = self.lock()?;
                file.seek(SeekFrom::Start(bytes.len() as u64))?;
                file.read_exact(&mut chunk[..count])?;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        self.check(self.lock()?.metadata()?)?;
        Ok(bytes)
    }
}

impl PagedFile {
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::from_source(Arc::new(FileSource::open(path)?), None)
    }

    /// Encoding changes reuse the same open file, including after a path replacement.
    pub fn from_source(source: Arc<FileSource>, choice: Option<Encoding>) -> io::Result<Self> {
        let len = source.len;
        let mut sample = vec![0; len.min(SAMPLE_BYTES as u64) as usize];
        let mut file = source.lock()?;
        source.check(file.metadata()?)?;
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut sample)?;
        drop(file);
        let (encoding, first) =
            if sample.starts_with(&[0xff, 0xfe, 0, 0]) || sample.starts_with(&[0, 0, 0xfe, 0xff]) {
                return Err(io::Error::other("分页查看暂不支持 UTF-32"));
            } else if sample.starts_with(&[0xef, 0xbb, 0xbf]) {
                (Encoding::Utf8, 3)
            } else if sample.starts_with(&[0xff, 0xfe]) {
                (Encoding::Utf16Le, 2)
            } else if sample.starts_with(&[0xfe, 0xff]) {
                (Encoding::Utf16Be, 2)
            } else if let Some(
                choice @ (Encoding::Utf8
                | Encoding::GbkWebEncodingRs0844
                | Encoding::Gb18030WebEncodingRs0844
                | Encoding::Utf16Le
                | Encoding::Utf16Be),
            ) = choice
            {
                (choice, 0)
            } else if !std::str::from_utf8(&sample).is_err_and(|err| err.error_len().is_some()) {
                (Encoding::Utf8, 0)
            } else {
                (Encoding::Gb18030WebEncodingRs0844, 0)
            };
        Ok(Self {
            source,
            len,
            first,
            encoding,
            anchors: Mutex::new(BTreeSet::from([first])),
        })
    }

    pub fn read(&self, position: Position) -> io::Result<Page> {
        self.read_aligned(position, None)
    }

    fn read_aligned(&self, position: Position, exact_start: Option<u64>) -> io::Result<Page> {
        let until = match position {
            Position::Before(n) => Some(n.clamp(self.first, self.len)),
            _ => None,
        };
        let offset = match position {
            Position::Boundary(n) | Position::Near(n) => n.clamp(self.first, self.len),
            Position::Before(_) => until
                .unwrap()
                .saturating_sub(PAGE_BYTES as u64)
                .max(self.first),
        };
        let mut file = self.source.lock()?;
        self.source.check(file.metadata()?)?;
        let guard = match (position, self.encoding) {
            (Position::Boundary(_), _) => 0,
            (_, encoding) if gb_encoding(encoding) => SYNC_BYTES,
            _ => 4,
        };
        let base = offset.saturating_sub(guard as u64).max(self.first);
        file.seek(SeekFrom::Start(base))?;
        let count = (self.len - base).min(offset - base + PAGE_BYTES as u64 + 8) as usize;
        let mut bytes = vec![0; count];
        file.read_exact(&mut bytes)?;
        self.source.check(file.metadata()?)?;
        drop(file);
        let index = (offset - base) as usize;
        let start = if let Some(at) = exact_start {
            (at - base) as usize
        } else if !matches!(position, Position::Boundary(_)) && offset < self.len {
            let known_end =
                until.or_else(|| (base + bytes.len() as u64 == self.len).then_some(self.len));
            self.align(&bytes, base, index, known_end)?
        } else {
            index
        };
        let window = &bytes[start..until.map_or(bytes.len(), |end| (end - base) as usize)];
        let limit = window.len().min(PAGE_BYTES);
        // Stop at the last complete scalar. The next page starts at this exact position.
        let end = match self.encoding {
            Encoding::Utf8 if limit < window.len() => {
                let mut end = limit;
                while end > 0 && window[end] & 0xc0 == 0x80 && limit - end < 3 {
                    end -= 1;
                }
                end
            }
            Encoding::Utf16Le | Encoding::Utf16Be if limit < window.len() => {
                let end = limit & !1;
                if (0xd800..=0xdbff).contains(&unit(&window[end - 2..], self.encoding)) {
                    end - 2
                } else {
                    end
                }
            }
            encoding if gb_encoding(encoding) => {
                let mut end = 0;
                while end < limit {
                    let n = gb_width(&window[end..], encoding)?;
                    if end + n > limit {
                        break;
                    }
                    end += n;
                }
                end
            }
            _ => limit,
        };
        let text = decode(&window[..end], self.encoding).map_err(|_| invalid())?;
        self.remember(base + start as u64);
        self.remember(base + (start + end) as u64);
        Ok(Page {
            text,
            start: base + start as u64,
            end: base + (start + end) as u64,
        })
    }

    fn align(
        &self,
        bytes: &[u8],
        base: u64,
        index: usize,
        known_end: Option<u64>,
    ) -> io::Result<usize> {
        match self.encoding {
            Encoding::Utf8 => {
                let mut at = index;
                while at < bytes.len() && bytes[at] & 0xc0 == 0x80 {
                    at += 1;
                    if at - index > 3 {
                        return Err(invalid());
                    }
                }
                Ok(at)
            }
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let mut at = index + ((base + index as u64 - self.first) % 2) as usize;
                if bytes.len().saturating_sub(at) >= 2
                    && (0xdc00..=0xdfff).contains(&unit(&bytes[at..], self.encoding))
                {
                    at += 2;
                }
                Ok(at.min(bytes.len()))
            }
            encoding if gb_encoding(encoding) => {
                // These ASCII byte values cannot be a trail or digit in a GB18030 scalar.
                let anchor = self.nearest(base + index as u64);
                let ascii = bytes[..=index]
                    .iter()
                    .rposition(|&b| b < 0x30 || (0x3a..=0x3f).contains(&b) || b == 0x7f);
                if let Some(at) = ascii.or_else(|| anchor.checked_sub(base).map(|n| n as usize)) {
                    return gb_advance(bytes, at, index, encoding);
                }
                // A GB18030 scalar is at most four bytes. Enumerate every possible
                // initial phase, ruling out parses inconsistent with an established
                // end (e.g. previous or last page) or an invalid byte sequence.
                let stop = known_end.map_or(bytes.len(), |end| (end - base) as usize);
                let mut unique = None;
                let max_tail = if encoding == Encoding::GbkWebEncodingRs0844 {
                    1
                } else {
                    3
                };
                for first in 0..=max_tail {
                    let Ok(start) = gb_advance(bytes, first, index, encoding) else {
                        continue;
                    };
                    let check_until = if known_end.is_some() {
                        stop
                    } else {
                        stop.saturating_sub(max_tail)
                    };
                    let Ok(end) = gb_advance(bytes, start, check_until, encoding) else {
                        continue;
                    };
                    if known_end.is_some() && end != stop {
                        continue;
                    }
                    if unique.is_some_and(|old| old != start) {
                        return Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            "此段多字节字符边界需要顺序校准。可继续查看当前页，或选择校准并定位。",
                        ));
                    }
                    unique = Some(start);
                }
                unique.ok_or_else(invalid)
            }
            _ => Err(invalid()),
        }
    }

    fn remember(&self, at: u64) {
        if !gb_encoding(self.encoding) {
            return;
        }
        // At most one proven boundary per bucket even for a terabyte-scale file.
        let stride = self.len.div_ceil(4096).max(1024 * 1024);
        let base = at / stride * stride;
        let mut anchors = self.anchors.lock().unwrap();
        if anchors
            .range(base..=base.saturating_add(stride - 1))
            .next()
            .is_none()
        {
            anchors.insert(at);
        }
    }

    fn nearest(&self, before: u64) -> u64 {
        self.anchors
            .lock()
            .unwrap()
            .range(..=before)
            .next_back()
            .copied()
            .unwrap_or(self.first)
    }

    pub fn scan_start(&self, position: Position) -> u64 {
        self.nearest(self.target(position))
    }

    fn target(&self, position: Position) -> u64 {
        match position {
            Position::Before(end) => end.min(self.len).saturating_sub(PAGE_BYTES as u64),
            Position::Boundary(n) | Position::Near(n) => n.min(self.len),
        }
        .max(self.first)
    }

    /// On explicit request, advance a bounded distance from a proven GB18030 boundary.
    /// The caller reports progress and can cancel between steps, with no whole-file buffer.
    pub fn calibrate_step(&self, position: Position, start: u64) -> io::Result<Calibration> {
        let target = self.target(position);
        if !gb_encoding(self.encoding) || start > target {
            return Err(invalid());
        }
        let to = target.min(start.saturating_add(SCAN_BYTES as u64));
        let wanted = (to - start) as usize;
        let count = (self.len - start).min(wanted as u64 + 3) as usize;
        let mut bytes = vec![0; count];
        {
            let mut file = self.source.lock()?;
            self.source.check(file.metadata()?)?;
            file.seek(SeekFrom::Start(start))?;
            file.read_exact(&mut bytes)?;
        }
        let at = start + gb_advance(&bytes, 0, wanted, self.encoding)? as u64;
        self.remember(at);
        if at < target {
            Ok(Calibration::More(at))
        } else {
            self.read_aligned(position, Some(at))
                .map(Calibration::Ready)
        }
    }
}

pub(super) fn gb_advance(
    bytes: &[u8],
    mut at: usize,
    target: usize,
    encoding: Encoding,
) -> io::Result<usize> {
    while at < target {
        at += gb_width(&bytes[at..], encoding)?;
    }
    Ok(at)
}

pub(super) fn unit(bytes: &[u8], encoding: Encoding) -> u16 {
    let pair = [bytes[0], bytes[1]];
    if encoding == Encoding::Utf16Le {
        u16::from_le_bytes(pair)
    } else {
        u16::from_be_bytes(pair)
    }
}

fn gb_encoding(encoding: Encoding) -> bool {
    matches!(
        encoding,
        Encoding::GbkWebEncodingRs0844 | Encoding::Gb18030WebEncodingRs0844
    )
}

fn gb_width(bytes: &[u8], encoding: Encoding) -> io::Result<usize> {
    let width = match bytes {
        [0..=0x80, ..] => 1,
        [0x81..=0xfe, 0x30..=0x39, 0x81..=0xfe, 0x30..=0x39, ..]
            if encoding == Encoding::Gb18030WebEncodingRs0844 =>
        {
            4
        }
        [0x81..=0xfe, 0x40..=0x7e | 0x80..=0xfe, ..] => 2,
        _ => return Err(invalid()),
    };
    Ok(width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::{Format, encode};
    use std::{
        fs,
        io::Write,
        path::PathBuf,
        sync::atomic::{AtomicU32, Ordering},
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new(bytes: &[u8]) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "texzi-paged-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = File::create_new(&path).unwrap();
            file.write_all(bytes).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn confirmed_gbk_after_an_ascii_sample_is_used_for_subsequent_pages() {
        let original = "a\n".repeat(4096) + &"中晚记\n".repeat(8000);
        let gbk = Encoding::GbkWebEncodingRs0844;
        let bytes = encode(
            &original,
            Format {
                encoding: gbk,
                bom: false,
            },
        )
        .unwrap();
        let fixture = Fixture::new(&bytes);
        let source = Arc::new(FileSource::open(&fixture.0).unwrap());
        assert_eq!(
            PagedFile::from_source(source.clone(), None)
                .unwrap()
                .encoding,
            Encoding::Utf8
        );
        let reader = PagedFile::from_source(source, Some(gbk)).unwrap();
        assert_eq!(reader.encoding, gbk);
        let mut at = 0;
        let mut all = String::new();
        while at < reader.len {
            let page = reader.read(Position::Boundary(at)).unwrap();
            assert!(page.end > at);
            at = page.end;
            all.push_str(&page.text);
        }
        assert_eq!(all, original);
        assert!(
            reader
                .read(Position::Before(reader.len))
                .unwrap()
                .text
                .ends_with("中晚记\n")
        );
    }

    #[test]
    fn paging_keeps_scalars_and_all_original_text_across_windows() {
        let original = "日😀a\r\n".repeat(14000);
        for (encoding, bom) in [
            (Encoding::Utf8, true),
            (Encoding::Utf16Le, true),
            (Encoding::Utf16Be, true),
            (Encoding::Gb18030WebEncodingRs0844, false),
        ] {
            let bytes = encode(&original, Format { encoding, bom }).unwrap();
            let fixture = Fixture::new(&bytes);
            let reader = PagedFile::open(&fixture.0).unwrap();
            assert_eq!(reader.encoding, encoding);
            let mut text = String::new();
            let mut at = reader.first;
            while at < reader.len {
                let page = reader.read(Position::Boundary(at)).unwrap();
                assert_eq!(page.start, at);
                assert!((1..=PAGE_BYTES as u64).contains(&(page.end - at)));
                text.push_str(&page.text);
                at = page.end;
            }
            assert_eq!(text, original);
            let near = reader.read(Position::Near(reader.len / 2 + 1)).unwrap();
            assert!(original.contains(&near.text));
            let previous = reader.read(Position::Before(near.start)).unwrap();
            assert_eq!(previous.end, near.start);
            let tail = reader.read(Position::Before(reader.len)).unwrap();
            assert!(original.ends_with(&tail.text));
            assert_eq!(tail.end, reader.len);
        }
    }

    #[test]
    fn first_page_does_not_decode_the_entire_file_and_changed_files_are_rejected() {
        let mut bytes = vec![b'a'; PAGE_BYTES * 8];
        let last = bytes.len() - 1;
        bytes[last] = 0xff;
        let fixture = Fixture::new(&bytes);
        let reader = PagedFile::open(&fixture.0).unwrap();
        assert_eq!(
            reader.read(Position::Boundary(0)).unwrap().text.len(),
            PAGE_BYTES
        );
        assert!(reader.read(Position::Before(reader.len)).is_err());
        // Fails before decoding any more pages when a live handle changes.
        File::options()
            .write(true)
            .open(&fixture.0)
            .unwrap()
            .set_len(3)
            .unwrap();
        assert!(reader.read(Position::Boundary(0)).is_err());
    }

    #[test]
    fn ambiguous_legacy_long_line_can_be_read_from_known_boundaries() {
        // An odd ASCII prefix followed by ambiguous pairs catches guesses based on
        // absolute file parity. The seek is deliberately several scan budgets away.
        let original = "a".to_string() + &"汉".repeat(SCAN_BYTES * 2);
        let bytes = encode(
            &original,
            Format {
                encoding: Encoding::Gb18030WebEncodingRs0844,
                bom: false,
            },
        )
        .unwrap();
        let fixture = Fixture::new(&bytes);
        let reader = PagedFile::open(&fixture.0).unwrap();
        let target = Position::Near(SCAN_BYTES as u64 * 2 + 2);
        assert_eq!(
            reader.read(target).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        let mut checkpoint = reader.scan_start(target);
        let mut steps = 0;
        let middle = loop {
            steps += 1;
            match reader.calibrate_step(target, checkpoint).unwrap() {
                Calibration::More(next) => {
                    assert!(next > checkpoint && next - checkpoint <= SCAN_BYTES as u64 + 3);
                    checkpoint = next;
                }
                Calibration::Ready(page) => break page,
            }
        };
        assert!(steps > 1);
        assert_eq!(middle.start % 2, 1);
        assert_eq!(middle.text, "汉".repeat(PAGE_BYTES / 2));
        let previous = reader.read(Position::Before(middle.start)).unwrap();
        assert_eq!(previous.end, middle.start);
        assert_eq!(previous.text, middle.text);
        let tail = reader.read(Position::Before(reader.len)).unwrap();
        assert_eq!(tail.end, reader.len);
        assert_eq!(tail.text, middle.text);
        let first = reader.read(Position::Boundary(0)).unwrap();
        let second = reader.read(Position::Boundary(first.end)).unwrap();
        assert_eq!(second.start, first.end);
        assert!(original.starts_with(&(first.text + &second.text)));
    }

    #[test]
    fn path_replacement_does_not_retarget_paging_or_full_import() {
        let fixture = Fixture::new(b"old object");
        let source = Arc::new(FileSource::open(&fixture.0).unwrap());
        let renamed = fixture.0.with_extension("renamed");
        fs::rename(&fixture.0, &renamed).unwrap();
        let _guard = Fixture(renamed);
        fs::write(&fixture.0, b"new object").unwrap();
        // Both have equal length; identity, not just length or time, protects promotion.
        assert!(source.verify_path(&fixture.0).is_err());
        let reader = PagedFile::from_source(source.clone(), Some(Encoding::Utf8)).unwrap();
        assert_eq!(
            reader.read(Position::Boundary(0)).unwrap().text,
            "old object"
        );
        assert_eq!(
            source.complete_bytes(64, &AtomicBool::new(false)).unwrap(),
            b"old object"
        );
        assert!(source.complete_bytes(2, &AtomicBool::new(false)).is_err());
        assert!(source.complete_bytes(64, &AtomicBool::new(true)).is_err());
    }
}
