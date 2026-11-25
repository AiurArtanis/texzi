//! A fixed scroll container with a window of live top-level block views.
use std::collections::HashMap;
use std::ops::Range;

use nsh_core::prelude::*;

use super::{
    controller::{BlockItem, Rv},
    geometry::DOC,
};

pub(super) const WINDOWS_VIEWPORT_ID: &str = "texzi.rich.virtual-scroll";
pub(super) const ROW_BIT: u64 = 1 << 61;
pub(super) const ROWS_ROOT: u64 = ROW_BIT;
const DEFAULT_VIEWPORT: f32 = 480.0;

pub(super) fn enabled(count: usize) -> bool {
    count >= 256
        && std::env::var_os("TEXZI_RICH_VISIBLE_ROWS").as_deref() != Some(std::ffi::OsStr::new("0"))
}

struct Row {
    item: BlockItem,
    estimate: f32,
    measured: Option<f32>,
}

impl Row {
    fn height(&self) -> f32 {
        self.measured.unwrap_or(self.estimate)
    }
}

pub(crate) struct Rows {
    rows: Vec<Row>,
    indices: HashMap<u64, usize>,
    offsets: Vec<f32>,
    pending: HashMap<u64, f32>,
    scheduled: bool,
    width: f32,
}

fn estimate(item: &BlockItem, width: f32) -> f32 {
    let text = item.node.text_content();
    let available = (width - 24.0).max(80.0);
    let lines: f32 = text
        .split('\n')
        .map(|line| {
            let points: f32 = line
                .chars()
                .map(|ch| if ch.is_ascii() { 8.0 } else { 16.0 })
                .sum();
            (points / available).ceil().max(1.0)
        })
        .sum();
    lines * 24.0 + 12.0
}

impl Rows {
    fn new(items: &[BlockItem], width: f32) -> Self {
        let mut rows = Self {
            rows: Vec::new(),
            indices: HashMap::new(),
            offsets: vec![0.0],
            pending: HashMap::new(),
            scheduled: false,
            width,
        };
        rows.replace(items, width);
        rows
    }

    fn replace(&mut self, items: &[BlockItem], width: f32) {
        let same_width = (width - self.width).abs() < 1.0;
        self.rows = items
            .iter()
            .map(|item| {
                let old = self
                    .indices
                    .get(&item.id)
                    .map(|&i| &self.rows[i])
                    .filter(|row| same_width && row.item.node.ptr_eq(&item.node));
                Row {
                    item: item.clone(),
                    estimate: old.map_or_else(|| estimate(item, width), |row| row.estimate),
                    measured: old.and_then(|row| row.measured),
                }
            })
            .collect();
        self.indices = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| (row.item.id, i))
            .collect();
        self.pending.clear();
        self.width = width;
        self.rebuild_offsets();
    }

    fn rebuild_offsets(&mut self) {
        self.offsets.clear();
        self.offsets.push(0.0);
        self.offsets
            .extend(self.rows.iter().scan(0.0, |total, row| {
                *total += row.height();
                Some(*total)
            }));
    }

    fn extent(&self) -> f32 {
        self.offsets.last().copied().unwrap_or_default()
    }

    fn at(&self, y: f32) -> usize {
        self.offsets
            .partition_point(|offset| *offset <= y)
            .saturating_sub(1)
            .min(self.rows.len().saturating_sub(1))
    }

    fn window(&self, y: f32, height: f32) -> Range<usize> {
        let height = if height > 0.0 {
            height
        } else {
            DEFAULT_VIEWPORT
        };
        let start = self.at((y - height).max(0.0));
        let stop = (self.at(y + 2.0 * height) + 1).min(self.rows.len());
        start..stop
    }

    fn anchor(&self, y: f32) -> Option<(u64, f32)> {
        let index = self.at(y);
        let row = self.rows.get(index)?;
        Some((row.item.id, (y - self.offsets[index]).max(0.0)))
    }

    fn anchored_y(&self, anchor: Option<(u64, f32)>) -> Option<f32> {
        let (id, within) = anchor?;
        let index = *self.indices.get(&id)?;
        Some(self.offsets[index] + within.min((self.rows[index].height() - 1.0).max(0.0)))
    }
}

