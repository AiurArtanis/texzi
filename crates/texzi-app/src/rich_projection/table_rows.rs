//! Exact source seams for a plain GFM table's appended empty row.
use super::{Block, Projection};
use markdown::{ParseOptions, mdast};
use nsh_base::rich_editor::{model::Node, transform::ReplaceStep};
use std::ops::Range;

struct CellSource {
    span: Range<usize>,
    empty: bool,
}

struct RowSource {
    line: Range<usize>,
    cells: Vec<CellSource>,
}

struct TableSource {
    columns: usize,
    rows: Vec<RowSource>,
}

fn cell_paragraph(cell: &Node, header: bool) -> Option<&Node> {
    if cell.ty().name() != if header { "tableHeader" } else { "tableCell" }
        || cell.child_count() != 1
        || ["colspan", "rowspan"]
            .iter()
            .any(|name| cell.attrs().get(*name).and_then(|v| v.as_u64()) != Some(1))
    {
        return None;
    }
    let paragraph = cell.child(0);
    (paragraph.ty().name() == "paragraph").then_some(paragraph)
}

// Bind every rich row and cell to an actual GFM AST node. In particular,
// an importer-filled cell in a short source row has no source seam.
fn table_source(block: &Block, source: &str) -> Option<TableSource> {
    if block.opaque || block.node.ty().name() != "table" {
        return None;
    }
    let raw = source.get(block.source.clone())?;
    // Owner normally supplies canonical LF. Raw CRLF is deliberately outside
    // this seam so a CR can never be mistaken for cell padding or an LF byte.
    if raw.contains('\r') {
        return None;
    }
    let ast = markdown::to_mdast(raw, &ParseOptions::gfm()).ok()?;
    let [mdast::Node::Table(table)] = ast.children()?.as_slice() else {
        return None;
    };
    let columns = table.align.len();
    if columns == 0 || table.children.is_empty() || table.children.len() != block.node.child_count()
    {
        return None;
    }
    let mut rows = Vec::new();
    for (i, child) in table.children.iter().enumerate() {
        let mdast::Node::TableRow(row) = child else {
            return None;
        };
        let rich_row = block.node.child(i);
        if rich_row.ty().name() != "tableRow"
            || rich_row.child_count() != columns
            || row.children.len() != columns
        {
            return None;
        }
        let position = row.position.as_ref()?;
        if position.start.line != position.end.line
            || position.start.offset > position.end.offset
            || position.end.offset > raw.len()
        {
            return None;
        }
        let start = block.source.start.checked_add(position.start.offset)?;
        let end = block.source.start.checked_add(position.end.offset)?;
        let line_start = source.get(..start)?.rfind('\n').map_or(0, |at| at + 1);
        let line_end = source
            .get(end..)?
            .find('\n')
            .map_or(source.len(), |at| end + at);
        // mdast may end before the physical row's trailing spaces. Extend
        // only across ASCII padding, never across later opaque source.
        if !source
            .get(end..line_end)?
            .bytes()
            .all(|b| matches!(b, b' ' | b'\t'))
            || source.get(line_start..line_end)?.contains('\r')
            || rows
                .last()
                .is_some_and(|previous: &RowSource| previous.line.end >= line_start)
        {
            return None;
        }
        let mut cells = Vec::new();
        for (j, child) in row.children.iter().enumerate() {
            let mdast::Node::TableCell(cell) = child else {
                return None;
            };
            cell_paragraph(rich_row.child(j), i == 0)?;
            let cell_position = cell.position.as_ref()?;
            if cell_position.start.offset > cell_position.end.offset
                || cell_position.start.offset < position.start.offset
                || cell_position.end.offset > position.end.offset
            {
                return None;
            }
            cells.push(CellSource {
                span: block.source.start.checked_add(cell_position.start.offset)?
                    ..block.source.start.checked_add(cell_position.end.offset)?,
                empty: cell.children.is_empty(),
            });
        }
        rows.push(RowSource {
            line: line_start..line_end,
            cells,
        });
    }
    let last = rows.last()?;
    if block.source.end > last.line.end
        || !source
            .get(block.source.end..last.line.end)?
            .bytes()
            .all(|b| matches!(b, b' ' | b'\t'))
    {
        return None;
    }
    Some(TableSource { columns, rows })
}

