//! Source-preserving first rich projection. Source bytes remain authoritative.
use markdown::{ParseOptions, mdast};
use nsh_base::rich_editor::{
    extensions::{ExtensionManager, kit},
    formats::{DocFormat, MarkdownFormat},
    model::{Fragment, Node, Schema},
    state::Transaction,
    transform::ReplaceStep,
};
use serde_json::json;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub fn manager() -> ExtensionManager {
    ExtensionManager::with_core(vec![
        kit::starter_kit()
            .configure(json!({"history":false,"trailingNode":false,"typography":false})),
        kit::table_kit(),
    ])
    .expect("fixed rich schema")
}

#[derive(Clone)]
pub struct Block {
    pub source: Range<usize>,
    pub node: Node,
    pub opaque: bool,
}
pub struct Projection {
    pub source: String,
    pub doc: Node,
    pub blocks: Vec<Block>,
}
fn message(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn root(schema: &Schema, nodes: Vec<Node>) -> Result<Node, String> {
    schema
        .node("doc", None, Fragment::from(nodes), vec![])
        .map_err(message)
}
impl Projection {
    pub fn rich_position(&self, byte: usize) -> Option<usize> {
        let mut base = 0;
        for block in &self.blocks {
            let mut found = None;
            block.node.descendants(&mut |n, p, _, _| {
                if let Some(t) = n.text() {
                    if let Some(at) = raw_text_start(block, t, &self.source) {
                        let start = block.source.start + at;
                        if let Some(delta) = byte
                            .checked_sub(start)
                            .filter(|d| *d <= t.len() && t.is_char_boundary(*d))
                        {
                            found = Some(base + 1 + p + t[..delta].encode_utf16().count());
                        }
                    }
                }
                true
            });
            if found.is_some() {
                return found;
            }
            if block.node.content().size() == 0 && Some(byte) == empty_position(block, &self.source)
            {
                return Some(base + 1);
            }
            base += block.node.node_size();
        }
        None
    }
    /// A single plain-text ReplaceStep supplies the identity of the edited
    /// source range. Validate the complete projection before preferring it to
    /// a structural serializer/diff; never infer newline identity from a diff.
    pub fn transaction_edits(
        &self,
        next: &Node,
        applied: &[Transaction],
    ) -> Result<Vec<(Range<usize>, String)>, String> {
        if &self.doc == next {
            return Ok(vec![]);
        }
        let steps: Vec<_> = applied.iter().flat_map(|t| t.steps.iter()).collect();
        if steps.len() == 1 {
            if let Some(step) = steps[0].as_any().downcast_ref::<ReplaceStep>() {
                if !step.structure
                    && step.slice.open_start == 0
                    && step.slice.open_end == 0
                    && (0..step.slice.content.child_count())
                        .all(|i| step.slice.content.child(i).is_text())
                {
                    if let (Some(a), Some(h)) = (
                        self.source_position(step.from),
                        self.source_position(step.to),
                    ) {
                        if a <= h {
                            let text: String = (0..step.slice.content.child_count())
                                .filter_map(|i| step.slice.content.child(i).text())
                                .collect();
                            let mut raw = self.source.clone();
                            raw.replace_range(a..h, &text);
                            if (a != h || !text.is_empty())
                                && Self::parse(next.ty().schema(), raw)
                                    .is_ok_and(|p| &p.doc == next)
                            {
                                return Ok(vec![(a..h, text)]);
                            }
                        }
                    }
                }
            }
        }
        self.source_edits(next)
    }
    /// Split a semantic block replacement into grapheme edits. Matched source
    /// spans (including every existing EOL) stay in Store's provenance tree.
    /// This first bridge rejects changes to line breaks inside multiline blocks
    /// rather than guessing which original newline an ambiguous diff denotes.
    pub fn source_edits(&self, next: &Node) -> Result<Vec<(Range<usize>, String)>, String> {
        let Some((range, text)) = self.patch(next)? else {
            return Ok(vec![]);
        };
        let before = &self.source[range.clone()];
        if before.len() > i32::MAX as usize || text.len() > i32::MAX as usize {
            return Err("源片段超过当前映射索引范围，原文未变".into());
        }
        let mut input = imara_diff::InternedInput::<&str>::default();
        input.update_before(before.graphemes(true));
        input.update_after(text.graphemes(true));
        let a: Vec<_> = before
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(before.len()))
            .collect();
        let b: Vec<_> = text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .collect();
        let diff = imara_diff::Diff::compute(imara_diff::Algorithm::Histogram, &input);
        let mut edits = Vec::new();
        for h in diff.hunks() {
            let old = a[h.before.start as usize]..a[h.before.end as usize];
            let new = b[h.after.start as usize]..b[h.after.end as usize];
            let simple_blocks = self
                .blocks
                .iter()
                .filter(|block| block.source.start < range.end && block.source.end > range.start)
                .all(|block| !block.opaque && !self.source[block.source.clone()].contains('\n'));
            if !simple_blocks
                && before.contains('\n')
                && (before[old.clone()].contains('\n') || text[new.clone()].contains('\n'))
            {
                return Err("该多行块的换行暂不能保真修改，请切到源码编辑；原文未变".into());
            }
            edits.push((
                range.start + old.start..range.start + old.end,
                text[new].to_owned(),
            ));
        }
        Ok(edits)
    }
    pub fn parse(schema: &Schema, source: String) -> Result<Self, String> {
        let tree = markdown::to_mdast(&source, &ParseOptions::gfm()).map_err(message)?;
        let mut blocks = Vec::new();
        for child in tree.children().ok_or("missing Markdown root")? {
            let p = child.position().ok_or("missing source span")?;
            let span = p.start.offset..p.end.offset;
            let raw = source
                .get(span.clone())
                .ok_or("invalid parser byte boundary")?;
            // Unsupported syntax is represented visibly as a local source block.
            // It is never silently discarded by the Markdown importer.
            let mut opaque = contains_opaque(child);
            let parsed = if opaque {
                None
            } else {
                MarkdownFormat::default().parse(schema, raw).ok()
            };
            let node = match parsed.filter(|d| d.child_count() == 1) {
                Some(d) => d.child(0).clone(),
                None => {
                    opaque = true;
                    schema
                        .node(
                            "codeBlock",
                            None,
                            Fragment::from(vec![schema.text(raw, vec![])]),
                            vec![],
                        )
                        .map_err(message)?
                }
            };
            let opaque = opaque
                || matches!(
                    child,
                    mdast::Node::Definition(_) | mdast::Node::FootnoteDefinition(_)
                );
            blocks.push(Block {
                source: span,
                node,
                opaque,
            });
        }
        // Markdown discards blank paragraphs. Project pairs of otherwise unused
        // LF separators as empty top-level paragraphs without changing source.
        // A normal single terminal LF remains trivia, not an extra paragraph.
        let mut spaced = Vec::new();
        let mut previous_end = 0;
        for block in blocks {
            let gap = &source[previous_end..block.source.start];
            let has_previous = !spaced.is_empty();
            let count = if gap.bytes().all(|b| b == b'\n') {
                gap.len() / 2 - usize::from(has_previous).min(gap.len() / 2)
            } else {
                0
            };
            for i in 0..count {
                let at = previous_end + 2 * (i + usize::from(has_previous));
                spaced.push(empty_block(schema, at)?);
            }
            previous_end = block.source.end;
            spaced.push(block);
        }
        let tail = &source[previous_end..];
        if spaced.is_empty() {
            spaced.push(empty_block(schema, 0)?);
        }
        if tail.bytes().all(|b| b == b'\n') {
            for i in 0..tail.len() / 2 {
                spaced.push(empty_block(schema, previous_end + 2 * (i + 1))?);
            }
        }
        let mut blocks = spaced;
        if blocks.is_empty() {
            let empty = schema
                .node("paragraph", None, Fragment::empty(), vec![])
                .map_err(message)?;
            blocks.push(Block {
                source: source.len()..source.len(),
                node: empty,
                opaque: false,
            });
        }
        let doc = root(schema, blocks.iter().map(|b| b.node.clone()).collect())?;
        doc.check().map_err(message)?;
        Ok(Self {
            source,
            doc,
            blocks,
        })
    }
    /// Strict mapping of text leaves and block edges; no UTF16/byte coercion.
    pub fn source_position(&self, pos: usize) -> Option<usize> {
        let mut base = 0;
        for b in &self.blocks {
            let size = b.node.node_size();
            if pos == base {
                return Some(b.source.start);
            }
            if pos == base + size {
                return Some(b.source.end);
            }
            if pos > base && pos < base + size {
                let mut found = None;
                b.node.descendants(&mut |n, p, _, _| {
                    if let Some(t) = n.text() {
                        let local = pos.checked_sub(base + 1 + p);
                        if let Some(units) = local.filter(|u| *u <= t.encode_utf16().count()) {
                            if let Some(at) = raw_text_start(b, t, &self.source) {
                                let mut used = 0;
                                let byte = t
                                    .char_indices()
                                    .find_map(|(i, c)| {
                                        if used == units {
                                            Some(i)
                                        } else {
                                            used += c.len_utf16();
                                            None
                                        }
                                    })
                                    .or_else(|| {
                                        (units == t.encode_utf16().count()).then_some(t.len())
                                    });
                                found = byte.map(|i| b.source.start + at + i);
                            }
                        }
                    }
                    true
                });
                if found.is_none() && b.node.content().size() == 0 && pos == base + 1 {
                    return empty_position(b, &self.source);
                }
                return found;
            }
            base += size;
        }
        None
    }
    /// Produce one local source replacement. No-op projections produce no patch.
    /// An opaque source block may only be edited as that same code block.
    pub fn patch(&self, next: &Node) -> Result<Option<(Range<usize>, String)>, String> {
        if &self.doc == next {
            return Ok(None);
        }
        let mut first = 0;
        while first < self.blocks.len()
            && first < next.child_count()
            && self.blocks[first].node == *next.child(first)
        {
            first += 1;
        }
        let mut old_end = self.blocks.len();
        let mut new_end = next.child_count();
        while old_end > first
            && new_end > first
            && self.blocks[old_end - 1].node == *next.child(new_end - 1)
        {
            old_end -= 1;
            new_end -= 1;
        }
        if self.blocks[first..old_end].iter().any(|b| b.opaque) {
            if old_end - first != 1
                || new_end - first != 1
                || next.child(first).ty().name() != "codeBlock"
            {
                return Err("该操作跨越未知语法；请切换源码编辑此范围".into());
            }
            return Ok(Some((
                self.blocks[first].source.clone(),
                next.child(first).text_content(),
            )));
        }
        // Text-only edits can preserve delimiters and trivia even inside the block.
        if old_end - first == 1 && new_end - first == 1 {
            if let Some(patch) =
                task_toggle_patch(&self.blocks[first], next.child(first), &self.source)?
            {
                return Ok(Some(patch));
            }
            if let Some((range, text)) =
                single_text_patch(&self.blocks[first], next.child(first), &self.source)
            {
                return Ok(Some((range, text)));
            }
        }
        let mut start = self
            .blocks
            .get(first)
            .map(|b| b.source.start)
            .unwrap_or(self.source.len());
        let mut end = if old_end > first {
            self.blocks[old_end - 1].source.end
        } else {
            start
        };
        // Removing a trailing empty paragraph also removes its separator pair.
        if new_end == first && old_end > first && start == end && first > 0 {
            let previous = self.blocks[first - 1].source.end;
            if start >= previous + 2 && &self.source[start - 2..start] == "\n\n" {
                start -= 2;
            }
        }
        if new_end == first && old_end > first && start == end && first == 0 {
            let following = self
                .blocks
                .get(old_end)
                .map(|b| b.source.start)
                .unwrap_or(self.source.len());
            if following >= end + 2 && &self.source[end..end + 2] == "\n\n" {
                end += 2;
            }
        }
        if self.source[start..end]
            .chars()
            .any(|c| c == '\\' || c == '&')
        {
            return Err("此块的转义或实体暂不能保真改写，请切到源码编辑；原文未变".into());
        }
        let mut pieces = Vec::new();
        for i in first..new_end {
            let node = next.child(i);
            let doc = root(&self.doc.ty().schema(), vec![node.clone()])?;
            let mut piece = MarkdownFormat::default().serialize(&doc).map_err(message)?;
            if piece.ends_with('\n') {
                piece.pop();
            }
            pieces.push(piece);
        }
        let mut text = pieces.join("\n\n");
        // Match the modified block's EOL convention. Unchanged source stays raw.
        if self.source[start..end].contains("\r\n") {
            text = text.replace('\n', "\r\n");
        }
        if old_end == first && first > 0 && first == self.blocks.len() {
            text = format!("\n\n{text}");
        }
        if old_end == first && first < self.blocks.len() {
            text.push_str("\n\n");
        }
        Ok(Some((start..end, text)))
    }
}

