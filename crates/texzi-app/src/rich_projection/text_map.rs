//! Bind visible text to parser-owned source spans, including escapes/entities.
//! Text identity comes from AST order, never from a substring search.
use markdown::{ParseOptions, mdast};
use nsh_base::rich_editor::model::Node;
use std::ops::Range;

#[derive(Clone, Debug)]
pub(super) struct Unit {
    pub rich: Range<usize>,
    pub source: Range<usize>,
}

#[derive(Clone, Default)]
pub(super) struct TextMap {
    pub units: Vec<Unit>,
}

struct Atom {
    text: String,
    units: Vec<(Range<usize>, Range<usize>)>,
}

fn entity(raw: &str) -> Option<String> {
    let ast = markdown::to_mdast(raw, &ParseOptions::gfm()).ok()?;
    let [mdast::Node::Paragraph(p)] = ast.children()?.as_slice() else {
        return None;
    };
    let [mdast::Node::Text(t)] = p.children.as_slice() else {
        return None;
    };
    (t.value != raw).then(|| t.value.clone())
}

// A token owns all of its original bytes. An entity that expands to several
// scalars has only two exact source boundaries, not invented interior offsets.
fn decoded_atom(
    raw: &str,
    expected: &str,
    start: usize,
    code: bool,
    table_code_pipe: bool,
) -> Option<Atom> {
    let mut units = Vec::new();
    let (mut a, mut b) = (0, 0);
    while a < raw.len() && b < expected.len() {
        let tail = &raw[a..];
        let (length, value) = if table_code_pipe && tail.starts_with("\\|") {
            // GFM removes a pipe's escape before interpreting a code span in
            // a table cell. This exception does not apply to ordinary code.
            (2, "|".into())
        } else if !code
            && tail.starts_with('\\')
            && tail.as_bytes().get(1).is_some_and(u8::is_ascii_punctuation)
        {
            (2, tail[1..2].to_owned())
        } else if !code && tail.starts_with('&') {
            tail.find(';')
                .filter(|at| *at <= 64)
                .and_then(|at| entity(&tail[..=at]).map(|v| (at + 1, v)))
                .unwrap_or_else(|| (1, "&".into()))
        } else if tail.starts_with("\r\n") && !expected[b..].starts_with("\r\n") {
            (2, "\n".into())
        } else {
            let ch = tail.chars().next()?;
            (ch.len_utf8(), ch.to_string())
        };
        if expected[b..].starts_with(&value) {
            units.push((b..b + value.len(), start + a..start + a + length));
            b += value.len();
            a += length;
            continue;
        }
        // mdast removes quote/list continuation prefixes. Map each remaining
        // line independently; never attach those structural bytes to text.
        if a == 0 || raw[..a].ends_with('\n') {
            let line_end = tail.find('\n').unwrap_or(tail.len());
            let mut found = None;
            for (skip, _) in tail[..line_end].char_indices() {
                if !tail[..skip]
                    .bytes()
                    .all(|b| matches!(b, b' ' | b'\t' | b'>'))
                {
                    break;
                }
                if skip > 0 && expected[b..].starts_with(tail[skip..].chars().next()?) {
                    if let Some(rest) = decoded_atom(
                        &tail[skip..],
                        &expected[b..],
                        start + a + skip,
                        code,
                        table_code_pipe,
                    ) {
                        found = Some((skip, rest));
                        break;
                    }
                }
            }
            if let Some((_, rest)) = found {
                units.extend(
                    rest.units
                        .into_iter()
                        .map(|(r, s)| (b + r.start..b + r.end, s)),
                );
                return Some(Atom {
                    text: expected.into(),
                    units,
                });
            }
        }
        return None;
    }
    (a == raw.len() && b == expected.len()).then(|| Atom {
        text: expected.into(),
        units,
    })
}

