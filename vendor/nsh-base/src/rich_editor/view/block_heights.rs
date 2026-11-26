//! Prefix sums for the upstream rich-editor block window. Unknown heights
//! use the running measured average, so measuring one block need not rewrite
//! every spacer. Sums and estimates stay in f64 until passed to NSH layout.

use std::collections::HashMap;

#[derive(Clone, Copy, Default)]
struct Measured {
    sum: f64,
    known: isize,
}

impl Measured {
    fn replace(&mut self, old: Option<f32>, new: Option<f32>) {
        self.sum += f64::from(new.unwrap_or_default()) - f64::from(old.unwrap_or_default());
        self.known += isize::from(new.is_some()) - isize::from(old.is_some());
    }

    fn average(self, fallback: f64) -> f64 {
        if self.known > 0 {
            self.sum / self.known as f64
        } else {
            fallback
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Frame {
    pub top: f32,
    pub height: f32,
}

#[derive(Default)]
pub(super) struct BlockHeights {
    ids: Vec<u64>,
    indices: HashMap<u64, usize>,
    frames: Vec<Option<Frame>>,
    sums: Vec<Measured>,
    heights: Measured,
    gaps: Measured,
}

impl BlockHeights {
    /// Reindex after a structural edit, retaining the geometry of surviving
    /// block IDs. Plain typing usually leaves the block order unchanged.
    pub(super) fn sync(
        &mut self,
        ids: impl ExactSizeIterator<Item = u64> + Clone,
        frame: impl Fn(u64) -> Option<Frame>,
    ) {
        if ids.len() == self.ids.len() && ids.clone().eq(self.ids.iter().copied()) {
            return;
        }
        self.ids.clear();
        self.ids.extend(ids);
        self.indices.clear();
        self.frames.clear();
        self.sums.clear();
        self.sums.resize(self.ids.len() + 1, Measured::default());
        self.heights = Measured::default();
        self.gaps = Measured::default();
        for (i, &id) in self.ids.iter().enumerate() {
            self.indices.insert(id, i);
            let current = frame(id);
            self.frames.push(current);
            let h = current.map(|f| f.height);
            self.heights.replace(None, h);
            let n = i + 1;
            self.sums[n].replace(None, h);
            let parent = n + n.isolate_lowest_one();
            if parent < self.sums.len() {
                let part = self.sums[n];
                self.sums[parent].sum += part.sum;
                self.sums[parent].known += part.known;
            }
        }
        for i in 1..self.frames.len() {
            self.gaps
                .replace(None, gap(self.frames[i - 1], self.frames[i]));
        }
    }

    /// Only two neighbouring gaps can change when a block is laid out.
    pub(super) fn measure(&mut self, id: u64, new: Frame) -> bool {
        let Some(&i) = self.indices.get(&id) else {
            return false;
        };
        let old = self.frames[i].replace(new);
        if old == Some(new) {
            return false;
        }
        let left = i.checked_sub(1).and_then(|j| self.frames[j]);
        let right = self.frames.get(i + 1).copied().flatten();
        let old_gap = self.gaps.average(0.0);
        self.gaps.replace(gap(left, old), gap(left, Some(new)));
        self.gaps.replace(gap(old, right), gap(Some(new), right));
        let old_height = old.map(|f| f.height);
        if old_height == Some(new.height) {
            return self.gaps.average(0.0) != old_gap;
        }
        self.heights.replace(old_height, Some(new.height));
        let mut n = i + 1;
        while n < self.sums.len() {
            self.sums[n].replace(old_height, Some(new.height));
            n += n.isolate_lowest_one();
        }
        true
    }

    fn extent(&self, n: usize, measured: Measured) -> f64 {
        measured.sum
            + (n as f64 - measured.known as f64) * self.heights.average(24.0)
            + n as f64 * self.gaps.average(0.0)
    }

    /// Offset of block `end`, or the total extent for `end == len`.
    pub(super) fn prefix(&self, mut end: usize) -> f64 {
        let mut height = 0.0;
        while end > 0 {
            let step = end.isolate_lowest_one();
            height += self.extent(step, self.sums[end]);
            end -= step;
        }
        height
    }

    /// Invert the same prefix sum by descending the Fenwick tree. The two
    /// edges distinguish the first block ending at y from the last starting there.
    pub(super) fn preceding(&self, y: f64, inclusive: bool) -> usize {
        let mut step = self.ids.len().next_power_of_two();
        let (mut i, mut top) = (0, 0.0);
        while step > 0 {
            let next = i + step;
            if next <= self.ids.len() {
                let bottom = top + self.extent(step, self.sums[next]);
                if bottom < y || (inclusive && bottom == y) {
                    i = next;
                    top = bottom;
                }
            }
            step >>= 1;
        }
        i
    }
}

fn gap(a: Option<Frame>, b: Option<Frame>) -> Option<f32> {
    let a = a?;
    let value = b?.top - (a.top + a.height);
    (0.0..200.0).contains(&value).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_unknown_and_reordered_blocks_share_the_same_offsets() {
        let mut rows = BlockHeights::default();
        rows.sync([10, 11, 12, 13].into_iter(), |_| None);
        assert_eq!(rows.prefix(4), 96.0);
        assert!(rows.measure(
            10,
            Frame {
                top: 0.0,
                height: 12.0
            }
        ));
        assert!(rows.measure(
            11,
            Frame {
                top: 16.0,
                height: 20.0
            }
        ));
        // Known heights 12,20; estimated 16,16. The measured gap is 4.
        for (i, y) in [0.0, 16.0, 40.0, 60.0, 80.0].into_iter().enumerate() {
            assert_eq!(rows.prefix(i), y);
            if i > 0 {
                assert_eq!(rows.preceding(y, false), i - 1);
                assert_eq!(rows.preceding(y, true), i);
            }
        }
        assert!(rows.measure(
            11,
            Frame {
                top: 12.0,
                height: 20.0
            }
        ));
        assert_eq!(rows.prefix(4), 64.0);
        rows.sync([11, 20, 21].into_iter(), |id| {
            (id == 11).then_some(Frame {
                top: 0.0,
                height: 20.0,
            })
        });
        assert_eq!(rows.prefix(3), 60.0);
        assert!(!rows.measure(
            10,
            Frame {
                top: 0.0,
                height: 2.0
            }
        ));
        assert_eq!(rows.preceding(-1.0, false), 0);
        assert_eq!(rows.preceding(10_000.0, true), 3);
    }
}
