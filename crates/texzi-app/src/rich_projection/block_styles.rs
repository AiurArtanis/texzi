//! Style-only rewrites keep inline spelling and newline provenance outside
//! the actual Markdown delimiters. A complete reparse validates every plan.
use super::Projection;
use nsh_base::rich_editor::model::Node;
use std::ops::Range;

type Edits = Vec<(Range<usize>, String)>;

pub(super) fn edits(p: &Projection, next: &Node) -> Result<Option<Edits>, String> {
    if p.doc.child_count() != next.child_count() {
        return Ok(None);
    }
    let changed: Vec<_> = (0..p.doc.child_count())
        .filter(|&i| p.doc.child(i) != next.child(i))
        .collect();
    if changed.len() != 1 {
        return Ok(None);
    }
    let index = changed[0];
    let block = &p.blocks[index];
    let old = &block.node;
    let new = next.child(index);
    let (a, z) = (old.ty().name(), new.ty().name());
    if block.opaque
        || !matches!(a, "paragraph" | "heading")
        || !matches!(z, "paragraph" | "heading")
        || a == "paragraph" && z == "paragraph"
        || old.content() != new.content()
    {
        return Ok(None);
    }
    let level = if z == "heading" {
        new.attrs()
            .get("level")
            .and_then(|v| v.as_u64())
            .filter(|n| (1..=6).contains(n))
            .ok_or("标题级别必须是 H1–H6")? as usize
    } else {
        0
    };
    let raw = &p.source[block.source.clone()];
    let indent = raw.bytes().take_while(|b| *b == b' ').count();
    if indent > 3 {
        return Err("该块缩进不能可靠转换段落样式，原文未变".into());
    }
    let first_end = raw.find('\n').unwrap_or(raw.len());
    let first = &raw[..first_end];
    let run = first[indent..].bytes().take_while(|b| *b == b'#').count();
    let atx = a == "heading"
        && (1..=6).contains(&run)
        && first
            .as_bytes()
            .get(indent + run)
            .is_none_or(|b| matches!(b, b' ' | b'\t'));
    let start = block.source.start;
    let mut plan: Edits = vec![];
    if a == "paragraph" {
        plan.push((
            start + indent..start + indent,
            format!("{} ", "#".repeat(level)),
        ));
    } else if atx {
        let marker_end = indent + run;
        if z == "heading" {
            plan.push((start + indent..start + marker_end, "#".repeat(level)));
        } else {
            let body_start = marker_end
                + first[marker_end..]
                    .bytes()
                    .take_while(|b| matches!(b, b' ' | b'\t'))
                    .count();
            plan.push((start + indent..start + body_start, String::new()));
            let content_end = first.trim_end_matches([' ', '\t']).len();
            let closing = first[..content_end]
                .bytes()
                .rev()
                .take_while(|b| *b == b'#')
                .count();
            let closing_start = content_end - closing;
            if closing > 0
                && closing_start >= body_start
                && matches!(first.as_bytes()[closing_start - 1], b' ' | b'\t')
            {
                let mut remove = closing_start;
                while remove > body_start && matches!(first.as_bytes()[remove - 1], b' ' | b'\t') {
                    remove -= 1;
                }
                plan.push((start + remove..start + first.len(), String::new()));
            }
        }
    } else {
        // The parser owns this Setext block. Keep its inline source untouched;
        // changing to ATX/P removes only the internal underline separator. The
        // original terminal EOL lies outside block.source and is never reinput.
        let at = raw
            .rfind('\n')
            .ok_or("该标题缺少可核对的语法边界，原文未变")?;
        let underline = &raw[at + 1..];
        let pad = underline.bytes().take_while(|b| *b == b' ').count();
        let marker = underline[pad..].trim_end_matches([' ', '\t']);
        if pad > 3
            || marker.is_empty()
            || !(marker.bytes().all(|b| b == b'=') || marker.bytes().all(|b| b == b'-'))
        {
            return Err("该标题的下划线边界不能可靠核对，原文未变".into());
        }
        if z == "heading" && matches!(level, 1 | 2) {
            plan.push((
                start + at + 1 + pad..start + at + 1 + pad + marker.len(),
                if level == 1 { "=" } else { "-" }.repeat(marker.len()),
            ));
        } else {
            plan.push((start + at..block.source.end, String::new()));
            if z == "heading" {
                plan.push((
                    start + indent..start + indent,
                    format!("{} ", "#".repeat(level)),
                ));
            }
        }
    }
    plan.retain(|(range, text)| p.source.get(range.clone()) != Some(text.as_str()));
    plan.sort_by_key(|(range, _)| (range.start, range.end));
    let mut candidate = p.source.clone();
    for (range, text) in plan.iter().rev() {
        candidate.replace_range(range.clone(), text);
    }
    if !Projection::parse(next.ty().schema(), candidate).is_ok_and(|parsed| &parsed.doc == next) {
        return Err("该段落样式暂不能保真往返，原文未变；可使用源码编辑".into());
    }
    Ok(Some(plan))
}
