use crate::{
    Error,
    files::history_file::{Backing, Temporary},
};

// Random access stays O(1) per run in either representation. Indexes store
// numeric positions only; a long irregular edit spills without collecting it.
const MEMORY_BYTES: usize = 128 * 1024;
pub(crate) type Row = [u32; 6];
const ROW_BYTES: usize = size_of::<Row>();
#[derive(Debug)]
pub(crate) enum Table {
    Memory(Vec<Row>),
    File(Backing),
}
impl Table {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Memory(rows) => rows.len(),
            Self::File(bytes) => bytes.len() / ROW_BYTES,
        }
    }
    pub(crate) fn allocation(&self) -> usize {
        match self {
            Self::Memory(rows) => rows.capacity() * ROW_BYTES,
            Self::File(_) => 0,
        }
    }
    pub(crate) fn disk_bytes(&self) -> usize {
        match self {
            Self::Memory(_) => 0,
            Self::File(bytes) => bytes.len(),
        }
    }
    pub(crate) fn get(&self, at: usize) -> Row {
        match self {
            Self::Memory(rows) => rows[at],
            Self::File(bytes) => std::array::from_fn(|n| {
                let start = at * ROW_BYTES + 4 * n;
                u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap())
            }),
        }
    }
    pub(crate) fn partition(&self, predicate: impl Fn(Row) -> bool) -> usize {
        let (mut low, mut high) = (0, self.len());
        while low < high {
            let mid = low + (high - low) / 2;
            if predicate(self.get(mid)) {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        low
    }
}
#[derive(Default)]
pub(crate) struct Writer {
    rows: Vec<Row>,
    disk: Option<Temporary>,
}
impl Writer {
    pub(crate) fn push(&mut self, row: Row) -> Result<(), Error> {
        let max_rows = MEMORY_BYTES / ROW_BYTES;
        if self.disk.is_none() && self.rows.len() >= max_rows {
            let mut disk = Temporary::new()?;
            for row in std::mem::take(&mut self.rows) {
                Self::write(&mut disk, row)?;
            }
            self.disk = Some(disk);
        }
        if let Some(file) = &mut self.disk {
            Self::write(file, row)
        } else {
            if self.rows.len() == self.rows.capacity() {
                let next = (2 * self.rows.capacity()).max(4).min(max_rows);
                self.rows.reserve_exact(next - self.rows.len());
            }
            self.rows.push(row);
            Ok(())
        }
    }
    fn write(file: &mut Temporary, row: Row) -> Result<(), Error> {
        let mut bytes = [0; ROW_BYTES];
        for (chunk, word) in bytes.chunks_exact_mut(4).zip(row) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        file.append(&bytes)
    }
    pub(crate) fn finish(self) -> Result<Table, Error> {
        if let Some(file) = self.disk {
            Ok(Table::File(file.seal()?))
        } else {
            Ok(Table::Memory(self.rows))
        }
    }
}
