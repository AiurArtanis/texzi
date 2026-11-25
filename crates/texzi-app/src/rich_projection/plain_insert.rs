use super::{Projection, SourceEdit, parse_options, text_map::TextMap};
use markdown::mdast;
use nsh_base::rich_editor::{
    formats::{DocFormat, MarkdownFormat},
    model::Node,
    transform::ReplaceStep,
};

fn literal_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || (!ch.is_ascii() && !ch.is_control())
}

fn strip_eol(text: &str) -> Option<&str> {
    text.strip_prefix("\r\n")
        .or_else(|| text.strip_prefix('\n'))
}

fn isolated_paragraph(before: &str, after: &str) -> bool {
    let starts_block = before.is_empty() || before.ends_with("\n\n") || before.ends_with("\n\r\n");
    starts_block
        && (after.is_empty()
            || strip_eol(after).is_some_and(|tail| tail.is_empty() || strip_eol(tail).is_some()))
}

/// Reparse only an isolated, literal top-level paragraph. Excluding ASCII
/// Markdown punctuation also rules out definitions, references and autolinks
/// that could depend on text beyond this paragraph.
pub(super) fn project(
    current: &Projection,
    next: &Node,
    step: &ReplaceStep,
) -> Option<(SourceEdit, Projection)> {
    if step.from != step.to
        || step.slice.content.child_count() != 1
        || next.child_count() != current.blocks.len()
        || !current.doc.same_markup(next)
    {
        return None;
    }
    let leaf = step.slice.content.child(0);
    let text = leaf.text()?;
    if !leaf.marks().is_empty()
        || text.is_empty()
        || !text
            .chars()
            .all(|ch| literal_char(ch) && !ch.is_whitespace())
    {
        return None;
    }

    let mut base = 0;
    let (index, offset, block) = current
        .blocks
        .iter()
        .enumerate()
        .find_map(|(index, block)| {
            let start = base;
            base += block.node.node_size();
            (start < step.from && step.from < base).then(|| (index, step.from - start, block))
        })?;
    if block.opaque || block.node.ty().name() != "paragraph" {
        return None;
    }
    let span = &block.source;
    let raw = current.source.get(span.clone())?;
    if !isolated_paragraph(&current.source[..span.start], &current.source[span.end..])
        || !raw.split('\n').all(|line| {
            line.strip_suffix('\r')
                .unwrap_or(line)
                .chars()
                .all(|ch| ch == ' ' || literal_char(ch))
        })
    {
        return None;
    }
    let positions = block.text_map.source_positions(offset);
    let [at] = positions.as_slice() else {
        return None;
    };
    let local = at.checked_sub(span.start)?;
    if !raw.is_char_boundary(local) {
        return None;
    }
    let mut paragraph = raw.to_owned();
    paragraph.insert_str(local, text);
    let ast = markdown::to_mdast(&paragraph, &parse_options()).ok()?;
    let [plain @ mdast::Node::Paragraph(_)] = ast.children()?.as_slice() else {
        return None;
    };
    let position = plain.position()?;
    if position.start.offset != 0 || position.end.offset != paragraph.len() {
        return None;
    }
    let imported = MarkdownFormat::default()
        .parse(next.ty().schema(), &paragraph)
        .ok()?;
    if imported.child_count() != 1
        || imported.child(0) != next.child(index)
        || current
            .blocks
            .iter()
            .enumerate()
            .any(|(i, b)| i != index && &b.node != next.child(i))
    {
        return None;
    }
    let node = next.child(index);
    let mut map = TextMap::build(node, plain, &paragraph, false, 0..paragraph.len());
    if map.rich_position(local + text.len()) != Some(offset + text.encode_utf16().count()) {
        return None;
    }
    map.shift_source(span.start);
    let mut blocks = current.blocks.clone();
    blocks[index].node = node.clone();
    blocks[index].source.end += text.len();
    blocks[index].text_map = map;
    for later in &mut blocks[index + 1..] {
        later.source.start += text.len();
        later.source.end += text.len();
        later.text_map.shift_source(text.len());
    }
    let mut source = current.source.clone();
    source.insert_str(*at, text);
    if std::env::var_os("TEXZI_TRACE_IME").as_deref() == Some(std::ffi::OsStr::new("1")) {
        log::info!("RICH_PROJECTION action=local_insert bytes={}", text.len());
    }
    Some((
        (*at..*at, text.into()),
        Projection {
            source,
            doc: next.clone(),
            blocks,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_base::rich_editor::state::{EditorState, EditorStateConfig};

    #[test]
    fn literal_insert_matches_full_projection_at_chinese_and_emoji_boundaries() {
        let schema = super::super::manager().schema().clone();
        let raw = "　　正文😀\n　　第二行。\n\n后段🙂\n\n[保留][ref]\n\n[ref]: /old\n";
        let first_end = raw.find("\n\n").unwrap();
        for (at, inserted) in [(0, "汉"), ("　　正文😀".len(), "🙂"), (first_end, "尾")] {
            let current = Projection::parse(&schema, raw.into()).unwrap();
            let pos = current.rich_position(at).unwrap();
            let state = EditorState::create(
                EditorStateConfig::new(schema.clone()).doc(current.doc.clone()),
            )
            .unwrap();
            let mut tr = state.tr();
            tr.insert_text(inserted, Some(pos), Some(pos));
            let step = tr.steps[0].as_any().downcast_ref::<ReplaceStep>().unwrap();
            let (edit, updated) = project(&current, tr.doc(), step).expect("isolated paragraph");
            assert_eq!(edit, (at..at, inserted.into()));
            let mut expected = raw.to_owned();
            expected.insert_str(at, inserted);
            let full = Projection::parse(&schema, expected.clone()).unwrap();
            assert_eq!(updated.source, expected);
            assert_eq!(updated.doc, full.doc);
            let caret = pos + inserted.encode_utf16().count();
            assert_eq!(updated.source_position(caret), Some(at + inserted.len()));
            for (local, reparsed) in updated.blocks.iter().zip(&full.blocks) {
                assert_eq!(local.source, reparsed.source);
                assert_eq!(local.opaque, reparsed.opaque);
                assert!(
                    local
                        .text_map
                        .units
                        .iter()
                        .map(|unit| (&unit.rich, &unit.source))
                        .eq(reparsed
                            .text_map
                            .units
                            .iter()
                            .map(|unit| (&unit.rich, &unit.source)))
                );
            }
        }
    }

    #[test]
    fn markdown_syntax_insert_uses_full_projection() {
        let schema = super::super::manager().schema().clone();
        let current = Projection::parse(&schema, "正文\n\n后段".into()).unwrap();
        let state =
            EditorState::create(EditorStateConfig::new(schema.clone()).doc(current.doc.clone()))
                .unwrap();
        let mut tr = state.tr();
        tr.insert_text("# ", Some(1), Some(1));
        let step = tr.steps[0].as_any().downcast_ref::<ReplaceStep>().unwrap();
        assert!(project(&current, tr.doc(), step).is_none());
        let (edits, verified) = current
            .transaction_edits_with_projection(tr.doc(), std::slice::from_ref(&tr))
            .unwrap();
        let verified = verified.expect("full projection verified an escaped candidate");
        assert_eq!(edits, vec![(0..0, "\\# ".into())]);
        assert_eq!(verified.doc, *tr.doc());
    }
}
