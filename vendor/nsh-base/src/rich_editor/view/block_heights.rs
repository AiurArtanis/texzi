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

#[derive(Clone, Copy, Default)]
struct Stamp<T> {
    epoch: u64,
    value: T,
}

impl<T: Copy + Default> Stamp<T> {
    fn get(self, epoch: u64) -> T {
        if self.epoch == epoch {
            self.value
        } else {
            T::default()
        }
    }

    fn current(&mut self, epoch: u64) -> &mut T {
        if self.epoch != epoch {
            *self = Self {
                epoch,
                value: T::default(),
            };
        }
        &mut self.value
    }
}

#[derive(Default)]
pub(super) struct BlockHeights {
    ids: Vec<u64>,
    indices: HashMap<u64, usize>,
    frames: Vec<Stamp<Option<Frame>>>,
    sums: Vec<Stamp<Measured>>,
    heights: Measured,
    gaps: Measured,
    epoch: u64,
    fallback: Option<(f64, f64)>,
}

impl BlockHeights {
    /// Reindex after a structural edit, retaining the geometry of surviving
    /// block IDs. Plain typing usually leaves the block order unchanged.
    pub(super) fn sync(&mut self, ids: impl ExactSizeIterator<Item = u64> + Clone) {
        if ids.len() == self.ids.len() && ids.clone().eq(self.ids.iter().copied()) {
            return;
        }
        let retained: HashMap<_, _> = self
            .ids
            .iter()
            .zip(&self.frames)
            .filter_map(|(&id, f)| f.get(self.epoch).map(|f| (id, f)))
            .collect();
        self.ids.clear();
        self.ids.extend(ids);
        self.indices.clear();
        self.frames.clear();
        self.sums.clear();
        self.sums.resize(
            self.ids.len() + 1,
            Stamp {
                epoch: self.epoch,
                value: Measured::default(),
            },
        );
        self.heights = Measured::default();
        self.gaps = Measured::default();
        for (i, &id) in self.ids.iter().enumerate() {
            self.indices.insert(id, i);
            let current = retained.get(&id).copied();
            self.frames.push(Stamp {
                epoch: self.epoch,
                value: current,
            });
            let h = current.map(|f| f.height);
            self.heights.replace(None, h);
            let n = i + 1;
            self.sums[n].value.replace(None, h);
            let parent = n + n.isolate_lowest_one();
            if parent < self.sums.len() {
                let part = self.sums[n].value;
                self.sums[parent].value.sum += part.sum;
                self.sums[parent].value.known += part.known;
            }
        }
        for i in 1..self.frames.len() {
            self.gaps
                .replace(None, gap(self.frames[i - 1].value, self.frames[i].value));
        }
    }

    /// Width, DPI or typography changed. Old heights may guide the initial
    /// estimate, but none is a layout receipt in the new coordinate space.
    pub(super) fn reset_layout(&mut self) {
        let (height, gap) = self.fallback.unwrap_or((24.0, 0.0));
        self.fallback = Some((self.heights.average(height), self.gaps.average(gap)));
        self.epoch += 1;
        self.heights = Measured::default();
        self.gaps = Measured::default();
    }

    pub(super) fn index(&self, id: u64) -> Option<usize> {
        self.indices.get(&id).copied()
    }

    pub(super) fn measured(&self, i: usize) -> bool {
        self.frames[i].get(self.epoch).is_some()
    }

    pub(super) fn invalidate(&mut self, id: u64) {
        self.update(id, None);
    }

    /// Only two neighbouring gaps can change when a block is laid out.
    pub(super) fn measure(&mut self, id: u64, new: Frame) -> bool {
        self.update(id, Some(new))
    }

    fn update(&mut self, id: u64, new: Option<Frame>) -> bool {
        let Some(&i) = self.indices.get(&id) else {
            return false;
        };
        let old = std::mem::replace(self.frames[i].current(self.epoch), new);
        if old == new {
            return false;
        }
        let left = i
            .checked_sub(1)
            .and_then(|j| self.frames[j].get(self.epoch));
        let right = self.frames.get(i + 1).and_then(|f| f.get(self.epoch));
        let gap_estimate = self.fallback.map_or(0.0, |v| v.1);
        let old_gap = self.gaps.average(gap_estimate);
        self.gaps.replace(gap(left, old), gap(left, new));
        self.gaps.replace(gap(old, right), gap(new, right));
        let old_height = old.map(|f| f.height);
        let new_height = new.map(|f| f.height);
        if old_height == new_height {
            return self.gaps.average(gap_estimate) != old_gap;
        }
        self.heights.replace(old_height, new_height);
        let mut n = i + 1;
        while n < self.sums.len() {
            self.sums[n]
                .current(self.epoch)
                .replace(old_height, new_height);
            n += n.isolate_lowest_one();
        }
        true
    }

    fn extent(&self, n: usize, measured: Measured) -> f64 {
        let (height, gap) = self.fallback.unwrap_or((24.0, 0.0));
        measured.sum
            + (n as f64 - measured.known as f64) * self.heights.average(height)
            + n as f64 * self.gaps.average(gap)
    }

    /// Offset of block `end`, or the total extent for `end == len`.
    pub(super) fn prefix(&self, mut end: usize) -> f64 {
        let mut height = 0.0;
        while end > 0 {
            let step = end.isolate_lowest_one();
            height += self.extent(step, self.sums[end].get(self.epoch));
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
                let bottom = top + self.extent(step, self.sums[next].get(self.epoch));
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
        rows.sync([10, 11, 12, 13].into_iter());
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
        rows.sync([11, 20, 21].into_iter());
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

    #[test]
    fn stale_layout_and_edited_content_are_estimates_until_measured() {
        let mut rows = BlockHeights::default();
        rows.sync([0, 1, 2, 3].into_iter());
        rows.measure(
            0,
            Frame {
                top: 0.0,
                height: 12.0,
            },
        );
        rows.measure(
            1,
            Frame {
                top: 16.0,
                height: 20.0,
            },
        );
        assert_eq!(rows.prefix(4), 80.0);
        rows.reset_layout();
        assert_eq!(rows.prefix(4), 80.0);
        assert!(!rows.measured(0) && !rows.measured(1));
        rows.measure(
            3,
            Frame {
                top: 120.0,
                height: 36.0,
            },
        );
        assert_eq!(rows.prefix(4), 160.0);
        rows.measure(
            2,
            Frame {
                top: 85.0,
                height: 32.0,
            },
        );
        assert_eq!(rows.prefix(4), 148.0);
        rows.invalidate(2);
        assert!(!rows.measured(2) && rows.measured(3));
        assert_eq!(rows.prefix(4), 160.0);
        rows.sync([3, 2, 1].into_iter());
        assert!(rows.measured(0) && !rows.measured(1) && !rows.measured(2));
    }
}
