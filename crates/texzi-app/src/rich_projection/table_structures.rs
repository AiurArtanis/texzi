//! A bounded rewrite of one actual table, authorized by replay rather than by
//! metadata. All other blocks and every source byte outside its span survive.
use super::{Block, Projection, parse_options, table_header::MARKER};
use crate::table_controls::{META, TableAction, action_transaction};
use markdown::mdast;
use nsh_base::rich_editor::{
    model::Node,
    state::{EditorState, EditorStateConfig, Transaction},
};
use std::ops::Range;

type SourceEdit = (Range<usize>, String);

struct SourceRow {
    range: Range<usize>,
    cells: Vec<Range<usize>>,
    leading_pipe: bool,
    trailing_pipe: bool,
}
impl SourceRow {
    fn parse(source: &str, range: Range<usize>, columns: usize) -> Result<Self, String> {
        let raw = &source[range.clone()];
        let pipes: Vec<_> = raw
            .match_indices('|')
            .filter_map(|(at, _)| {
                let escapes = raw.as_bytes()[..at]
                    .iter()
                    .rev()
                    .take_while(|b| **b == b'\\')
                    .count();
                (escapes % 2 == 0).then_some(at)
            })
            .collect();
        let leading_pipe = raw.trim_start_matches([' ', '\t']).starts_with('|');
        let trailing_pipe = pipes
            .last()
            .is_some_and(|at| raw[*at + 1..].bytes().all(|b| matches!(b, b' ' | b'\t')));
        let begin = if leading_pipe {
            pipes.first().copied().ok_or("表格起始边界无效")? + 1
        } else {
            0
        };
        let end = if trailing_pipe {
            *pipes.last().ok_or("表格结束边界无效")?
        } else {
            raw.len()
        };
        let mut cells = Vec::new();
        let mut before = begin;
        for pipe in pipes.into_iter().filter(|at| *at >= begin && *at < end) {
            cells.push(range.start + before..range.start + pipe);
            before = pipe + 1;
        }
        cells.push(range.start + before..range.start + end);
        if cells.len() != columns || cells.iter().any(|cell| cell.start > cell.end) {
            return Err("该表格的省略单元格或分隔边界暂不能局部保真编辑，原文未变".into());
        }
        Ok(Self {
            range,
            cells,
            leading_pipe,
            trailing_pipe,
        })
    }
    fn insert_column(&self, source: &str, column: usize, content: &str) -> SourceEdit {
        if column < self.cells.len() {
            let prefix = if column == 0 && !self.leading_pipe {
                "|"
            } else {
                ""
            };
            let at = self.cells[column].start;
            (at..at, format!("{prefix}{content}|"))
        } else {
            let at = self.cells.last().expect("nonempty source row").end;
            let suffix = if self.trailing_pipe { "" } else { "|" };
            let _ = source;
            (at..at, format!("|{content}{suffix}"))
        }
    }
    fn delete_column(&self, source: &str, column: usize) -> SourceEdit {
        let range = if column + 1 < self.cells.len() {
            self.cells[column].start..self.cells[column + 1].start
        } else {
            self.cells[column - 1].end..self.cells[column].end
        };
        // A single-column GFM table needs an explicit pipe. Keep the surviving
        // cell's spelling and padding exactly, adding only the needed syntax.
        if self.cells.len() == 2 && !self.leading_pipe && !self.trailing_pipe {
            let remaining = if column == 0 {
                &self.cells[1]
            } else {
                &self.cells[0]
            };
            (
                self.range.clone(),
                format!("|{}|", &source[remaining.clone()]),
            )
        } else {
            (range, String::new())
        }
    }
}