pub(super) fn empty_positions(block: &Block, source: &str) -> Vec<(usize, usize)> {
    let Some(table) = table_source(block, source) else {
        return vec![];
    };
    let mut positions = Vec::new();
    let mut before_row = 0;
    for (i, row) in table.rows.iter().enumerate() {
        let rich_row = block.node.child(i);
        let Some(line) = source.get(row.line.clone()) else {
            return vec![];
        };
        // GFM splits on unescaped pipes even inside code spans. Count the
        // immediately preceding backslashes so an escaped content pipe never
        // steals the source seam of a later empty cell in the same row.
        let plain = line.trim_matches(|c| c == ' ' || c == '\t');
        if !plain.starts_with('|') || !plain.ends_with('|') {
            before_row += rich_row.node_size();
            continue;
        }
        let pipes: Vec<_> = line
            .match_indices('|')
            .filter_map(|(at, _)| {
                let escapes = line.as_bytes()[..at]
                    .iter()
                    .rev()
                    .take_while(|b| **b == b'\\')
                    .count();
                (escapes % 2 == 0).then_some(at)
            })
            .collect();
        if pipes.len() != table.columns + 1 {
            before_row += rich_row.node_size();
            continue;
        }
        let mut row_positions = Vec::new();
        let mut before_cell = 0;
        let mut valid = true;
        for (j, cell) in row.cells.iter().enumerate() {
            let left = row.line.start + pipes[j];
            let right = row.line.start + pipes[j + 1];
            // Cell AST spans include a left pipe/padding, and the final cell
            // may include its trailing pipe/padding. Keep them within the
            // corresponding physical cell rather than using span.start as
            // the caret.
            let span_end = if j + 1 == table.columns {
                row.line.end
            } else {
                right + 1
            };
            if cell.span.start < if j == 0 { row.line.start } else { left }
                || cell.span.start > right
                || cell.span.end < left + 1
                || cell.span.end > span_end
            {
                valid = false;
                break;
            }
            let rich_cell = rich_row.child(j);
            let paragraph = rich_cell.child(0);
            if paragraph.content().size() == 0
                && cell.empty
                && source
                    .get(left + 1..right)
                    .is_some_and(|text| text.bytes().all(|b| matches!(b, b' ' | b'\t')))
            {
                let padding_start = left + 1;
                let caret = padding_start + usize::from(padding_start < right);
                row_positions.push((before_row + before_cell + 4, caret));
            }
            before_cell += rich_cell.node_size();
        }
        if valid {
            positions.extend(row_positions);
        }
        before_row += rich_row.node_size();
    }
    positions
}

pub(super) fn append_edit(
    projection: &Projection,
    next: &Node,
    step: &ReplaceStep,
) -> Result<Option<(Range<usize>, String)>, String> {
    if step.structure
        || step.from != step.to
        || step.slice.open_start != 0
        || step.slice.open_end != 0
        || step.slice.content.child_count() != 1
        || projection.doc.child_count() != next.child_count()
        || projection.blocks.len() != next.child_count()
        || !projection.doc.same_markup(next)
    {
        return Ok(None);
    }
    let changed: Vec<_> = projection
        .blocks
        .iter()
        .enumerate()
        .filter(|(i, block)| block.node != *next.child(*i))
        .map(|(i, _)| i)
        .collect();
    let [index] = changed.as_slice() else {
        return Ok(None);
    };
    let block = &projection.blocks[*index];
    let old = &block.node;
    let new = next.child(*index);
    if block.opaque
        || old.ty().name() != "table"
        || !old.same_markup(new)
        || old.child_count() == 0
        || new.child_count() != old.child_count() + 1
        || (0..old.child_count()).any(|i| old.child(i) != new.child(i))
    {
        return Ok(None);
    }
    let appended = new.child(old.child_count());
    let columns = old.child(0).child_count();
    if appended.ty().name() != "tableRow"
        || columns == 0
        || appended.child_count() != columns
        || (0..columns).any(|i| {
            !cell_paragraph(appended.child(i), false)
                .is_some_and(|paragraph| paragraph.content().size() == 0)
        })
        || *step.slice.content.child(0) != *appended
    {
        return Ok(None);
    }
    let base: usize = projection.blocks[..*index]
        .iter()
        .map(|block| block.node.node_size())
        .sum();
    if step.from != base + old.node_size() - 1 {
        return Ok(None);
    }
    let Some(table) = table_source(block, &projection.source) else {
        return Err("该表格追加行暂不能保真映射，原文未变".into());
    };
    if table.columns != columns {
        return Err("表格列数与原文不一致，原文未变".into());
    }
    let end = table.rows.last().expect("table has rows").line.end;
    let mut row = String::from("|");
    for _ in 0..columns {
        row.push_str("  |");
    }
    let (at, text) = if projection.source.as_bytes().get(end) == Some(&b'\n') {
        // Retain the old row's LF identity and the original blank-line tail.
        (end + 1, format!("{row}\n"))
    } else {
        (end, format!("\n{row}"))
    };
    Ok(Some((at..at, text)))
}
