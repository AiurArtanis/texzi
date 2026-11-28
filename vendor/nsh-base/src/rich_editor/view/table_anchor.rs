use super::{
    RichEditorApi,
    geometry::{DOC, Hit},
};
use crate::rich_editor::state::Selection;
use nsh_core::prelude::*;

/// A specific table cell in one revision of the editor's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableAnchor {
    pub revision: u64,
    pub table: usize,
    pub cell: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TableAnchorGeometry {
    Stale,
    Composing,
    Unmeasured,
    Clipped,
    Visible {
        cell: Bounds<Points>,
        table: Bounds<Points>,
        viewport: Size<Points>,
        /// Bottom of the preceding and top of the following visible block.
        neighbors: [Option<Points>; 2],
    },
}

impl RichEditorApi {
    /// Reads only mounted geometry from this view. Visible bounds use the
    /// same local coordinate space as its viewport layer.
    pub fn table_anchor_geometry(&self, anchor: TableAnchor) -> TableAnchorGeometry {
        use TableAnchorGeometry::*;
        let rv = &self.rv;
        rv.doc_rev.get();
        rv.sel_rev.get();
        if rv.session.state().revision() != anchor.revision {
            return Stale;
        }
        if rv.composition.get().is_some() {
            return Composing;
        }
        rv.layout_rev.get();
        rv.block_layout.get();
        let viewport = rv.viewport.get();
        let offset = rv.scroll_y.get();
        if rv.st.borrow().revision() != anchor.revision
            || viewport.width <= px(0.0)
            || viewport.height <= px(0.0)
        {
            return Unmeasured;
        }
        let tree = rv.tree.borrow();
        let geo = rv.geo.borrow();
        let mounted = || {
            let table = tree.id_at(anchor.table)?;
            let cell = tree.id_at(anchor.cell)?;
            geo.table_scroll_handles.get(&table)?.node()?;
            let table_clip = geo.table_viewport(table)?;
            let (parent, table_frame) = geo.frames.get(&table)?;
            let table_bounds = Bounds::new(geo.origin(table)?, table_frame.size);
            let (_, cell_frame) = geo.frames.get(&cell)?;
            let cell_bounds = Bounds::new(geo.origin(cell)?, cell_frame.size);
            let mut neighbors = [None; 2];
            if let Some(siblings) = tree.children.get(&parent.unwrap_or(DOC))
                && let Ok(index) = siblings.binary_search_by_key(&anchor.table, |item| {
                    tree.pos(item.id).unwrap_or(usize::MAX)
                })
            {
                let bounds = |index: usize| {
                    let id = siblings.get(index)?.id;
                    let (_, bounds) = geo.frames.get(&id)?;
                    Some(Bounds::new(geo.origin(id)?, bounds.size))
                };
                neighbors[0] = index
                    .checked_sub(1)
                    .and_then(bounds)
                    .filter(|bounds| bounds.bottom() > offset)
                    .map(|bounds| bounds.bottom() - offset);
                neighbors[1] = bounds(index + 1)
                    .filter(|bounds| bounds.origin.y < offset + viewport.height)
                    .map(|bounds| bounds.origin.y - offset);
            }
            Some((cell_bounds, table_bounds.intersect(&table_clip), neighbors))
        };
        let Some((cell, mut table, neighbors)) = mounted() else {
            return Unmeasured;
        };
        let body = Bounds::new(point(px(0.0), offset), viewport);
        let mut cell = cell.intersect(&table).intersect(&body);
        if cell.is_empty() {
            return Clipped;
        }
        cell.origin.y -= offset;
        table.origin.y -= offset;
        Visible {
            cell,
            table,
            viewport,
            neighbors,
        }
    }

    pub fn restore_table_input(&self, from: ElementRef) {
        self.rv.input.focus_back(from);
    }

    /// Scrolls the viewport without changing the document selection.
    pub fn scroll_table_viewport(&self, delta: f32, page: bool) {
        let rv = &self.rv;
        let step = if page {
            delta * (rv.viewport.get_untracked().height.0 - 24.0).max(24.0)
        } else {
            delta
        };
        rv.set_viewport_scroll(rv.scroll_y.get_untracked().0 + step);
    }

    /// An explicit Tab from the viewport resumes editing at a visible text
    /// position, without returning to the old, now offscreen, selection.
    pub fn enter_visible_table_body(&self) {
        let rv = &self.rv;
        let viewport = rv.viewport.get_untracked();
        let top = rv.scroll_y.get_untracked();
        let hit = {
            let geo = rv.geo.borrow();
            let near = point(viewport.width * 0.5, top + viewport.height * 0.45);
            match geo.hit(near) {
                Some(Hit::Text(position, affinity))
                    if geo.caret_rect(position, affinity).is_some_and(|caret| {
                        caret.bottom() > top && caret.origin.y < top + viewport.height
                    }) =>
                {
                    Some(position)
                }
                _ => None,
            }
        };
        if let Some(position) = hit {
            let state = rv.session.state();
            if let Ok(at) = state.doc().resolve(position) {
                rv.set_selection(Selection::near(&at, 1), false);
                rv.focus();
            }
        }
    }
}
