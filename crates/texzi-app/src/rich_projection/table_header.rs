//! The table owns this exact, adjacent top-level Markdown prefix.
use markdown::mdast;
use nsh_base::rich_editor::model::{Fragment, Node, Schema};

pub(super) const MARKER: &str = "<!-- texzi:table-no-header -->";

pub(super) fn prefix_len(raw: &str) -> Option<usize> {
    let rest = raw.strip_prefix(MARKER)?;
    if rest.starts_with("\r\n") {
        Some(MARKER.len() + 2)
    } else if rest.starts_with('\n') {
        Some(MARKER.len() + 1)
    } else {
        None
    }
}

pub(super) fn marker_before(
    prior: &mdast::Node,
    source: &str,
    table_start: usize,
) -> Option<usize> {
    let mdast::Node::Html(comment) = prior else {
        return None;
    };
    let start = comment.position.as_ref()?.start.offset;
    let prefix = source.get(start..table_start)?;
    if prefix_len(prefix)? != prefix.len() {
        return None;
    }
    // Indented or inline comments and comments separated by a blank line
    // remain ordinary user source.
    (start == 0 || source.as_bytes()[start - 1] == b'\n').then_some(start)
}

pub(super) fn without_header(schema: &Schema, table: &Node) -> Option<Node> {
    if table.ty().name() != "table" || table.child_count() == 0 {
        return None;
    }
    let row = table.child(0);
    let cells: Vec<_> = (0..row.child_count())
        .map(|index| {
            let cell = row.child(index);
            if cell.ty().name() != "tableHeader" {
                return Some(cell.clone());
            }
            schema
                .node(
                    "tableCell",
                    Some(cell.attrs().clone()),
                    cell.content().clone(),
                    cell.marks().to_vec(),
                )
                .ok()
        })
        .collect::<Option<_>>()?;
    let rows = std::iter::once(row.copy(Fragment::from(cells)))
        .chain((1..table.child_count()).map(|index| table.child(index).clone()))
        .collect::<Vec<_>>();
    Some(table.copy(Fragment::from(rows)))
}
