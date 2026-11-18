//! Recover a literal leading hash sequence excluded before GFM's first Data
//! event. Original source and source coordinates never pass through an escape.
use markdown::{mdast, unist::Position};
use nsh_base::rich_editor::{
    formats::{DocFormat, MarkdownFormat},
    model::{Node, Schema},
};

pub(super) struct HeadingImport {
    pub ast: mdast::Node,
    escape: usize,
}
pub(super) fn prepare(node: &mdast::Node, source: &str) -> Option<HeadingImport> {
    let mdast::Node::Heading(heading) = node else {
        return None;
    };
    let span = heading.position.as_ref()?;
    let raw = source.get(span.start.offset..span.end.offset)?;
    let indent = raw.bytes().take_while(|b| *b == b' ').count();
    let run = raw
        .get(indent..)?
        .bytes()
        .take_while(|b| *b == b'#')
        .count();
    if !(1..=6).contains(&run)
        || run != heading.depth as usize
        || !matches!(raw.as_bytes().get(indent + run), Some(b' ' | b'\t'))
    {
        return None;
    }
    let after = indent + run;
    let padding = raw[after..]
        .bytes()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count();
    let body = span.start.offset + after + padding;
    let first = heading.children.first()?.position()?.start.clone();
    if first.line != span.start.line || first.offset <= body {
        return None;
    }
    let missing = source.get(body..first.offset)?;
    let hashes = missing.bytes().take_while(|b| *b == b'#').count();
    if hashes == 0
        || !missing[hashes..]
            .bytes()
            .all(|b| matches!(b, b'#' | b' ' | b'\t'))
    {
        return None;
    }
    // A real following inline node distinguishes a literal prefix from an
    // empty heading's closing fence. Add only the AST-owned omitted source.
    let mut start = span.start.clone();
    start.column += body - span.start.offset;
    start.offset = body;
    let text = mdast::Node::Text(mdast::Text {
        value: missing.to_owned(),
        position: Some(Position { start, end: first }),
    });
    let mut ast = node.clone();
    ast.children_mut()?.insert(0, text);
    Some(HeadingImport { ast, escape: body })
}
impl HeadingImport {
    pub fn import(
        &self,
        schema: &Schema,
        source: &str,
        definitions: &[&mdast::Definition],
    ) -> Option<Node> {
        let span = self.ast.position()?;
        let mut input = super::references::import_source(&self.ast, source, definitions)?;
        let relative = self.escape.checked_sub(span.start.offset)?;
        if input.as_bytes().get(relative) != Some(&b'#') {
            return None;
        }
        // The one temporary escape makes the parser start Data at the actual
        // body prefix. It is decoded by the normal inline reader, not stripped
        // by a global text search. SourceMap still consumes self.ast + source.
        input.insert(relative, '\\');
        let imported = MarkdownFormat::default().parse(schema, &input).ok()?;
        if imported.child_count() != 1 || imported.child(0).ty().name() != "heading" {
            return None;
        }
        Some(imported)
    }
}
