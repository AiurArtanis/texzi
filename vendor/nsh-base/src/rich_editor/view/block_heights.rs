//! Prefix sums of the layout height occupied by each top-level block,
//! including its margins. Moving a mounted range cannot change its heights.
//! Unknown blocks use the measured average. Sums stay in f64 until layout.

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
    pitches: Vec<Stamp<Option<f32>>>,
    sums: Vec<Stamp<Measured>>,
    heights: Measured,
    epoch: u64,
    fallback: Option<f64>,
    pixel_ratio: Option<f64>,
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
            .zip(&self.pitches)
            .filter_map(|(&id, f)| f.get(self.epoch).map(|f| (id, f)))
            .collect();
        self.ids.clear();
        self.ids.extend(ids);
        self.indices.clear();
        self.pitches.clear();
        self.sums.clear();
        self.sums.resize(
            self.ids.len() + 1,
            Stamp {
                epoch: self.epoch,
                value: Measured::default(),
            },
        );
        self.heights = Measured::default();
        for (i, &id) in self.ids.iter().enumerate() {
            self.indices.insert(id, i);
            let current = retained.get(&id).copied();
            self.pitches.push(Stamp {
                epoch: self.epoch,
                value: current,
            });
            self.heights.replace(None, current);
            let n = i + 1;
            self.sums[n].value.replace(None, current);
            let parent = n + n.isolate_lowest_one();
            if parent < self.sums.len() {
                let part = self.sums[n].value;
                self.sums[parent].value.sum += part.sum;
                self.sums[parent].value.known += part.known;
            }
        }
    }

    /// Width, DPI or typography changed. Old heights may guide the initial
    /// estimate, but none is a layout receipt in the new coordinate space.
    pub(super) fn reset_layout(&mut self, pixel_ratio: f32) {
        self.fallback = Some(self.heights.average(self.fallback.unwrap_or(24.0)));
        self.pixel_ratio =
            (pixel_ratio.is_finite() && pixel_ratio > 0.0).then_some(f64::from(pixel_ratio));
        self.epoch += 1;
        self.heights = Measured::default();
    }

    pub(super) fn index(&self, id: u64) -> Option<usize> {
        self.indices.get(&id).copied()
    }

    pub(super) fn measured(&self, i: usize) -> bool {
        self.pitches[i].get(self.epoch).is_some()
    }

    pub(super) fn invalidate(&mut self, id: u64) {
        if let Some(i) = self.index(id)
            && self.measured(i)
        {
            self.record(i, None);
        }
    }

    pub(super) fn measure(&mut self, id: u64, pitch: f32) -> bool {
        let Some(i) = self.index(id) else {
            return false;
        };
        // Core rounds both absolute edges to device pixels. Translating an
        // unchanged row can alternate their distance by one device pixel.
        // Compare with the last accepted pitch so genuine growth accumulates.
        let scale = self.pixel_ratio.unwrap_or(1.0);
        let pixels = (f64::from(pitch) * scale).round();
        if self.pitches[i]
            .get(self.epoch)
            .is_some_and(|old| ((f64::from(old) * scale).round() - pixels).abs() <= 1.0)
        {
            return false;
        }
        self.record(i, Some((pixels / scale) as f32));
        true
    }

    fn record(&mut self, i: usize, new: Option<f32>) {
        let old = std::mem::replace(self.pitches[i].current(self.epoch), new);
        self.heights.replace(old, new);
        let mut n = i + 1;
        while n < self.sums.len() {
            self.sums[n].current(self.epoch).replace(old, new);
            n += n.isolate_lowest_one();
        }
    }

    fn extent(&self, n: usize, measured: Measured) -> f64 {
        measured.sum
            + (n as f64 - measured.known as f64)
                * self.heights.average(self.fallback.unwrap_or(24.0))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_unknown_and_reordered_blocks_share_the_same_offsets() {
        let mut rows = BlockHeights::default();
        rows.sync([10, 11, 12, 13].into_iter());
        assert_eq!(rows.prefix(4), 96.0);
        // Two known row boxes of 16 and 24, two estimated at 20.
        assert!(rows.measure(10, 16.0));
        assert!(rows.measure(11, 24.0));
        for (i, y) in [0.0, 16.0, 40.0, 60.0, 80.0].into_iter().enumerate() {
            assert_eq!(rows.prefix(i), y);
            if i > 0 {
                assert_eq!(rows.preceding(y, false), i - 1);
                assert_eq!(rows.preceding(y, true), i);
            }
        }
        assert!(rows.measure(10, 12.0));
        assert!(rows.measure(11, 20.0));
        assert_eq!(rows.prefix(4), 64.0);
        rows.sync([11, 20, 21].into_iter());
        assert_eq!(rows.prefix(3), 60.0);
        assert!(!rows.measure(10, 2.0));
        assert_eq!(rows.preceding(-1.0, false), 0);
        assert_eq!(rows.preceding(10_000.0, true), 3);
    }

    #[test]
    fn stale_layout_and_edited_content_are_estimates_until_measured() {
        let mut rows = BlockHeights::default();
        rows.sync([0, 1, 2, 3].into_iter());
        rows.measure(0, 16.0);
        rows.measure(1, 24.0);
        assert_eq!(rows.prefix(4), 80.0);
        rows.reset_layout(1.0);
        assert_eq!(rows.prefix(4), 80.0);
        assert!(!rows.measured(0) && !rows.measured(1));
        rows.measure(3, 40.0);
        assert_eq!(rows.prefix(4), 160.0);
        rows.measure(2, 34.0);
        assert_eq!(rows.prefix(4), 148.0);
        rows.invalidate(2);
        assert!(!rows.measured(2) && rows.measured(3));
        assert_eq!(rows.prefix(4), 160.0);
        rows.sync([3, 2, 1].into_iter());
        assert!(rows.measured(0) && !rows.measured(1) && !rows.measured(2));
    }

    #[test]
    fn rounded_edges_do_not_move_a_still_page_but_real_growth_and_reflow_do() {
        let mut rows = BlockHeights::default();
        rows.reset_layout(1.25);
        rows.sync((0..8192u32).map(u64::from));
        rows.measure(0, 30.4);
        let above = rows.prefix(7296);
        // A 30 DIP row alternates between 37 and 38 device pixels.
        for pitch in [29.6, 30.4, 29.6] {
            assert!(!rows.measure(0, pitch));
            assert_eq!(rows.prefix(7296), above);
        }
        // Growth beyond the ambiguity and a fresh content/layout measurement
        // at any size must still affect scrolling.
        assert!(rows.measure(0, 32.0));
        assert!(rows.prefix(7296) > above);
        rows.invalidate(0);
        assert!(rows.measure(0, 31.2));
        rows.reset_layout(1.5);
        assert!(rows.measure(0, 30.0));
    }
}
