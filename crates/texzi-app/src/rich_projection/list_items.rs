//! List key bindings expressed as AST-bound source-prefix edits.
//! Every candidate must reproduce the complete projected tree.
use super::{Block, Projection};
use markdown::{ParseOptions, mdast};
use nsh_base::rich_editor::{
    model::Node,
    state::Transaction,
    transform::{ReplaceAroundStep, ReplaceStep},
};
use std::ops::Range;

type Edit = (Range<usize>, String);

struct ItemSource {
    prefix: Range<usize>,
    body: Range<usize>,
    span: Range<usize>,
    rich: Range<usize>,
    paragraph: Range<usize>,
    indent: usize,
    marker: Range<usize>,
    checked: Option<usize>,
    parent: Option<usize>,
    previous: Option<usize>,
}

fn list_name(node: &Node) -> bool {
    matches!(node.ty().name(), "bulletList" | "orderedList" | "taskList")
}

fn collect(
    ast: &mdast::List,
    list: &Node,
    base: usize,
    parent: Option<usize>,
    source: &str,
    out: &mut Vec<ItemSource>,
) -> Option<()> {
    if !list_name(list) || ast.children.len() != list.child_count() {
        return None;
    }
    let mut before = base + 1;
    let mut previous = None;
    for (i, child) in ast.children.iter().enumerate() {
        let mdast::Node::ListItem(item) = child else {
            return None;
        };
        let node = list.child(i);
        if !matches!(node.ty().name(), "listItem" | "taskItem") || node.child_count() == 0 {
            return None;
        }
        let p = node.child(0);
        if p.ty().name() != "paragraph"
            || (!item.children.is_empty()
                && (!matches!(item.children[0], mdast::Node::Paragraph(_))
                    || item.children.len() != node.child_count()))
        {
            return None;
        }
        let position = item.position.as_ref()?;
        let at = position.start.offset;
        let start = source.get(..at)?.rfind('\n').map_or(0, |n| n + 1);
        let end = source
            .get(at..)?
            .find('\n')
            .map_or(source.len(), |n| at + n);
        let end = end - usize::from(source.get(..end)?.ends_with('\r'));
        let line = source.get(start..end)?;
        let indent = line.bytes().take_while(|b| *b == b' ').count();
        if at > start + indent {
            return None;
        }
        let mut marker_end = indent;
        if ast.ordered {
            while line
                .as_bytes()
                .get(marker_end)
                .is_some_and(u8::is_ascii_digit)
            {
                marker_end += 1;
            }
            if marker_end == indent || !matches!(line.as_bytes().get(marker_end), Some(b'.' | b')'))
            {
                return None;
            }
            marker_end += 1;
        } else if matches!(line.as_bytes().get(marker_end), Some(b'-' | b'+' | b'*')) {
            marker_end += 1;
        } else {
            return None;
        }
        // Marker-only imports deliberately have no editable source boundary.
        if !matches!(line.as_bytes().get(marker_end), Some(b' ' | b'\t')) {
            return None;
        }
        let mut body = marker_end;
        while matches!(line.as_bytes().get(body), Some(b' ' | b'\t')) {
            body += 1;
        }
        let checked = if item.checked.is_some() {
            if !matches!(line.get(body..body + 3), Some("[ ]" | "[x]" | "[X]")) {
                return None;
            }
            let at = start + body + 1;
            body += 3;
            while matches!(line.as_bytes().get(body), Some(b' ' | b'\t')) {
                body += 1;
            }
            Some(at)
        } else {
            None
        };
        let index = out.len();
        out.push(ItemSource {
            prefix: start..start + body,
            body: start + body..end,
            span: start..position.end.offset,
            rich: before..before + node.node_size(),
            paragraph: before + 2..before + 2 + p.content().size(),
            indent,
            marker: start + indent..start + marker_end,
            checked,
            parent,
            previous,
        });
        let mut child_base = before + 1 + p.node_size();
        for (j, child) in item.children.iter().enumerate().skip(1) {
            let mdast::Node::List(nested) = child else {
                return None;
            };
            collect(nested, node.child(j), child_base, Some(index), source, out)?;
            child_base += node.child(j).node_size();
        }
        previous = Some(index);
        before += node.node_size();
    }
    Some(())
}

