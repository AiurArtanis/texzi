//! Exact source seams for flat, plain bullet-list item splits.
use super::{Block, Projection, raw_text_start};
use markdown::{ParseOptions, mdast};
use nsh_base::rich_editor::{model::Node, transform::ReplaceStep};
use std::ops::Range;

struct ItemSource {
    prefix: Range<usize>,
    body: Range<usize>,
}

fn paragraph(item: &Node) -> Option<&Node> {
    if item.ty().name() != "listItem" || item.child_count() != 1 {
        return None;
    }
    let p = item.child(0);
    (p.ty().name() == "paragraph" && (0..p.child_count()).all(|i| p.child(i).is_text()))
        .then_some(p)
}

// Bind each prefix to its AST item; never search body text for a marker.
fn item_sources(block: &Block, source: &str) -> Option<Vec<ItemSource>> {
    if block.opaque || block.node.ty().name() != "bulletList" {
        return None;
    }
    let ast = markdown::to_mdast(source.get(block.source.clone())?, &ParseOptions::gfm()).ok()?;
    let children = ast.children()?;
    let [mdast::Node::List(list)] = children.as_slice() else {
        return None;
    };
    if list.ordered || list.children.len() != block.node.child_count() {
        return None;
    }
    let mut result = Vec::new();
    for (i, child) in list.children.iter().enumerate() {
        let mdast::Node::ListItem(item) = child else {
            return None;
        };
        if item.checked.is_some()
            || !(item.children.is_empty()
                || matches!(item.children.as_slice(), [mdast::Node::Paragraph(_)]))
        {
            return None;
        }
        let p = paragraph(block.node.child(i))?;
        let at = block
            .source
            .start
            .checked_add(item.position.as_ref()?.start.offset)?;
        let line_start = source.get(..at)?.rfind('\n').map_or(0, |n| n + 1);
        let line_end = source
            .get(at..)?
            .find('\n')
            .map_or(source.len(), |n| at + n);
        let line_end = if source.get(..line_end)?.ends_with('\r') {
            line_end - 1
        } else {
            line_end
        };
        let line = source.get(line_start..line_end)?;
        let indent = line.bytes().take_while(|b| *b == b' ').count();
        if indent > 3 || !matches!(line.as_bytes().get(indent), Some(b'-' | b'+' | b'*')) {
            return None;
        }
        // The AST item must actually start at this line's marker/indent.
        if at > line_start + indent {
            return None;
        }
        let mut end = indent + 1;
        if end < line.len() && !matches!(line.as_bytes()[end], b' ' | b'\t') {
            return None;
        }
        while end < line.len() && matches!(line.as_bytes()[end], b' ' | b'\t') {
            end += 1;
        }
        if p.content().size() == 0 && end != line.len() {
            return None;
        }
        result.push(ItemSource {
            prefix: line_start..line_start + end,
            body: line_start + end..line_end,
        });
    }
    Some(result)
}

pub(super) fn empty_positions(block: &Block, source: &str) -> Vec<(usize, usize)> {
    let Some(items) = item_sources(block, source) else {
        return vec![];
    };
    let mut positions = Vec::new();
    let mut before = 0;
    for (i, item) in items.iter().enumerate() {
        let node = block.node.child(i);
        if paragraph(node).is_some_and(|p| p.content().size() == 0) {
            positions.push((before + 3, item.body.start));
        }
        before += node.node_size();
    }
    positions
}

pub(super) fn split_edit(
    projection: &Projection,
    next: &Node,
    step: &ReplaceStep,
) -> Option<(Range<usize>, String)> {
    if !step.structure
        || step.from != step.to
        || step.slice.open_start != 2
        || step.slice.open_end != 2
        || step.slice.content.child_count() != 2
        || projection.doc.child_count() != next.child_count()
    {
        return None;
    }
    let changed: Vec<_> = projection
        .blocks
        .iter()
        .enumerate()
        .filter(|(i, block)| block.node != *next.child(*i))
        .map(|(i, _)| i)
        .collect();
    let [index] = changed.as_slice() else {
        return None;
    };
    let block = &projection.blocks[*index];
    let items = item_sources(block, &projection.source)?;
    let old = &block.node;
    let new = next.child(*index);
    if !old.same_markup(new) || new.child_count() != old.child_count() + 1 {
        return None;
    }
    let block_base: usize = projection.blocks[..*index]
        .iter()
        .map(|b| b.node.node_size())
        .sum();
    let mut before = 0;
    let item_index = (0..old.child_count()).find(|i| {
        let item = old.child(*i);
        let start = block_base + before + 3;
        before += item.node_size();
        paragraph(item)
            .is_some_and(|p| step.from >= start && step.from <= start + p.content().size())
    })?;
    if (0..item_index).any(|i| old.child(i) != new.child(i))
        || (item_index + 1..old.child_count()).any(|i| old.child(i) != new.child(i + 1))
    {
        return None;
    }
    let original = old.child(item_index);
    let left_item = new.child(item_index);
    let right_item = new.child(item_index + 1);
    let p = paragraph(original)?;
    let left = paragraph(left_item)?;
    let right = paragraph(right_item)?;
    if !original.same_markup(left_item)
        || !original.same_markup(right_item)
        || !p.same_markup(left)
        || !p.same_markup(right)
        || &left.content().append(right.content()) != p.content()
    {
        return None;
    }
    for i in 0..2 {
        let slice_item = step.slice.content.child(i);
        if !slice_item.same_markup(original)
            || !paragraph(slice_item).is_some_and(|n| n.same_markup(p) && n.content().size() == 0)
        {
            return None;
        }
    }
    let item = &items[item_index];
    let text = p.text_content();
    if text.is_empty()
        || text.contains(['\r', '\n'])
        || projection.source.get(item.body.clone())? != text
        || block.source.start + raw_text_start(block, &text, &projection.source)? != item.body.start
    {
        return None;
    }
    let item_base: usize = (0..item_index).map(|i| old.child(i).node_size()).sum();
    if step.from != block_base + item_base + 3 + left.content().size() {
        return None;
    }
    let at = projection.source_position(step.from)?;
    if at != item.body.start + left.text_content().len() {
        return None;
    }
    Some((
        at..at,
        format!("\n{}", projection.source.get(item.prefix.clone())?),
    ))
}