fn atoms(node: &mdast::Node, source: &str, out: &mut Vec<Atom>, in_table_cell: bool) -> Option<()> {
    let p = node.position();
    let atom = match node {
        mdast::Node::Text(t) => {
            let p = p?;
            Some(decoded_atom(
                source.get(p.start.offset..p.end.offset)?,
                &t.value,
                p.start.offset,
                false,
                false,
            )?)
        }
        mdast::Node::InlineCode(c) => {
            let p = p?;
            let raw = source.get(p.start.offset..p.end.offset)?;
            let run = raw.bytes().take_while(|b| *b == b'`').count();
            let body = raw.get(run..raw.len().checked_sub(run)?)?;
            // CommonMark strips one surrounding space if the code is not all
            // spaces. Its line endings become spaces inside an inline span.
            let normalized = body.replace("\r\n", " ").replace(['\n', '\r'], " ");
            let trim = normalized.starts_with(' ')
                && normalized.ends_with(' ')
                && normalized.trim_matches(' ').len() > 0;
            let (body, at) = if trim {
                (
                    body.get(1..body.len().checked_sub(1)?)?,
                    p.start.offset + run + 1,
                )
            } else {
                (body, p.start.offset + run)
            };
            let mut atom = decoded_atom(body, &c.value, at, true, in_table_cell);
            if atom.is_none() && !body.contains('\r') {
                let flattened = body.replace('\n', " ");
                atom = decoded_atom(&flattened, &c.value, at, true, in_table_cell);
            }
            Some(atom?)
        }
        mdast::Node::Code(c) => {
            let p = p?;
            let raw = source.get(p.start.offset..p.end.offset)?;
            let first = raw.split('\n').next()?;
            let trimmed = first.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                let body_start = raw.find('\n')? + 1;
                let last_line = raw.rfind('\n').map_or(body_start, |at| at + 1);
                let closing = raw[last_line..].trim_start_matches([' ', '\t', '>']);
                let marker = trimmed.as_bytes()[0];
                let run = trimmed.bytes().take_while(|b| *b == marker).count();
                let closing_run = closing.bytes().take_while(|b| *b == marker).count();
                let closed = closing_run >= run && closing[closing_run..].trim().is_empty();
                let body_end = if closed {
                    last_line.saturating_sub(1)
                } else {
                    raw.len()
                };
                let body = raw.get(body_start..body_end.max(body_start))?;
                let body = body.strip_suffix('\r').unwrap_or(body);
                let at = p.start.offset + body_start;
                Some(decoded_atom(body, &c.value, at, true, false)?)
            } else {
                // Indented code owns only the content after each line's
                // indentation. Bind lines independently to retain indentation.
                let mut units = Vec::new();
                let (mut offset, mut visible) = (p.start.offset, 0);
                for line in raw.split_inclusive('\n') {
                    let indent = if line.starts_with('\t') {
                        1
                    } else {
                        line.bytes().take_while(|b| *b == b' ').count().min(4)
                    };
                    let body = &line[indent..];
                    let remaining = &c.value[visible..];
                    let length = body.len().min(remaining.len());
                    let atom = decoded_atom(
                        &body[..length],
                        &remaining[..length],
                        offset + indent,
                        true,
                        false,
                    )?;
                    units.extend(
                        atom.units
                            .into_iter()
                            .map(|(r, s)| (visible + r.start..visible + r.end, s)),
                    );
                    visible += length;
                    offset += line.len();
                    if visible == c.value.len() {
                        break;
                    }
                }
                (visible == c.value.len()).then(|| Atom {
                    text: c.value.clone(),
                    units,
                })
            }
        }
        // Images and hard breaks have rich node positions but no text leaf.
        mdast::Node::Image(_) | mdast::Node::Break(_) => None,
        _ => {
            if let Some(children) = node.children() {
                for child in children {
                    atoms(
                        child,
                        source,
                        out,
                        in_table_cell || matches!(node, mdast::Node::TableCell(_)),
                    )?;
                }
            }
            None
        }
    };
    if let Some(atom) = atom {
        out.push(atom);
    }
    Some(())
}

impl TextMap {
    pub(super) fn shift_source(&mut self, bytes: usize) {
        for unit in &mut self.units {
            unit.source.start += bytes;
            unit.source.end += bytes;
        }
    }

