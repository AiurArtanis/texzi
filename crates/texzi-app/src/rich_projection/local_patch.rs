//! Verified local rewrites. Every candidate must reparse to the requested tree.
use super::{Block, Projection};
use nsh_base::rich_editor::{
    model::{Mark, Node},
    transform::ReplaceStep,
};
use std::ops::Range;

fn escaped(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn encoded(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            ' ' => out.push_str("&#32;"),
            '\t' => out.push_str("&#9;"),
            _ => {
                if ch.is_ascii_punctuation() {
                    out.push('\\');
                }
                out.push(ch);
            }
        }
    }
    out
}

fn candidates(p: &Projection, pos: usize) -> Vec<usize> {
    let mut base = 0;
    let mut out = Vec::new();
    let mut edges = Vec::new();
    for block in &p.blocks {
        if pos >= base && pos <= base + block.node.node_size() {
            out.extend(block.text_map.source_positions(pos - base));
            if block.node.is_textblock() && !block.opaque {
                if pos == base + 1 {
                    edges.push(block.source.start);
                }
                if pos == base + block.node.node_size() - 1 {
                    edges.push(block.source.end);
                }
            }
        }
        base += block.node.node_size();
    }
    if let Some(at) = p.source_position(pos) {
        if !out.contains(&at) {
            out.push(at);
        }
    }
    // At a textblock edge, visible text may start/end inside a link or
    // another inline wrapper. Try its outside source boundary only after
    // exact visible boundaries; text_step verifies the complete next tree.
    for at in edges {
        if !out.contains(&at) {
            out.push(at);
        }
    }
    out
}

pub(super) fn text_step(
    p: &Projection,
    next: &Node,
    step: &ReplaceStep,
) -> Option<(Range<usize>, String)> {
    let text: String = (0..step.slice.content.child_count())
        .filter_map(|i| step.slice.content.child(i).text())
        .collect();
    if step.from == step.to && text.is_empty() {
        return None;
    }
    let starts = candidates(p, step.from);
    let ends = candidates(p, step.to);
    for inserted in [text.clone(), escaped(&text), encoded(&text)] {
        for &a in &starts {
            for &z in &ends {
                if a > z || (step.from == step.to && a != z) {
                    continue;
                }
                let mut raw = p.source.clone();
                raw.replace_range(a..z, &inserted);
                if Projection::parse(next.ty().schema(), raw)
                    .is_ok_and(|projected| &projected.doc == next)
                {
                    return Some((a..z, inserted));
                }
            }
        }
    }
    None
}

struct Verify<'a> {
    projection: &'a Projection,
    next: &'a Node,
}

fn verified(
    verify: &Verify<'_>,
    block: &Block,
    range: Range<usize>,
    text: String,
) -> Option<(Range<usize>, String)> {
    if range.start < block.source.start || range.end > block.source.end {
        return None;
    }
    let mut raw = verify.projection.source.clone();
    raw.get(range.clone())?;
    raw.replace_range(range.clone(), &text);
    // Reference labels depend on definitions outside this block. Retain that
    // import context and verify every block, not just the edited child.
    let projected = Projection::parse(verify.next.ty().schema(), raw).ok()?;
    (&projected.doc == verify.next).then_some((range, text))
}

