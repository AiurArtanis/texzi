use crate::{Error, Receipt, SourceByte};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Affinity {
    Before,
    After,
}

impl Receipt {
    /// Maps a logical anchor independently of any active view. No UI ownership.
    pub fn map_anchor(&self, p: SourceByte, affinity: Affinity) -> Result<SourceByte, Error> {
        if p.doc != self.snapshot.doc {
            return Err(Error::InvalidDoc);
        }
        if p.revision != self.previous {
            return Err(Error::StaleRevision);
        }
        self.previous_body.scalar(p.offset)?;
        let mut shift = 0isize;
        for d in &self.forward {
            if p.offset < d.range.start {
                break;
            }
            // A deletion's right endpoint is outside its half-open interval.
            // Continue so an adjacent insertion at that endpoint can honor affinity.
            if p.offset < d.range.end {
                let start = d
                    .range
                    .start
                    .checked_add_signed(shift)
                    .ok_or(Error::InvalidBoundary)?;
                return Ok(SourceByte::at(
                    &self.snapshot,
                    start
                        + if affinity == Affinity::After {
                            d.text.len()
                        } else {
                            0
                        },
                ));
            }
            if p.offset == d.range.start && affinity == Affinity::Before {
                continue;
            }
            shift += d.text.len() as isize - (d.range.end - d.range.start) as isize;
        }
        Ok(SourceByte::at(
            &self.snapshot,
            p.offset
                .checked_add_signed(shift)
                .ok_or(Error::InvalidBoundary)?,
        ))
    }
}