fn item_sources(block: &Block, source: &str) -> Option<Vec<ItemSource>> {
    if block.opaque || !list_name(&block.node) {
        return None;
    }
    let tree = markdown::to_mdast(source, &ParseOptions::gfm()).ok()?;
    let ast = tree.children()?.iter().find(|n| {
        n.position()
            .is_some_and(|p| p.start.offset == block.source.start)
    })?;
    let mdast::Node::List(list) = ast else {
        return None;
    };
    let mut result = Vec::new();
    collect(list, &block.node, 0, None, source, &mut result)?;
    Some(result)
}

pub(super) fn empty_positions(block: &Block, source: &str) -> Vec<(usize, usize)> {
    item_sources(block, source)
        .unwrap_or_default()
        .into_iter()
        .filter(|item| item.paragraph.is_empty() && item.body.is_empty())
        .map(|item| (item.paragraph.start, item.body.start))
        .collect()
}

fn verified(projection: &Projection, next: &Node, mut edits: Vec<Edit>) -> Option<Vec<Edit>> {
    edits.sort_by_key(|(range, _)| range.start);
    if edits.windows(2).any(|pair| pair[0].0.end > pair[1].0.start) {
        return None;
    }
    let mut raw = projection.source.clone();
    for (range, text) in edits.iter().rev() {
        raw.get(range.clone())?;
        raw.replace_range(range.clone(), text);
    }
    Projection::parse(next.ty().schema(), raw)
        .ok()
        .filter(|p| &p.doc == next)
        .map(|_| edits)
}

fn continuation(item: &ItemSource, source: &str) -> Option<String> {
    let mut prefix = source.get(item.prefix.clone())?.to_owned();
    if let Some(at) = item.checked {
        prefix.replace_range(at - item.prefix.start..at - item.prefix.start + 1, " ");
    }
    let marker = source.get(item.marker.clone())?;
    if marker.as_bytes().first()?.is_ascii_digit() {
        let number: u64 = marker.get(..marker.len() - 1)?.parse().ok()?;
        let at = item.marker.start - item.prefix.start;
        prefix.replace_range(
            at..at + marker.len() - 1,
            &number.checked_add(1)?.to_string(),
        );
    }
    Some(prefix)
}

fn split_candidate(projection: &Projection, next: &Node, step: &ReplaceStep) -> Option<Edit> {
    if !step.structure
        || step.from != step.to
        || step.slice.open_start != 2
        || step.slice.open_end != 2
    {
        return None;
    }
    let mut base = 0;
    for block in &projection.blocks {
        if let Some(items) = item_sources(block, &projection.source) {
            for item in items {
                if item.paragraph.start + base <= step.from
                    && step.from <= item.paragraph.end + base
                {
                    let at = projection.source_position(step.from)?;
                    if !item.body.contains(&at) && at != item.body.end {
                        return None;
                    }
                    let edit = (
                        at..at,
                        format!("\n{}", continuation(&item, &projection.source)?),
                    );
                    if verified(projection, next, vec![edit.clone()]).is_some() {
                        return Some(edit);
                    }
                }
            }
        }
        base += block.node.node_size();
    }
    None
}

// Compatibility with the older flat-list entry point; same full-tree check.
pub(super) fn split_edit(projection: &Projection, next: &Node, step: &ReplaceStep) -> Option<Edit> {
    split_candidate(projection, next, step)
}

fn indentation(item: &ItemSource, source: &str, amount: usize, add: bool) -> Option<Vec<Edit>> {
    let mut edits = Vec::new();
    let mut at = item.span.start;
    loop {
        if add {
            edits.push((at..at, " ".repeat(amount)));
        } else {
            if source.get(at..at + amount)? != " ".repeat(amount) {
                return None;
            }
            edits.push((at..at + amount, String::new()));
        }
        let Some(n) = source.get(at..item.span.end)?.find('\n') else {
            break;
        };
        at += n + 1;
        if at >= item.span.end {
            break;
        }
    }
    Some(edits)
}

