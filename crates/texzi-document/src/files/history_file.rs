//! Private, quota-limited backing for immutable bulk-edit history.
use crate::Error;
use memmap2::{Mmap, MmapOptions};
use std::{
    fs::{File, OpenOptions},
    hash::{BuildHasher, RandomState},
    io::{BufWriter, Write},
    ops::Deref,
    sync::atomic::{AtomicU64, Ordering},
};

// Includes live history, observer receipts and simultaneous preparations.
const PROCESS_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const FILE_BYTES: u64 = 512 * 1024 * 1024;
static RESERVED: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);

pub(crate) fn usage() -> (u64, u64) {
    (
        RESERVED.load(Ordering::Relaxed),
        PEAK.load(Ordering::Relaxed),
    )
}

#[derive(Debug, Default)]
struct Reservation(u64);
impl Reservation {
    fn grow(&mut self, len: usize) -> Result<(), Error> {
        let len = len as u64;
        let total = self
            .0
            .checked_add(len)
            .filter(|n| *n <= FILE_BYTES)
            .ok_or(Error::HistoryBudget)?;
        let previous = RESERVED
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                n.checked_add(len).filter(|v| *v <= PROCESS_BYTES)
            })
            .map_err(|_| Error::HistoryBudget)?;
        PEAK.fetch_max(previous + len, Ordering::Relaxed);
        self.0 = total;
        Ok(())
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        RESERVED.fetch_sub(self.0, Ordering::Relaxed);
    }
}

pub(crate) struct Temporary {
    writer: BufWriter<File>,
    bytes: Reservation,
}
impl Temporary {
    pub(crate) fn new() -> Result<Self, Error> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true).read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_DELETE_ON_CLOSE;
            options
                .share_mode(0)
                .custom_flags(FILE_FLAG_DELETE_ON_CLOSE);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let nonce = RandomState::new().hash_one(sequence);
            let path = std::env::temp_dir().join(format!(
                ".texzi-bulk-{}-{nonce:016x}.tmp",
                std::process::id()
            ));
            match options.open(&path) {
                Ok(file) => {
                    #[cfg(unix)]
                    std::fs::remove_file(&path).map_err(|_| Error::HistoryBudget)?;
                    return Ok(Self {
                        writer: BufWriter::with_capacity(64 * 1024, file),
                        bytes: Reservation::default(),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(Error::HistoryBudget),
            }
        }
    }
    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.bytes.grow(bytes.len())?;
        self.writer
            .write_all(bytes)
            .map_err(|_| Error::HistoryBudget)
    }
    pub(crate) fn seal(mut self) -> Result<Backing, Error> {
        self.writer.flush().map_err(|_| Error::HistoryBudget)?;
        let file = self.writer.get_ref();
        file.sync_data().map_err(|_| Error::HistoryBudget)?;
        if file.metadata().map_err(|_| Error::HistoryBudget)?.len() != self.bytes.0 {
            return Err(Error::HistoryBudget);
        }
        // SAFETY: This exclusively created file was opened with no sharing on
        // Windows, or unlinked immediately on Unix. No writable handle escapes
        // this owner. The writer is consumed here; the mapping is read-only.
        let map = unsafe { MmapOptions::new().map(file) }.map_err(|_| Error::HistoryBudget)?;
        Ok(Backing {
            map,
            _reserved: self.bytes,
        })
    }
}

#[derive(Debug)]
pub(crate) struct Backing {
    map: Mmap,
    _reserved: Reservation,
}
impl Deref for Backing {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.map
    }
}