fn publish(rv: &Rv, to: Option<f32>) {
    batch(|| {
        rv.visible_rows_rev.update(|revision| *revision += 1);
        if let Some(to) = to {
            rv.move_visible_row_scroll(to);
        }
    });
}

fn measure_row(rv: &Rv, id: u64, height: f32) {
    if !height.is_finite() || height <= 0.0 {
        return;
    }
    let schedule = {
        let mut state = rv.visible_rows.borrow_mut();
        let Some(rows) = state.as_mut() else { return };
        let Some(index) = rows.indices.get(&id).copied() else {
            return;
        };
        if (rows.rows[index].height() - height).abs() < 0.5 {
            return;
        }
        rows.pending.insert(id, height);
        !std::mem::replace(&mut rows.scheduled, true)
    };
    if schedule {
        let owner = rv.owner;
        let rv = rv.clone();
        owner.run(|| {
            spawn(async move {
                next_frame().await;
                let mut state = rv.visible_rows.borrow_mut();
                let Some(rows) = state.as_mut() else { return };
                rows.scheduled = false;
                let pending = std::mem::take(&mut rows.pending);
                if pending.is_empty() {
                    return;
                }
                let anchor = rows.anchor(rv.scroll_y.get_untracked().0);
                for (id, height) in pending {
                    if let Some(&index) = rows.indices.get(&id) {
                        rows.rows[index].measured = Some(height);
                    }
                }
                rows.rebuild_offsets();
                let to = rows.anchored_y(anchor);
                drop(state);
                publish(&rv, to);
            })
        });
    }
}

impl Rv {
    pub(super) fn scroll_visible_rows_by(&self, delta: f32) {
        if !delta.is_finite() || delta == 0.0 {
            return;
        }
        let Some(extent) = self.visible_rows.borrow().as_ref().map(Rows::extent) else {
            return;
        };
        let current = self.scroll_y.get_untracked().0;
        let from = self
            .wheel_target
            .get_untracked()
            .map_or(current, |(_, y)| y);
        let limit = (extent - self.viewport.get_untracked().height.0).max(0.0);
        let target = (from + delta).clamp(0.0, limit);
        if (target - from).abs() <= 0.5 {
            return;
        }

        let generation = self.wheel_generation.get().wrapping_add(1);
        self.wheel_generation.set(generation);
        self.wheel_target.set(Some((generation, target)));
        // Keep the displayed rows while mounting the destination. The
        // platform runs this fence after applying the preceding mount.
        let mounted = nsh_core::element::run_on_main_thread(|| ());
        let rv = self.clone();
        self.owner.run(|| {
            spawn(async move {
                let ready = mounted.await.is_ok();
                if rv.wheel_target.get_untracked() != Some((generation, target)) {
                    return;
                }
                batch(|| {
                    if ready {
                        let current = rv.scroll_y.get_untracked().0;
                        if (target - current).abs() > 0.5 {
                            if rv.wheel_held_offset.get_untracked().is_none() {
                                rv.wheel_held_offset.set(Some(current));
                            }
                            rv.pending_wheel_scroll.set(Some(target));
                            rv.move_visible_row_scroll(target);
                        }
                    }
                    rv.wheel_target.set(None);
                });
            })
        });
    }

    pub(super) fn cancel_wheel_scroll(&self) {
        batch(|| {
            self.pending_wheel_scroll.set(None);
            self.wheel_target.set(None);
            self.wheel_held_offset.set(None);
        });
    }

    pub(super) fn observe_scroll(&self, e: &ScrollEvent) {
        self.viewport.set(e.size);
        if let Some(target) = self.pending_wheel_scroll.get() {
            let limit = (e.content_size.height - e.size.height).0.max(0.0);
            if (e.offset.y.0 - target.clamp(0.0, limit)).abs() > 0.5 {
                return;
            }
            self.pending_wheel_scroll.set(None);
            self.wheel_held_offset.set(None);
        }
        self.scroll_y.set(e.offset.y);
        self.scrolled.set(e.offset.y);
    }