pub(super) fn transaction_edits(
    projection: &Projection,
    next: &Node,
    applied: &[Transaction],
) -> Result<Option<Vec<Edit>>, String> {
    if &projection.doc == next {
        return Ok(None);
    }
    let steps: Vec<_> = applied.iter().flat_map(|tr| tr.steps.iter()).collect();
    let [step] = steps.as_slice() else {
        let structural = steps.iter().any(|step| {
            step.as_any().is::<ReplaceAroundStep>()
                || step
                    .as_any()
                    .downcast_ref::<ReplaceStep>()
                    .is_some_and(|s| {
                        s.structure
                            || s.slice.open_start > 0
                            || s.slice.open_end > 0
                            || (0..s.slice.content.child_count())
                                .any(|i| !s.slice.content.child(i).is_text())
                    })
        });
        let from = steps.first().and_then(|step| {
            step.as_any()
                .downcast_ref::<ReplaceStep>()
                .map(|s| s.from)
                .or_else(|| {
                    step.as_any()
                        .downcast_ref::<ReplaceAroundStep>()
                        .map(|s| s.from)
                })
        });
        if structural && let Some(from) = from {
            let mut base = 0;
            for block in &projection.blocks {
                let end = base + block.node.node_size();
                if list_name(&block.node) && from > base && from < end {
                    return Err("该多步列表结构操作暂不能保真映射，原文未变".into());
                }
                base = end;
            }
        }
        return Ok(None);
    };
    let replace = step.as_any().downcast_ref::<ReplaceStep>();
    let around = step.as_any().downcast_ref::<ReplaceAroundStep>();
    let structural = around.is_some()
        || replace.is_some_and(|s| {
            s.structure
                || s.slice.open_start > 0
                || s.slice.open_end > 0
                || (0..s.slice.content.child_count()).any(|i| !s.slice.content.child(i).is_text())
        });
    if !structural {
        return Ok(None);
    }
    if let Some(step) = replace {
        if let Some(edit) = split_candidate(projection, next, step) {
            return Ok(Some(vec![edit]));
        }
    }
    let (from, to) = around
        .map(|s| (s.from, s.to))
        .or_else(|| replace.map(|s| (s.from, s.to)))
        .unwrap();
    let mut base = 0;
    let mut in_list = false;
    for block in &projection.blocks {
        let end = base + block.node.node_size();
        if list_name(&block.node) && from < end && to > base {
            in_list = true;
        }
        let Some(items) = item_sources(block, &projection.source) else {
            base = end;
            continue;
        };
        for item in &items {
            // A structural step, not a text search, identifies the source item.
            let matches = around.is_some_and(|s| {
                (s.gap_from == base + item.rich.start && s.gap_to == base + item.rich.end)
                    || (s.gap_from == base + item.rich.start + 1
                        && s.gap_to == base + item.rich.end - 1)
            }) || (item.paragraph.is_empty()
                && from <= base + item.paragraph.start
                && to >= base + item.paragraph.start);
            if !matches {
                continue;
            }
            let mut candidates = Vec::new();
            if let Some(previous) = item.previous {
                let previous = &items[previous];
                if let Some(amount) = (previous.marker.end - previous.prefix.start + 1)
                    .checked_sub(item.indent)
                    .filter(|n| *n > 0 && *n <= 10)
                {
                    if let Some(edits) = indentation(item, &projection.source, amount, true) {
                        candidates.push(edits);
                    }
                }
            }
            if let Some(parent) = item.parent {
                let parent = &items[parent];
                if let Some(amount) = item.indent.checked_sub(parent.indent).filter(|n| *n > 0) {
                    if let Some(edits) = indentation(item, &projection.source, amount, false) {
                        candidates.push(edits);
                    }
                }
                if item.paragraph.is_empty() && item.body.is_empty() {
                    candidates.push(vec![(
                        item.prefix.clone(),
                        continuation(parent, &projection.source).ok_or("列表前缀无效")?,
                    )]);
                }
            } else {
                // Lift strips only this item's prefix. Separator variants
                // account for the parser's projected empty paragraphs.
                for before in 0..=2 {
                    for after in 0..=2 {
                        let mut edits = vec![(item.prefix.clone(), "\n".repeat(before))];
                        if after > 0 {
                            edits.push((item.span.end..item.span.end, "\n".repeat(after)));
                        }
                        candidates.push(edits);
                    }
                }
            }
            for edits in candidates {
                if let Some(edits) = verified(projection, next, edits) {
                    return Ok(Some(edits));
                }
            }
        }
        base = end;
    }
    if in_list {
        Err("该列表结构操作暂不能保真映射，原文未变".into())
    } else {
        Ok(None)
    }
}