fn empty_block(schema: &Schema, at: usize) -> Result<Block, String> {
    Ok(Block {
        source: at..at,
        node: schema
            .node("paragraph", None, Fragment::empty(), vec![])
            .map_err(message)?,
        opaque: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_base::rich_editor::state::{EditorState, EditorStateConfig};
    fn edited(p: &Projection, from: usize, to: usize, text: &str) -> Node {
        let state = EditorState::create(
            EditorStateConfig::new(p.doc.ty().schema().clone()).doc(p.doc.clone()),
        )
        .unwrap();
        let mut tr = state.tr();
        tr.insert_text(text, Some(from), Some(to));
        state.apply(tr).unwrap().doc().clone()
    }
    fn apply(p: &Projection, n: &Node) -> String {
        let mut raw = p.source.clone();
        for (r, t) in p.source_edits(n).unwrap().iter().rev() {
            raw.replace_range(r.clone(), t);
        }
        raw
    }
    #[test]
    fn local_heading_edit_preserves_unknown_and_mixed_eol() {
        let m = manager();
        let raw = "# 甲😀\r\n\r\n<div data-unknown='x'>KEEP</div>\n\n- [x] 保留\r\n";
        let p = Projection::parse(m.schema(), raw.into()).unwrap();
        assert_eq!(p.blocks[0].source, 0..9);
        let n = edited(&p, 2, 4, "乙");
        let result = apply(&p, &n);
        assert_eq!(result, raw.replacen("😀", "乙", 1));
        assert_eq!(Projection::parse(m.schema(), result).unwrap().doc, n);
    }
    #[test]
    fn strict_utf16_positions_reject_surrogate_interior() {
        let m = manager();
        let p = Projection::parse(m.schema(), "# 甲😀".into()).unwrap();
        assert_eq!(p.source_position(1), Some(2));
        assert_eq!(p.source_position(2), Some(5));
        assert_eq!(p.source_position(3), None);
        assert_eq!(p.source_position(4), Some(9));
    }
    #[test]
    fn opaque_source_is_editable_but_never_serialized_as_code_fence() {
        let m = manager();
        let p = Projection::parse(m.schema(), "<custom>保留</custom>\r\n".into()).unwrap();
        assert!(p.blocks[0].opaque);
        let n = edited(&p, 9, 11, "更新");
        let raw = apply(&p, &n);
        assert_eq!(raw, "<custom>更新</custom>\r\n");
    }
    #[test]
    fn hundred_projection_switches_leave_owner_token_history_and_dirty_unchanged() {
        use texzi_document::*;
        let m = manager();
        let raw = "\u{feff}# 甲😀\r\n\n未知 <widget data-x='a'/>\n";
        let mut store = Store::new(HistoryBudget::default());
        let s = store.open_utf8(raw, Profile::default(), false).unwrap();
        let dirty = store.dirty(s.doc).unwrap();
        let depth = store.history_depth(s.doc).unwrap();
        for _ in 0..100 {
            let p = Projection::parse(m.schema(), s.body.source().to_string()).unwrap();
            assert!(p.patch(&p.doc).unwrap().is_none());
            assert_eq!(p.source, s.body.source().to_string());
        }
        let after = store.snapshot(s.doc).unwrap();
        assert_eq!(s.revision, after.revision);
        assert_eq!(s.token, after.token);
        assert_eq!(dirty, store.dirty(s.doc).unwrap());
        assert_eq!(depth, store.history_depth(s.doc).unwrap());
    }
    #[test]
    fn rich_local_patch_uses_shared_history_and_source_view_can_undo_raw_eols() {
        use texzi_document::*;
        let m = manager();
        let raw = "# 甲😀\r\n\r\n<custom>keep</custom>\n\n末尾\r";
        let mut store = Store::new(HistoryBudget::default());
        let s = store.open_utf8(raw, Profile::default(), false).unwrap();
        let set = SelectionSet::caret(&s, 0);
        let source = store
            .open_view(
                s.doc,
                ModeTag::Source,
                set.clone(),
                set.selections[0].head,
                0,
            )
            .unwrap();
        let rich = store
            .open_view(s.doc, ModeTag::Rich, set.clone(), set.selections[0].head, 0)
            .unwrap();
        let p = Projection::parse(m.schema(), s.body.source().to_string()).unwrap();
        let n = edited(&p, 2, 4, "乙");
        let edits = p.source_edits(&n).unwrap();
        let caret = edits[0].0.start + edits[0].1.len();
        let plan = store
            .plan_view_edits(
                rich.id,
                s.revision,
                rich.epoch,
                edits,
                vec![(caret, caret)],
                0,
                InputKind::Structural,
                0,
            )
            .unwrap();
        let r = store.submit_view(plan).unwrap();
        assert_eq!(
            r.snapshot.body.to_utf8(),
            raw.replacen("😀", "乙", 1).as_bytes()
        );
        assert_eq!(store.history_depth(s.doc).unwrap(), (1, 0));
        let v = store.view(source.id).unwrap();
        store.undo_view(v.id, v.epoch).unwrap();
        let undone = store.snapshot(s.doc).unwrap();
        assert_eq!(undone.body.to_utf8(), raw.as_bytes());
        assert_eq!(undone.token, s.token);
        let v = store.view(rich.id).unwrap();
        store.redo_view(v.id, v.epoch).unwrap();
        assert_eq!(
            store.snapshot(s.doc).unwrap().body.to_utf8(),
            raw.replacen("😀", "乙", 1).as_bytes()
        );
    }
    #[test]
    fn frozen_corpus_noop_preserves_every_source_byte_and_gap() {
        let m = manager();
        let cases = [
            include_str!("../tests/fixtures/markdown/code-fences.md"),
            include_str!("../tests/fixtures/markdown/escaped-inline.md"),
            include_str!("../tests/fixtures/markdown/images.md"),
            include_str!("../tests/fixtures/markdown/links.md"),
            include_str!("../tests/fixtures/markdown/list-style.md"),
            include_str!("../tests/fixtures/markdown/mixed-known.md"),
            include_str!("../tests/fixtures/markdown/no-final-newline.md"),
            include_str!("../tests/fixtures/markdown/opaque-extension.md"),
            include_str!("../tests/fixtures/markdown/opaque-html.md"),
            include_str!("../tests/fixtures/markdown/setext-and-tabs.md"),
            include_str!("../tests/fixtures/markdown/table-pipes.md"),
            include_str!("../tests/fixtures/markdown/unclosed-fence.md"),
        ];
        for raw in cases {
            for _ in 0..100 {
                let p = Projection::parse(m.schema(), raw.into()).unwrap();
                assert_eq!(p.source.as_bytes(), raw.as_bytes());
                assert!(p.patch(&p.doc).unwrap().is_none());
                let mut cursor = 0;
                for b in &p.blocks {
                    assert!(b.source.start >= cursor && b.source.end <= raw.len());
                    assert!(
                        raw.is_char_boundary(b.source.start) && raw.is_char_boundary(b.source.end)
                    );
                    cursor = b.source.end;
                }
            }
        }
    }
}
fn contains_opaque(n: &mdast::Node) -> bool {
    matches!(
        n,
        mdast::Node::Html(_)
            | mdast::Node::Definition(_)
            | mdast::Node::FootnoteDefinition(_)
            | mdast::Node::FootnoteReference(_)
            | mdast::Node::LinkReference(_)
            | mdast::Node::ImageReference(_)
    ) || n
        .children()
        .is_some_and(|xs| xs.iter().any(contains_opaque))
}
fn single_text_patch(b: &Block, next: &Node, source: &str) -> Option<(Range<usize>, String)> {
    // Descend unchanged wrappers (list/table/marks) and require exactly one
    // changed text leaf. Everything outside its verified raw range stays raw.
    fn changed<'a>(a: &'a Node, z: &'a Node, out: &mut Vec<(&'a str, &'a str)>) -> Option<()> {
        if a == z {
            return Some(());
        }
        if !a.same_markup(z) {
            return None;
        }
        if let (Some(x), Some(y)) = (a.text(), z.text()) {
            out.push((x, y));
            return Some(());
        }
        if a.child_count() != z.child_count() {
            return None;
        }
        for i in 0..a.child_count() {
            changed(a.child(i), z.child(i), out)?;
        }
        Some(())
    }
    let mut leaves = Vec::new();
    changed(&b.node, next, &mut leaves)?;
    if leaves.len() != 1 {
        return None;
    }
    let (old, new) = leaves[0];
    let at = raw_text_start(b, old, source)?;
    Some((
        b.source.start + at..b.source.start + at + old.len(),
        new.into(),
    ))
}

fn fenced_content_start(b: &Block, text: &str, source: &str) -> Option<usize> {
    let same_text = (b.node.child_count() == 0 && text.is_empty())
        || (b.node.child_count() == 1 && b.node.child(0).text() == Some(text));
    if b.opaque || b.node.ty().name() != "codeBlock" || !same_text {
        return None;
    }
    let raw = &source[b.source.clone()];
    let first = raw.split('\n').next()?.trim_start();
    if !first.starts_with("```") && !first.starts_with("~~~") {
        return None;
    }
    let at = raw.find('\n')? + 1;
    (raw.get(at..at + text.len()) == Some(text)).then_some(at)
}
fn raw_text_start(b: &Block, text: &str, source: &str) -> Option<usize> {
    if let Some(at) = fenced_content_start(b, text, source) {
        return Some(at);
    }
    let raw = &source[b.source.clone()];
    let mut matches = raw.match_indices(text);
    let (at, _) = matches.next()?;
    matches.next().is_none().then_some(at)
}
fn empty_position(b: &Block, source: &str) -> Option<usize> {
    if b.node.ty().name() == "codeBlock" && !b.opaque {
        return fenced_content_start(b, "", source).map(|at| b.source.start + at);
    }
    Some(b.source.start)
}

fn task_toggle_patch(
    b: &Block,
    next: &Node,
    source: &str,
) -> Result<Option<(Range<usize>, String)>, String> {
    fn walk(
        a: &Node,
        z: &Node,
        index: &mut usize,
        changed: &mut Option<(usize, bool, bool)>,
    ) -> bool {
        if a.ty().name() != z.ty().name()
            || a.text() != z.text()
            || a.child_count() != z.child_count()
        {
            return false;
        }
        let task = a.ty().name() == "taskItem";
        let ordinal = *index;
        if task {
            *index += 1;
        }
        if !a.same_markup(z) {
            if !task || a.marks() != z.marks() || changed.is_some() {
                return false;
            }
            let mut old = a.attrs().clone();
            let mut new = z.attrs().clone();
            let Some(x) = old.remove("checked").and_then(|v| v.as_bool()) else {
                return false;
            };
            let Some(y) = new.remove("checked").and_then(|v| v.as_bool()) else {
                return false;
            };
            if old != new || x == y {
                return false;
            }
            *changed = Some((ordinal, x, y));
        }
        (0..a.child_count()).all(|i| walk(a.child(i), z.child(i), index, changed))
    }
    let mut change = None;
    if !walk(&b.node, next, &mut 0, &mut change) {
        return Ok(None);
    }
    let Some((ordinal, old, new)) = change else {
        return Ok(None);
    };
    fn tasks(n: &mdast::Node, out: &mut Vec<(usize, bool)>) {
        if let mdast::Node::ListItem(item) = n {
            if let (Some(done), Some(p)) = (item.checked, item.position.as_ref()) {
                out.push((p.start.offset, done));
            }
        }
        if let Some(children) = n.children() {
            for child in children {
                tasks(child, out);
            }
        }
    }
    let raw = &source[b.source.clone()];
    let ast = markdown::to_mdast(raw, &ParseOptions::gfm()).map_err(message)?;
    let mut items = Vec::new();
    tasks(&ast, &mut items);
    let &(start, done) = items
        .get(ordinal)
        .ok_or("任务标记源位置不可映射，原文未变")?;
    if done != old {
        return Err("任务标记源状态不一致，原文未变".into());
    }
    let line = raw[start..].split('\n').next().unwrap_or("");
    let marker = if old {
        line.find("[x]").or_else(|| line.find("[X]"))
    } else {
        line.find("[ ]")
    }
    .ok_or("任务标记源语法不可映射，原文未变")?;
    let at = b.source.start + start + marker + 1;
    Ok(Some((at..at + 1, if new { "x" } else { " " }.into())))
}