    pub(crate) fn reveal_unmounted_row(&self, position: usize) -> bool {
        let to = self.visible_rows.borrow().as_ref().and_then(|rows| {
            let index = rows
                .rows
                .partition_point(|row| row.item.pos < position)
                .saturating_sub(1);
            let range = rows.window(
                self.scroll_y.get_untracked().0,
                self.viewport.get_untracked().height.0,
            );
            (!range.contains(&index)).then(|| rows.offsets[index])
        });
        if let Some(to) = to {
            self.move_visible_row_scroll(to);
        }
        to.is_some()
    }
}

pub(super) fn render(rv: Rv, list: Memo<Vec<BlockItem>>) -> AnyElement {
    *rv.visible_rows.borrow_mut() = Some(Rows::new(&list.get_untracked(), 720.0));
    let changed = rv.clone();
    effect(move || {
        let items = list.get();
        let width = changed.viewport.get().width.0;
        let width = if width > 0.0 { width } else { 720.0 };
        let mut state = changed.visible_rows.borrow_mut();
        let rows = state.as_mut().expect("windowed editor");
        if (rows.width - width).abs() < 1.0
            && rows.rows.iter().map(|row| &row.item).eq(items.iter())
        {
            return;
        }
        let anchor = rows.anchor(changed.scroll_y.get_untracked().0);
        rows.replace(&items, width);
        let to = rows.anchored_y(anchor);
        drop(state);
        publish(&changed, to);
    });
    let window = rv.clone();
    let shown = computed(move || {
        window.visible_rows_rev.get();
        let offset = window.scroll_y.get().0;
        let height = window.viewport.get().height.0;
        let state = window.visible_rows.borrow();
        let rows = state.as_ref().expect("windowed editor");
        let mut indices = rows.window(offset, height).collect::<Vec<_>>();
        if let Some((_, target)) = window.wheel_target.get() {
            indices.extend(rows.window(target, height));
        }
        if let Some(held) = window.wheel_held_offset.get() {
            indices.extend(rows.window(held, height));
        }
        indices.sort_unstable();
        indices.dedup();
        indices
            .into_iter()
            .map(|index| rows.rows[index].item.clone())
            .collect::<Vec<_>>()
    });
    let extent = rv.clone();
    let layout = rv.clone();
    let trace =
        std::env::var_os("TEXZI_TRACE_RICH_ROWS").as_deref() == Some(std::ffi::OsStr::new("1"));
    view()
        .relative()
        .w_full()
        .flex_shrink_0()
        .h(move || {
            extent.visible_rows_rev.get();
            px(extent
                .visible_rows
                .borrow()
                .as_ref()
                .expect("windowed editor")
                .extent())
        })
        .on_layout(move |b: &Bounds<Points>| layout.set_frame(ROWS_ROOT, Some(DOC), *b))
        .child(each(
            shown,
            |item: &BlockItem| item.id,
            move |item, _| {
                let id = item.get_untracked().id;
                if trace {
                    let state = rv.visible_rows.borrow();
                    let rows = state.as_ref().expect("windowed editor");
                    log::info!("TEXZI_RICH_ROW_CREATED index={}", rows.indices[&id] + 1);
                }
                let frame = ROW_BIT | id;
                let place = rv.clone();
                let measure = rv.clone();
                let dispose = rv.clone();
                cleanup(move || {
                    let mut geo = dispose.geo.borrow_mut();
                    geo.frames.remove(&id);
                    geo.frames.remove(&frame);
                });
                view()
                    .absolute()
                    .left(px(0.0))
                    .w_full()
                    .top(move || {
                        place.visible_rows_rev.get();
                        let state = place.visible_rows.borrow();
                        let rows = state.as_ref().expect("windowed editor");
                        px(rows.indices.get(&id).map_or(0.0, |i| rows.offsets[*i]))
                    })
                    .on_layout(move |b: &Bounds<Points>| {
                        measure.set_frame(frame, Some(ROWS_ROOT), *b);
                        measure_row(&measure, id, b.size.height.0);
                    })
                    .child(super::render::block(rv.clone(), item, Some(frame)))
            },
        ))
        .into_element()
}
