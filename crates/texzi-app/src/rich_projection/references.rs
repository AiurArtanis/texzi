//! Resolve reference resources only in the disposable import input. Original
//! reference spelling, labels and definitions remain authoritative source.
use markdown::mdast;

pub(super) fn import_source(
    node: &mdast::Node,
    source: &str,
    definitions: &[&mdast::Definition],
) -> Option<String> {
    fn walk(
        node: &mdast::Node,
        source: &str,
        definitions: &[&mdast::Definition],
        edits: &mut Vec<(std::ops::Range<usize>, String)>,
    ) -> Option<()> {
        let reference = match node {
            mdast::Node::LinkReference(r) => {
                Some((&r.identifier, false, r.children.as_slice(), None))
            }
            mdast::Node::ImageReference(r) => {
                Some((&r.identifier, true, &[][..], Some(r.alt.as_str())))
            }
            _ => None,
        };
        if let Some((id, image, children, alt)) = reference {
            let def = definitions
                .iter()
                .find(|d| d.identifier.to_lowercase() == id.to_lowercase())?;
            let position = node.position()?;
            let label = if image {
                alt?.replace('\\', "\\\\")
                    .replace('[', "\\[")
                    .replace(']', "\\]")
            } else if let (Some(first), Some(last)) = (children.first(), children.last()) {
                source
                    .get(first.position()?.start.offset..last.position()?.end.offset)?
                    .into()
            } else {
                String::new()
            };
            let title = def
                .title
                .as_ref()
                .map(|t| format!(" \"{}\"", super::resources::title(t)))
                .unwrap_or_default();
            let url = super::resources::destination(&def.url)
                .replace('<', "%3C")
                .replace('>', "%3E");
            edits.push((
                position.start.offset..position.end.offset,
                format!("{}[{label}](<{url}>{title})", if image { "!" } else { "" }),
            ));
        } else if let Some(children) = node.children() {
            for child in children {
                walk(child, source, definitions, edits)?;
            }
        }
        Some(())
    }
    let p = node.position()?;
    let mut raw = source.get(p.start.offset..p.end.offset)?.to_owned();
    let mut edits = Vec::new();
    walk(node, source, definitions, &mut edits)?;
    for (range, text) in edits.into_iter().rev() {
        raw.replace_range(
            range.start - p.start.offset..range.end - p.start.offset,
            &text,
        );
    }
    Some(raw)
}