fn heading(
    block: &Block,
    next: &Node,
    source: &str,
    verify: &Verify<'_>,
) -> Option<(Range<usize>, String)> {
    let old = &block.node;
    let (a, z) = (old.ty().name(), next.ty().name());
    if !matches!(a, "heading" | "paragraph")
        || !matches!(z, "heading" | "paragraph")
        || a == "paragraph" && z == "paragraph"
    {
        return None;
    }
    let raw = source.get(block.source.clone())?;
    let level = next
        .attrs()
        .get("level")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    if old.content() == next.content() {
        let first = raw.split('\n').next()?;
        let indent = first.bytes().take_while(|b| *b == b' ').count();
        let run = first[indent..].bytes().take_while(|b| *b == b'#').count();
        if a == "heading" && run > 0 {
            let end = indent + run;
            let marker = if z == "heading" {
                "#".repeat(level)
            } else {
                String::new()
            };
            let end = if z == "paragraph" {
                end + first[end..]
                    .bytes()
                    .take_while(|b| matches!(b, b' ' | b'\t'))
                    .count()
            } else {
                end
            };
            if let Some(edit) = verified(
                verify,
                block,
                block.source.start + indent..block.source.start + end,
                marker,
            ) {
                return Some(edit);
            }
        }
        if a == "heading" && run == 0 {
            if let Some(at) = raw.rfind('\n') {
                let underline = &raw[at + 1..];
                let pad = underline.bytes().take_while(|b| *b == b' ').count();
                let count = underline[pad..].trim_end().len();
                if z == "heading" && matches!(level, 1 | 2) {
                    let marker = if level == 1 { "=" } else { "-" }.repeat(count);
                    if let Some(edit) = verified(
                        verify,
                        block,
                        block.source.start + at + 1 + pad
                            ..block.source.start + at + 1 + pad + count,
                        marker,
                    ) {
                        return Some(edit);
                    }
                }
                let text = if z == "heading" {
                    format!("{} {}", "#".repeat(level), &raw[..at])
                } else {
                    raw[..at].into()
                };
                if let Some(edit) = verified(verify, block, block.source.clone(), text) {
                    return Some(edit);
                }
            }
        }
        if a == "paragraph" && z == "heading" {
            if let Some(edit) = verified(
                verify,
                block,
                block.source.start..block.source.start,
                format!("{} ", "#".repeat(level)),
            ) {
                return Some(edit);
            }
        }
    }
    // A heading input rule consumes only the paragraph's literal hash run.
    // Escaped hashes used during its preceding keystrokes are owned by this
    // exact block; unrelated escaped/entity text never gets serialized.
    if a == "paragraph"
        && z == "heading"
        && next.content().size() == 0
        && (old.text_content() == "#".repeat(level)
            || old.text_content() == format!("{} ", "#".repeat(level)))
    {
        return verified(
            verify,
            block,
            block.source.clone(),
            format!("{} ", "#".repeat(level)),
        );
    }
    None
}

fn text_leaf(block: &Block, next: &Node, verify: &Verify<'_>) -> Option<(Range<usize>, String)> {
    fn changed<'a>(
        a: &'a Node,
        z: &'a Node,
        at: usize,
        out: &mut Vec<(usize, &'a str, &'a str)>,
    ) -> Option<()> {
        if a == z {
            return Some(());
        }
        if !a.same_markup(z) {
            return None;
        }
        if let (Some(x), Some(y)) = (a.text(), z.text()) {
            out.push((at, x, y));
            return Some(());
        }
        if a.child_count() != z.child_count() {
            return None;
        }
        let mut offset = at + 1;
        for i in 0..a.child_count() {
            changed(a.child(i), z.child(i), offset, out)?;
            offset += a.child(i).node_size();
        }
        Some(())
    }
    let mut leaves = Vec::new();
    changed(&block.node, next, 0, &mut leaves)?;
    let [(at, old, new)] = leaves.as_slice() else {
        return None;
    };
    let prefix = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, z)| a == z)
        .map(|(a, _)| a.len_utf8())
        .sum::<usize>();
    let suffix = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, z)| a == z)
        .map(|(a, _)| a.len_utf8())
        .sum::<usize>();
    let from = at + old[..prefix].encode_utf16().count();
    let to = at + old[..old.len() - suffix].encode_utf16().count();
    let replacement = &new[prefix..new.len() - suffix];
    for a in block.text_map.source_positions(from) {
        for z in block.text_map.source_positions(to) {
            if a > z {
                continue;
            }
            for text in [
                replacement.to_owned(),
                escaped(replacement),
                encoded(replacement),
            ] {
                if let Some(edit) = verified(verify, block, a..z, text) {
                    return Some(edit);
                }
            }
        }
    }
    None
}

fn marked_chars(node: &Node) -> Vec<(usize, char, Vec<Mark>)> {
    let mut out = Vec::new();
    node.descendants(&mut |n, at, _, _| {
        if let Some(text) = n.text() {
            let mut pos = at + 1;
            for ch in text.chars() {
                out.push((pos, ch, n.marks().to_vec()));
                pos += ch.len_utf16();
            }
        }
        true
    });
    out
}