struct SourceTable {
    rows: Vec<SourceRow>,
    separator: SourceRow,
    columns: usize,
}
impl SourceTable {
    fn parse(block: &Block, source: &str) -> Result<Self, String> {
        let body = block.table_source_span(source);
        let raw = source.get(body.clone()).ok_or("表格原文范围无效")?;
        if raw.contains('\r') {
            return Err("表格行结束符未规范到内部坐标，原文未变".into());
        }
        let ast = markdown::to_mdast(raw, &parse_options()).map_err(|e| e.to_string())?;
        let [mdast::Node::Table(table)] = ast.children().ok_or("表格语法树无效")?.as_slice()
        else {
            return Err("表格原文已变化".into());
        };
        let columns = table.align.len();
        if columns == 0 || table.children.len() != block.node.child_count() {
            return Err("表格行列数与原文不一致".into());
        }
        let mut rows = Vec::new();
        for row in &table.children {
            let mdast::Node::TableRow(row) = row else {
                return Err("表格行语法无效".into());
            };
            let p = row.position.as_ref().ok_or("表格行缺少原文范围")?;
            if row.children.len() != columns || p.start.line != p.end.line {
                return Err("该表格含省略单元格，暂不能局部保真编辑；原文未变".into());
            }
            let start = body.start + p.start.offset;
            let end = body.start + p.end.offset;
            let line_start = source[..start].rfind('\n').map_or(0, |at| at + 1);
            let line_end = source[end..].find('\n').map_or(source.len(), |at| end + at);
            if line_start < body.start
                || !source[end..line_end]
                    .bytes()
                    .all(|b| matches!(b, b' ' | b'\t'))
            {
                return Err("表格行不能绑定到独立原文行，原文未变".into());
            }
            rows.push(SourceRow::parse(source, line_start..line_end, columns)?);
        }
        let header = rows.first().ok_or("表格没有表头")?;
        let separator_start = header.range.end + 1;
        if source.as_bytes().get(header.range.end) != Some(&b'\n') || separator_start > source.len()
        {
            return Err("表格分隔行无效".into());
        }
        let separator_end = source[separator_start..]
            .find('\n')
            .map_or(source.len(), |at| separator_start + at);
        let separator = SourceRow::parse(source, separator_start..separator_end, columns)?;
        for cell in &separator.cells {
            marker_core(source, cell)?;
        }
        for i in 1..rows.len() {
            let previous = if i == 1 {
                separator.range.end
            } else {
                rows[i - 1].range.end
            };
            if rows[i].range.start != previous + 1 {
                return Err("表格行之间存在未映射原文，原文未变".into());
            }
        }
        Ok(Self {
            rows,
            separator,
            columns,
        })
    }
    fn remove_row(&self, source: &str, row: usize) -> SourceEdit {
        let line = &self.rows[row].range;
        if source.as_bytes().get(line.end) == Some(&b'\n') {
            (line.start..line.end + 1, String::new())
        } else {
            (line.start - 1..line.end, String::new())
        }
    }
    fn edits(
        &self,
        source: &str,
        row: usize,
        column: usize,
        action: TableAction,
        marker: Option<Range<usize>>,
    ) -> Result<Vec<SourceEdit>, String> {
        let empty_row = format!("|{}", "  |".repeat(self.columns));
        let rows = &self.rows;
        let changes = match action {
            TableAction::RowBefore if row == 0 => {
                let insert = if rows.len() > 1 {
                    (
                        rows[1].range.start..rows[1].range.start,
                        format!("{}\n", &source[rows[0].range.clone()]),
                    )
                } else {
                    (
                        self.separator.range.end..self.separator.range.end,
                        format!("\n{}", &source[rows[0].range.clone()]),
                    )
                };
                vec![(rows[0].range.clone(), empty_row), insert]
            }
            TableAction::RowBefore => vec![(
                rows[row].range.start..rows[row].range.start,
                format!("{empty_row}\n"),
            )],
            TableAction::RowAfter if row + 1 < rows.len() => vec![(
                rows[row + 1].range.start..rows[row + 1].range.start,
                format!("{empty_row}\n"),
            )],
            TableAction::RowAfter => vec![(
                rows[row].range.end..rows[row].range.end,
                format!("\n{empty_row}"),
            )],
            TableAction::DeleteRow if row == 0 => vec![
                (
                    rows[0].range.clone(),
                    source[rows[1].range.clone()].to_owned(),
                ),
                self.remove_row(source, 1),
            ],
            TableAction::DeleteRow => vec![self.remove_row(source, row)],
            TableAction::ColumnBefore | TableAction::ColumnAfter => {
                let at = column + usize::from(action == TableAction::ColumnAfter);
                rows.iter()
                    .map(|r| r.insert_column(source, at, "  "))
                    .chain(std::iter::once(
                        self.separator.insert_column(source, at, " --- "),
                    ))
                    .collect()
            }
            TableAction::DeleteColumn => rows
                .iter()
                .chain(std::iter::once(&self.separator))
                .map(|r| r.delete_column(source, column))
                .collect(),
            TableAction::AlignDefault
            | TableAction::AlignLeft
            | TableAction::AlignCenter
            | TableAction::AlignRight => {
                let range = marker_core(source, &self.separator.cells[column])?;
                let dashes = source[range.clone()].trim_matches(':');
                let left = if matches!(action, TableAction::AlignLeft | TableAction::AlignCenter) {
                    ":"
                } else {
                    ""
                };
                let right = if matches!(action, TableAction::AlignRight | TableAction::AlignCenter)
                {
                    ":"
                } else {
                    ""
                };
                vec![(range, format!("{left}{dashes}{right}"))]
            }
            TableAction::Header => {
                let mut changes = vec![(
                    rows[0].range.clone(),
                    source[rows[row].range.clone()].to_owned(),
                )];
                for i in 1..=row {
                    changes.push((
                        rows[i].range.clone(),
                        source[rows[i - 1].range.clone()].to_owned(),
                    ));
                }
                if let Some(marker) = marker {
                    changes.push((marker, String::new()));
                }
                changes
            }
            TableAction::CancelHeader if row == 0 && marker.is_none() => {
                let at = rows[0].range.start;
                vec![(at..at, format!("{MARKER}\n"))]
            }
            TableAction::CancelHeader => return Err("当前行无可取消的表头，原文未变".into()),
            TableAction::DeleteTable => return Err("表格删除应使用空段替换".into()),
        };
        Ok(changes)
    }
}

