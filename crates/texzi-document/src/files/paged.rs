//! Random-access, read-only text windows. No line index or full-file allocation.
use super::{Encoding, decode};
use std::{
    fs::{File, Metadata},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::Mutex,
    time::SystemTime,
};

pub const PAGE_BYTES: usize = 16 * 1024;
const SYNC_BYTES: usize = 4096;
const SAMPLE_BYTES: usize = 4096;

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

pub struct PagedFile {
    file: Mutex<File>,
    pub len: u64,
    pub first: u64,
    pub encoding: Encoding,
    modified: SystemTime,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "无法按当前编码解码此页")
}

impl PagedFile {
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::open_as(path, None)
    }

    /// For a BOM-less file whose ASCII prefix did not identify the body encoding.
    pub fn open_as(path: &Path, choice: Option<Encoding>) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::other("只能分页查看普通文件"));
        }
        let len = metadata.len();
        let mut sample = vec![0; len.min(SAMPLE_BYTES as u64) as usize];
        file.read_exact(&mut sample)?;
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
            file: Mutex::new(file),
            len,
            first,
            encoding,
            modified: metadata.modified()?,
        })
    }

    fn check(&self, metadata: Metadata) -> io::Result<()> {
        if metadata.len() == self.len && metadata.modified()? == self.modified {
            Ok(())
        } else {
            Err(io::Error::other("文件已更改，请关闭后重新打开"))
        }
    }

    pub fn read(&self, position: Position) -> io::Result<Page> {
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
        let mut file = self
            .file
            .lock()
            .map_err(|_| io::Error::other("读取已中断"))?;
        self.check(file.metadata()?)?;
        let guard = match (position, self.encoding) {
            (Position::Boundary(_), _) => 0,
            (_, Encoding::Gb18030WebEncodingRs0844) => SYNC_BYTES,
            _ => 4,
        };
        let base = offset.saturating_sub(guard as u64).max(self.first);
        file.seek(SeekFrom::Start(base))?;
        let count = (self.len - base).min(offset - base + PAGE_BYTES as u64 + 8) as usize;
        let mut bytes = vec![0; count];
        file.read_exact(&mut bytes)?;
        self.check(file.metadata()?)?;
        let index = (offset - base) as usize;
        let start = if !matches!(position, Position::Boundary(_)) && offset < self.len {
            self.align(&bytes, base, index)?
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
            Encoding::Gb18030WebEncodingRs0844 => {
                let mut end = 0;
                while end < limit {
                    let n = gb_width(&window[end..])?;
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
        Ok(Page {
            text,
            start: base + start as u64,
            end: base + (start + end) as u64,
        })
    }

    fn align(&self, bytes: &[u8], base: u64, index: usize) -> io::Result<usize> {
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
            Encoding::Gb18030WebEncodingRs0844 => {
                // These ASCII byte values cannot be a trail or digit in a GB18030 scalar.
                // A legacy line without a synchronization point is still readable sequentially.
                let mut at = if base == self.first {
                    0
                } else {
                    bytes[..=index]
                        .iter()
                        .rposition(|&b| b < 0x30 || (0x3a..=0x3f).contains(&b) || b == 0x7f)
                        .ok_or_else(|| {
                            io::Error::other(
                                "此位置缺少可靠的 GB18030 字符边界，请从上个已知位置向后翻页",
                            )
                        })?
                };
                while at < index {
                    at += gb_width(&bytes[at..])?;
                }
                Ok(at)
            }
            _ => Err(invalid()),
        }
    }
}

fn unit(bytes: &[u8], encoding: Encoding) -> u16 {
    let pair = [bytes[0], bytes[1]];
    if encoding == Encoding::Utf16Le {
        u16::from_le_bytes(pair)
    } else {
        u16::from_be_bytes(pair)
    }
}

fn gb_width(bytes: &[u8]) -> io::Result<usize> {
    let width = match bytes {
        [0..=0x80, ..] => 1,
        [0x81..=0xfe, 0x30..=0x39, 0x81..=0xfe, 0x30..=0x39, ..] => 4,
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
        let original = "汉".repeat(PAGE_BYTES * 3);
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
        assert!(reader.read(Position::Near(PAGE_BYTES as u64 + 1)).is_err());
        let first = reader.read(Position::Boundary(0)).unwrap();
        let second = reader.read(Position::Boundary(first.end)).unwrap();
        assert_eq!(second.start, first.end);
        assert_eq!(first.text + &second.text, "汉".repeat(PAGE_BYTES));
    }
}