    pub fn build(
        node: &Node,
        ast: &mdast::Node,
        source: &str,
        opaque: bool,
        span: Range<usize>,
    ) -> Self {
        let mut parsed = Vec::new();
        if opaque {
            let raw = &source[span.clone()];
            if let Some(atom) = decoded_atom(raw, raw, span.start, true, false) {
                parsed.push(atom);
            }
        } else if atoms(ast, source, &mut parsed, false).is_none() {
            return Self::default();
        }
        let visible: String = parsed.iter().map(|a| a.text.as_str()).collect();
        let mut leaves = Vec::new();
        node.descendants(&mut |n, at, _, _| {
            if let Some(text) = n.text() {
                leaves.push((at + 1, text.to_owned()));
            }
            true
        });
        if leaves.iter().map(|(_, t)| t.as_str()).collect::<String>() != visible {
            return Self::default();
        }
        let mut source_units = Vec::new();
        let mut before = 0;
        for atom in parsed {
            for (r, source) in atom.units {
                source_units.push((before + r.start..before + r.end, source));
            }
            before += atom.text.len();
        }
        let mut units = Vec::new();
        let mut source_units = source_units.into_iter().peekable();
        let mut before = 0;
        for (at, text) in leaves {
            let end = before + text.len();
            let mut byte = 0;
            let mut rich = at;
            // Text leaves and decoded units both follow visible byte order.
            // Visit each unit once and count UTF-16 only over new bytes.
            while source_units.peek().is_some_and(|(r, _)| r.start < end) {
                let (r, source) = source_units.next().unwrap();
                if r.end > end {
                    // An entity spanning leaves has no exact per-leaf mapping.
                    continue;
                }
                let start = r.start - before;
                let stop = r.end - before;
                rich += text[byte..start].encode_utf16().count();
                let rich_start = rich;
                rich += text[start..stop].encode_utf16().count();
                units.push(Unit {
                    rich: rich_start..rich,
                    source,
                });
                byte = stop;
            }
            before = end;
        }
        fn inline_atoms(node: &mdast::Node, out: &mut Vec<(&'static str, Range<usize>)>) {
            let kind = match node {
                mdast::Node::Image(_) | mdast::Node::ImageReference(_) => Some("image"),
                mdast::Node::Break(_) => Some("hardBreak"),
                _ => None,
            };
            if let (Some(kind), Some(p)) = (kind, node.position()) {
                out.push((kind, p.start.offset..p.end.offset));
            }
            if let Some(children) = node.children() {
                for child in children {
                    inline_atoms(child, out);
                }
            }
        }
        if !opaque {
            let mut ast_atoms = Vec::new();
            inline_atoms(ast, &mut ast_atoms);
            let mut rich_atoms = Vec::new();
            node.descendants(&mut |n, at, _, _| {
                if matches!(n.ty().name(), "image" | "hardBreak") {
                    rich_atoms.push((n.ty().name().to_owned(), at + 1..at + 1 + n.node_size()));
                }
                true
            });
            if ast_atoms.len() == rich_atoms.len()
                && ast_atoms.iter().zip(&rich_atoms).all(|(a, z)| a.0 == z.0)
            {
                units.extend(
                    ast_atoms
                        .into_iter()
                        .zip(rich_atoms)
                        .map(|((_, source), (_, rich))| Unit { rich, source }),
                );
            }
        }
        Self { units }
    }

    pub fn source_positions(&self, rich: usize) -> Vec<usize> {
        let mut out = Vec::new();
        for unit in &self.units {
            if unit.rich.start == rich && !out.contains(&unit.source.start) {
                out.push(unit.source.start);
            }
            if unit.rich.end == rich && !out.contains(&unit.source.end) {
                out.push(unit.source.end);
            }
        }
        out
    }

    pub fn rich_position(&self, source: usize) -> Option<usize> {
        self.units.iter().rev().find_map(|u| {
            if source == u.source.start {
                Some(u.rich.start)
            } else if source == u.source.end {
                Some(u.rich.end)
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        editor_owner::NativeView,
        rich_owner::ModeGate,
        rich_projection::{Projection, manager},
    };
    use nsh_base::{editor::ExternalDocument, rich_editor::state::TextSelection};
    use nsh_core::prelude::*;

    #[test]
    fn long_mixed_paragraph_keeps_utf16_mapping_across_marked_leaves() {
        let manager = manager();
        let prefix = "中a😀".repeat(8192);
        let raw = format!("{prefix} **文😀** &amp; 尾");
        let projection = Projection::parse(manager.schema(), raw.clone()).unwrap();

        let marked_byte = prefix.len() + " **".len();
        let marked_position = 1 + 8192 * 4 + 1;
        assert_eq!(projection.rich_position(marked_byte), Some(marked_position));
        assert_eq!(
            projection.source_position(marked_position + 1),
            Some(marked_byte + "文".len())
        );
        let entity = raw.find("&amp;").unwrap();
        let rich = projection.rich_position(entity).unwrap();
        assert_eq!(projection.source_position(rich + 1), Some(entity + 5));
        assert_eq!(projection.rich_position(entity + 2), None);
    }

    #[test]
    fn gfm_code_cell_escaped_pipe_has_fixed_source_and_utf16_boundaries() {
        let manager = manager();
        let raw = "| h | v |\n| --- | --- |\n| `a\\|b` | x |";
        let p = Projection::parse(manager.schema(), raw.into()).unwrap();
        assert_eq!(p.doc.child(0).ty().name(), "table");
        assert_eq!(p.doc.child(0).child(1).child(0).text_content(), "a|b");
        // Fixed byte positions, independent of the source mapper: third line
        // starts at byte 24, code content at 27, and \| owns bytes 28..30.
        assert_eq!(raw.get(27..31), Some("a\\|b"));
        for (source, rich) in [
            (2, 4),
            (6, 9),
            (27, 16),
            (28, 17),
            (30, 18),
            (31, 19),
            (35, 23),
        ] {
            assert_eq!(p.rich_position(source), Some(rich), "source {source}");
            assert_eq!(p.source_position(rich), Some(source), "rich {rich}");
        }
        assert_eq!(
            p.rich_position(29),
            None,
            "no invented boundary inside the source escape"
        );
    }

    #[test]
    fn ordinary_code_keeps_backslash_pipe_and_table_code_keeps_other_literal_escapes() {
        let manager = manager();
        let ordinary = Projection::parse(manager.schema(), "`a\\|b`".into()).unwrap();
        assert_eq!(ordinary.doc.child(0).text_content(), "a\\|b");
        for (source, rich) in [(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)] {
            assert_eq!(ordinary.rich_position(source), Some(rich));
            assert_eq!(ordinary.source_position(rich), Some(source));
        }
        let raw = "| h |\n| --- |\n| `a\\|b &amp; \\*x\\*` |";
        let table = Projection::parse(manager.schema(), raw.into()).unwrap();
        assert_eq!(
            table.doc.child(0).child(1).child(0).text_content(),
            "a|b &amp; \\*x\\*"
        );
        let literal_amp = raw.find("&amp;").unwrap();
        let pos = table.rich_position(literal_amp).unwrap();
        assert_eq!(
            table.source_position(pos + 1),
            Some(literal_amp + 1),
            "entity spelling remains literal in code"
        );
        let literal_slash = raw.find("\\*x").unwrap();
        let pos = table.rich_position(literal_slash).unwrap();
        assert_eq!(
            table.source_position(pos + 1),
            Some(literal_slash + 1),
            "only a table pipe loses its escape"
        );
    }

    #[test]
    fn owner_gfm_code_cell_edit_patches_escape_only_and_shared_undo_restores_raw_eols() {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let raw = "| h | v |\r\n| --- | --- |\n| `a\\|b` | x |\r\n\r\n<custom>keep</custom>\n";
        let source = NativeView::fixture(raw);
        let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
        let before =
            source.inspect(|store, id| store.snapshot(store.view(id).unwrap().doc).unwrap().token);
        gate.to_rich();
        assert!(gate.rich_mode.get(), "{}", gate.status.get());
        let session = gate.session();
        let state = session.state();
        let mut select = state.tr();
        select.set_selection(TextSelection::create(state.doc(), 17, Some(18)));
        session.dispatch(select).unwrap();
        assert!(gate.status.get().is_empty(), "{}", gate.status.get());
        let mut tr = session.state().tr();
        tr.insert_text("/", Some(17), Some(18));
        session.dispatch(tr).unwrap();
        assert!(gate.status.get().is_empty(), "{}", gate.status.get());
        let expected = raw.replacen("\\|", "/", 1);
        source.inspect(|store, id| {
            let doc = store.view(id).unwrap().doc;
            let snapshot = store.snapshot(doc).unwrap();
            assert_eq!(snapshot.body.to_utf8(), expected.as_bytes());
            assert_eq!(store.history_depth(doc).unwrap(), (1, 0));
            assert!(store.dirty(doc).unwrap());
            let retained: usize = snapshot
                .body
                .origins()
                .into_iter()
                .filter_map(|(_, origin)| origin.map(|origin| origin.raw.len()))
                .sum();
            assert_eq!(
                retained,
                raw.len() - 2,
                "every byte outside the replaced \\| keeps its original provenance"
            );
        });
        gate.to_source();
        assert!(!gate.rich_mode.get(), "{}", gate.status.get());
        gate.shell_history(true).unwrap();
        source.inspect(|store, id| {
            let doc = store.view(id).unwrap().doc;
            let snapshot = store.snapshot(doc).unwrap();
            assert_eq!(snapshot.body.to_utf8(), raw.as_bytes());
            assert_eq!(snapshot.token, before);
            assert!(!store.dirty(doc).unwrap());
            assert_eq!(store.history_depth(doc).unwrap(), (0, 1));
        });
        gate.shell_history(false).unwrap();
        assert_eq!(
            source.inspect(|store, id| store
                .snapshot(store.view(id).unwrap().doc)
                .unwrap()
                .body
                .to_utf8()),
            expected.as_bytes()
        );
    }
}
