use super::*;
use nsh_base::rich_editor::model::Node;
use std::{ops::Range, sync::atomic::AtomicBool};
use texzi_search::{Rope, RopeBuilder};

#[derive(Clone)]
struct Run {
    bytes: Range<usize>,
    rich: Range<usize>,
    units: usize,
    text: bool,
}

/// A revision-local view of rendered words. One mapping per leaf/separator;
/// UTF-16 positions within a leaf come from the Rope's measured tree.
pub(crate) struct RenderedText {
    pub(crate) rope: Rope,
    runs: Vec<Run>,
}
impl RenderedText {
    pub(crate) fn build(root: &Node, cancel: &AtomicBool) -> Result<Self, String> {
        struct Builder {
            rope: RopeBuilder,
            runs: Vec<Run>,
            bytes: usize,
            units: usize,
        }
        impl Builder {
            fn append(
                &mut self,
                text: &str,
                positions: Range<usize>,
                leaf: bool,
                cancel: &AtomicBool,
            ) -> Result<(), String> {
                if text.is_empty() {
                    return Ok(());
                }
                if leaf {
                    if let Some(last) = self.runs.last_mut().filter(|run| !run.text) {
                        last.rich.end = positions.start.max(last.rich.start);
                    }
                }
                let bytes = self.bytes;
                let units = self.units;
                let mut from = 0;
                while from < text.len() {
                    if cancel.load(Ordering::Relaxed) {
                        return Err("已取消查找".into());
                    }
                    let mut to = (from + 16 * 1024).min(text.len());
                    while !text.is_char_boundary(to) {
                        to -= 1;
                    }
                    let part = &text[from..to];
                    self.rope.append(part);
                    self.units += part.encode_utf16().count();
                    from = to;
                }
                self.bytes += text.len();
                self.runs.push(Run {
                    bytes: bytes..self.bytes,
                    rich: positions,
                    units,
                    text: leaf,
                });
                Ok(())
            }
            fn visit(
                &mut self,
                node: &Node,
                pos: usize,
                root: bool,
                cancel: &AtomicBool,
            ) -> Result<(), String> {
                if cancel.load(Ordering::Relaxed) {
                    return Err("已取消查找".into());
                }
                if let Some(value) = node.text() {
                    return self.append(value, pos..pos + node.node_size(), true, cancel);
                }
                if node.ty().name() == "hardBreak" {
                    return self.append("\n", pos..pos + 1, true, cancel);
                }
                let mut at = if root { pos } else { pos + 1 };
                for i in 0..node.child_count() {
                    let child = node.child(i);
                    if i != 0 && child.is_block() {
                        let previous = self.runs.last().map_or(pos, |run| run.rich.end);
                        let separator = if node.ty().name() == "tableRow" {
                            "\t"
                        } else {
                            "\n"
                        };
                        self.append(separator, previous..at + 1, false, cancel)?;
                    }
                    self.visit(child, at, false, cancel)?;
                    at += child.node_size();
                }
                Ok(())
            }
        }
        let mut b = Builder {
            rope: RopeBuilder::new(),
            runs: Vec::new(),
            bytes: 0,
            units: 0,
        };
        b.visit(root, 0, true, cancel)?;
        Ok(Self {
            rope: b.rope.finish(),
            runs: b.runs,
        })
    }
    pub(crate) fn rich_at(&self, byte: usize, end: bool) -> Option<usize> {
        if byte > self.rope.len_bytes() {
            return None;
        }
        let index = self.runs.partition_point(|run| {
            if end {
                run.bytes.end < byte
            } else {
                run.bytes.end <= byte
            }
        });
        let run = self.runs.get(index).or_else(|| self.runs.last())?;
        if byte <= run.bytes.start {
            return Some(run.rich.start);
        }
        if byte >= run.bytes.end {
            return Some(run.rich.end);
        }
        if !run.text {
            return None;
        }
        let units = self.rope.char_to_utf16_cu(self.rope.byte_to_char(byte));
        Some(run.rich.start + units - run.units)
    }
    pub(crate) fn byte_at(&self, rich: usize, end: bool) -> usize {
        let index = self.runs.partition_point(|run| {
            if end {
                run.rich.end < rich
            } else {
                run.rich.end <= rich
            }
        });
        let Some(run) = self.runs.get(index) else {
            return self.rope.len_bytes();
        };
        if rich <= run.rich.start {
            return run.bytes.start;
        }
        if rich >= run.rich.end {
            return run.bytes.end;
        }
        if !run.text {
            return if end { run.bytes.start } else { run.bytes.end };
        }
        let units = run.units + rich - run.rich.start;
        self.rope.char_to_byte(self.rope.utf16_cu_to_char(units))
    }
}

impl ModeGate {
    pub(crate) fn find_source_position(&self, rich: usize) -> Option<usize> {
        let active = self.active.borrow();
        active.as_ref()?.projection.borrow().source_position(rich)
    }

    pub(crate) fn find_rich_root(&self) -> Option<(Node, Range<usize>)> {
        let active = self.active.borrow();
        let owner = active.as_ref()?;
        owner.refresh_external().ok()?;
        let state = owner.session.state();
        Some((
            state.doc().clone(),
            state.selection().from()..state.selection().to(),
        ))
    }
    pub(crate) fn find_rich_scope(
        &self,
        source: &[Range<usize>],
    ) -> Result<Vec<Range<usize>>, String> {
        let active = self.active.borrow();
        let owner = active.as_ref().ok_or("正文视图尚未就绪")?;
        let projection = owner.projection.borrow();
        source
            .iter()
            .map(|r| {
                let start = projection
                    .rich_position(r.start)
                    .ok_or("选区起点暂不能映射至正文")?;
                let end = projection
                    .rich_position(r.end)
                    .ok_or("选区终点暂不能映射至正文")?;
                Ok(start.min(end)..start.max(end))
            })
            .collect()
    }
    pub(crate) fn find_rich_select(
        &self,
        revision: u64,
        range: Range<usize>,
    ) -> Result<(), String> {
        let active = self.active.borrow();
        let owner = active.as_ref().ok_or("正文视图尚未就绪")?;
        owner.refresh_external()?;
        let (snap, _) = owner.native.search_snapshot()?;
        if snap.revision.0 != revision {
            return Err("正文已变化，请重新查找".into());
        }
        let bound = |pos, affinity| -> Result<usize, String> {
            let projection = owner.projection.borrow();
            let source = projection
                .source_position(pos)
                .ok_or("匹配位置暂不能映射为正文选区")?;
            let nearest = snap
                .snap_source_offset(source, affinity)
                .map_err(|e| format!("{e:?}"))?;
            if nearest == source {
                Ok(pos)
            } else {
                projection
                    .rich_position(nearest)
                    .ok_or_else(|| "匹配位置暂不能映射为完整字素".into())
            }
        };
        let a = bound(range.start, texzi_document::Affinity::Before)?;
        let z = if range.is_empty() {
            a
        } else {
            bound(range.end, texzi_document::Affinity::After)?
        };
        let state = owner.session.state();
        let mut tr = state.tr();
        tr.set_selection(TextSelection::create(state.doc(), a, Some(z)));
        tr.scroll_into_view();
        owner.dispatch(&owner.session, tr)
    }
}