fn inline_code(
    block: &Block,
    next: &Node,
    source: &str,
    verify: &Verify<'_>,
) -> Option<(Range<usize>, String)> {
    fn leaves(node: &Node) -> Vec<(usize, Node)> {
        let mut out = Vec::new();
        node.descendants(&mut |n, at, _, _| {
            if n.is_text() {
                out.push((at + 1, n.clone()));
            }
            true
        });
        out
    }
    let old = leaves(&block.node);
    let new = leaves(next);
    if old.len() != new.len() {
        return None;
    }
    let changed: Vec<_> = old.iter().zip(&new).filter(|(a, z)| a.1 != z.1).collect();
    let [((at, old), (_, new))] = changed.as_slice() else {
        return None;
    };
    if !old.same_markup(new) || !old.marks().iter().any(|m| m.ty.name() == "code") {
        return None;
    }
    let old_text = old.text()?;
    let new_text = new.text()?;
    let ast = markdown::to_mdast(
        source.get(block.source.clone())?,
        &markdown::ParseOptions::gfm(),
    )
    .ok()?;
    fn spans<'a>(node: &'a markdown::mdast::Node, out: &mut Vec<&'a markdown::mdast::InlineCode>) {
        if let markdown::mdast::Node::InlineCode(code) = node {
            out.push(code);
        }
        if let Some(children) = node.children() {
            for child in children {
                spans(child, out);
            }
        }
    }
    let mut codes = Vec::new();
    spans(&ast, &mut codes);
    for code in codes {
        if code.value != old_text {
            continue;
        }
        let p = code.position.as_ref()?;
        let range = block.source.start + p.start.offset..block.source.start + p.end.offset;
        if !block
            .text_map
            .source_positions(*at)
            .iter()
            .any(|a| *a >= range.start && *a < range.end)
            || !block
                .text_map
                .source_positions(*at + old_text.encode_utf16().count())
                .iter()
                .any(|z| *z > range.start && *z <= range.end)
        {
            continue;
        }
        let previous = source
            .get(range.clone())?
            .bytes()
            .take_while(|b| *b == b'`')
            .count();
        let required = new_text
            .split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            + 1;
        let fence = "`".repeat(previous.max(required));
        let pad = new_text.starts_with('`')
            || new_text.ends_with('`')
            || new_text.starts_with(' ')
                && new_text.ends_with(' ')
                && !new_text.trim_matches(' ').is_empty();
        let text = format!(
            "{fence}{}{new_text}{}{fence}",
            if pad { " " } else { "" },
            if pad { " " } else { "" }
        );
        if let Some(edit) = verified(verify, block, range, text) {
            return Some(edit);
        }
    }
    None
}

fn image(
    block: &Block,
    next: &Node,
    source: &str,
    verify: &Verify<'_>,
) -> Option<(Range<usize>, String)> {
    fn changed<'a>(
        a: &'a Node,
        z: &'a Node,
        at: usize,
        out: &mut Vec<(usize, &'a Node)>,
    ) -> Option<()> {
        if a == z {
            return Some(());
        }
        if a.ty().name() == "image" && z.ty().name() == "image" && a.marks() == z.marks() {
            out.push((at, z));
            return Some(());
        }
        if !a.same_markup(z) || a.child_count() != z.child_count() {
            return None;
        }
        let mut offset = at + 1;
        for i in 0..a.child_count() {
            changed(a.child(i), z.child(i), offset, out)?;
            offset += a.child(i).node_size();
        }
        Some(())
    }
    let mut changes = Vec::new();
    changed(&block.node, next, 0, &mut changes)?;
    let [(pos, node)] = changes.as_slice() else {
        return None;
    };
    let ast = markdown::to_mdast(
        source.get(block.source.clone())?,
        &markdown::ParseOptions::gfm(),
    )
    .ok()?;
    fn images<'a>(node: &'a markdown::mdast::Node, out: &mut Vec<&'a markdown::mdast::Image>) {
        if let markdown::mdast::Node::Image(image) = node {
            out.push(image);
        }
        if let Some(children) = node.children() {
            for child in children {
                images(child, out);
            }
        }
    }
    let mut nodes = Vec::new();
    images(&ast, &mut nodes);
    for image in nodes {
        let p = image.position.as_ref()?;
        let range = block.source.start + p.start.offset..block.source.start + p.end.offset;
        if !block.text_map.source_positions(*pos).contains(&range.start)
            || !block
                .text_map
                .source_positions(pos + 1)
                .contains(&range.end)
        {
            continue;
        }
        let text = nsh_base::rich_editor::formats::MarkdownFormat::default().serialize_fragment(
            &nsh_base::rich_editor::model::Fragment::from(vec![(*node).clone()]),
        );
        if let Some(edit) = verified(verify, block, range, text) {
            return Some(edit);
        }
    }
    None
}