fn marker_core(source: &str, range: &Range<usize>) -> Result<Range<usize>, String> {
    let raw = &source[range.clone()];
    let trimmed = raw.trim_matches([' ', '\t']);
    let dashes = trimmed.trim_matches(':');
    if dashes.is_empty()
        || !dashes.bytes().all(|b| b == b'-')
        || trimmed.bytes().filter(|b| *b == b':').count() > 2
    {
        return Err("表格对齐分隔符不能局部保真映射，原文未变".into());
    }
    let start = range.start + raw.len() - raw.trim_start_matches([' ', '\t']).len();
    Ok(start..start + trimmed.len())
}

pub(super) fn has_exact_rows(block: &Block, source: &str) -> bool {
    SourceTable::parse(block, source).is_ok()
}

pub(super) fn transaction_edits(
    projection: &Projection,
    next: &Node,
    applied: &[Transaction],
) -> Result<Option<Vec<SourceEdit>>, String> {
    let metas: Vec<_> = applied.iter().filter_map(|tr| tr.get_meta(META)).collect();
    if metas.is_empty() {
        return Ok(None);
    }
    if metas.len() != 1 {
        return Err("表格事务标记重复，原文未变".into());
    }
    let meta = metas[0];
    let number = |key| {
        meta[key]
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .ok_or("表格操作位置无效")
    };
    let index = number("block")?;
    let row = number("row")?;
    let col = number("column")?;
    let action = meta["action"]
        .as_str()
        .and_then(TableAction::parse)
        .ok_or("表格操作类型无效")?;
    if projection.doc.child_count() != next.child_count()
        || !projection.doc.same_markup(next)
        || projection.blocks.len() != next.child_count()
        || index >= projection.blocks.len()
    {
        return Err("表格操作不能跨越其它区块，原文未变".into());
    }
    let block = &projection.blocks[index];
    if block.opaque
        || block.node.ty().name() != "table"
        || projection
            .blocks
            .iter()
            .enumerate()
            .any(|(i, old)| i != index && old.node != *next.child(i))
    {
        return Err("表格操作修改了其它区块，原文未变".into());
    }
    // Reconstruct this exact command from the current authoritative tree. A
    // forged marker cannot smuggle arbitrary cell text or unrelated changes.
    let state = EditorState::create(
        EditorStateConfig::new(next.ty().schema().clone()).doc(projection.doc.clone()),
    )
    .map_err(|e| e.to_string())?;
    let expected = action_transaction(&state, index, row, col, action)?;
    if expected.doc() != next {
        return Err("表格结果与命令不符，原文未变".into());
    }
    let node = next.child(index);
    let candidates = if node.ty().name() == "paragraph" && node.content().size() == 0 {
        vec![
            vec![(block.source.clone(), String::new())],
            vec![(block.source.clone(), "\n".into())],
            vec![(block.source.clone(), "\n\n".into())],
        ]
    } else if node.ty().name() == "table" {
        let table = SourceTable::parse(block, &projection.source)?;
        vec![table.edits(
            &projection.source,
            row,
            col,
            action,
            block.table_header_marker(&projection.source),
        )?]
    } else {
        return Err("表格结果类型无效，原文未变".into());
    };
    for mut edits in candidates {
        edits.retain(|(range, text)| projection.source[range.clone()] != *text);
        edits.sort_by_key(|(range, _)| (range.start, range.end));
        if edits
            .windows(2)
            .any(|pair| pair[0].0.end > pair[1].0.start || pair[0].0 == pair[1].0)
        {
            return Err("表格原文编辑范围重叠，原文未变".into());
        }
        let mut raw = projection.source.clone();
        for (range, text) in edits.iter().rev() {
            raw.replace_range(range.clone(), text);
        }
        if Projection::parse(next.ty().schema(), raw).is_ok_and(|p| p.doc == *next) {
            return Ok(Some(edits));
        }
    }
    Err("表格操作不能保真往返，原文未变".into())
}