fn format(
    block: &Block,
    next: &Node,
    source: &str,
    verify: &Verify<'_>,
) -> Option<(Range<usize>, String)> {
    let old = marked_chars(&block.node);
    let new = marked_chars(next);
    if old.len() != new.len() || old.iter().zip(&new).any(|(a, z)| a.0 != z.0 || a.1 != z.1) {
        return None;
    }
    let changed: Vec<_> = old.iter().zip(&new).filter(|(a, z)| a.2 != z.2).collect();
    let &(first, first_next) = changed.first()?;
    let (added, removed): (Vec<_>, Vec<_>) = (
        first_next
            .2
            .iter()
            .filter(|m| !first.2.contains(m))
            .collect(),
        first
            .2
            .iter()
            .filter(|m| !first_next.2.contains(m))
            .collect(),
    );
    let (mark, add) = match (added.as_slice(), removed.as_slice()) {
        ([m], []) => (*m, true),
        ([], [m]) => (*m, false),
        _ => return None,
    };
    if changed.iter().any(|(a, z)| {
        let mut expected = a.2.clone();
        if add {
            expected = mark.add_to_set(&expected);
        } else {
            expected.retain(|m| m != mark);
        }
        expected != z.2
    }) {
        return None;
    }
    let &(last, _) = changed.last()?;
    let from = first.0;
    let to = last.0 + last.1.len_utf16();
    // Do not include an unmodified run in the middle of a format operation.
    if old
        .iter()
        .zip(&new)
        .any(|(a, z)| a.0 >= from && a.0 < to && a.2 == z.2)
    {
        return None;
    }
    let delimiters: &[&str] = match mark.ty.name() {
        "bold" => &["**", "__"],
        "italic" => &["*", "_"],
        "strike" => &["~~"],
        "code" => &["`"],
        _ => return None,
    };
    for a in block.text_map.source_positions(from) {
        for z in block.text_map.source_positions(to) {
            if a > z {
                continue;
            }
            for &delimiter in delimiters {
                if add {
                    let text = if mark.ty.name() == "code" {
                        let visible: String = old
                            .iter()
                            .filter(|c| c.0 >= from && c.0 < to)
                            .map(|c| c.1)
                            .collect();
                        let run = visible.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1;
                        let fence = "`".repeat(run);
                        let pad = visible.starts_with('`')
                            || visible.ends_with('`')
                            || visible.starts_with(' ')
                                && visible.ends_with(' ')
                                && !visible.trim_matches(' ').is_empty();
                        format!(
                            "{fence}{}{visible}{}{fence}",
                            if pad { " " } else { "" },
                            if pad { " " } else { "" }
                        )
                    } else {
                        format!("{delimiter}{}{delimiter}", source.get(a..z)?)
                    };
                    if let Some(edit) = verified(verify, block, a..z, text) {
                        return Some(edit);
                    }
                } else if a >= delimiter.len()
                    && source.get(a - delimiter.len()..a) == Some(delimiter)
                    && source.get(z..z + delimiter.len()) == Some(delimiter)
                {
                    if let Some(edit) = verified(
                        verify,
                        block,
                        a - delimiter.len()..z + delimiter.len(),
                        source.get(a..z)?.into(),
                    ) {
                        return Some(edit);
                    }
                } else if !add && mark.ty.name() != "code" {
                    let left = a >= delimiter.len()
                        && source.get(a - delimiter.len()..a) == Some(delimiter);
                    let right = source.get(z..z + delimiter.len()) == Some(delimiter);
                    let range = if left { a - delimiter.len() } else { a }..if right {
                        z + delimiter.len()
                    } else {
                        z
                    };
                    let text = format!(
                        "{}{}{}",
                        if left { "" } else { delimiter },
                        source.get(a..z)?,
                        if right { "" } else { delimiter }
                    );
                    if let Some(edit) = verified(verify, block, range, text) {
                        return Some(edit);
                    }
                }
            }
        }
    }
    None
}

pub(super) fn block_patch(
    projection: &Projection,
    index: usize,
    next: &Node,
) -> Option<(Range<usize>, String)> {
    let block = projection.blocks.get(index)?;
    let target = next.child(index);
    let verify = Verify { projection, next };
    let source = projection.source.as_str();
    heading(block, target, source, &verify)
        .or_else(|| text_leaf(block, target, &verify))
        .or_else(|| inline_code(block, target, source, &verify))
        .or_else(|| image(block, target, source, &verify))
        .or_else(|| format(block, target, source, &verify))
}
