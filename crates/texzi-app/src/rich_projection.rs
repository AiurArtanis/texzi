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

mod block_styles;
mod code_actions;
mod heading_import;
mod list_items;
mod local_patch;
mod plain_insert;
mod references;
mod resources;
mod table_rows;
mod table_structures;
mod text_map;

pub use resources::{ResourceKind, ResourceTarget, ResourceValues};

pub fn manager() -> ExtensionManager {
    ExtensionManager::with_core(vec![
        kit::starter_kit().configure(
            json!({"history":false,"trailingNode":false,"typography":false,
                "codeBlock":{"enableTabIndentation":true,"tabSize":4,"exitOnTripleEnter":false}}),
        ),
        kit::table_kit(),
    ])
    .expect("fixed rich schema")
}

#[derive(Clone)]
pub struct Block {
    pub source: Range<usize>,
    pub node: Node,
    pub opaque: bool,
    text_map: text_map::TextMap,
}
pub struct Projection {
    pub source: String,
    pub doc: Node,
    pub blocks: Vec<Block>,
}
type SourceEdit = (Range<usize>, String);
fn message(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn parse_options() -> ParseOptions {
    let mut options = ParseOptions::gfm();
    options.constructs.math_flow = true;
    options.constructs.math_text = true;
    options
}
fn list_content_end(node: &mdast::Node, source: &str) -> Option<usize> {
    match node {
        mdast::Node::List(list) => list
            .children
            .last()
            .and_then(|n| list_content_end(n, source)),
        mdast::Node::ListItem(item) => {
            if let Some(last) = item.children.last()
                && !matches!(last, mdast::Node::Paragraph(p) if p.children.is_empty())
            {
                return list_content_end(last, source);
            }
            // An empty item has syntax but no content child. Its complete
            // first-line prefix, including trailing spaces, remains source.
            let start = item.position.as_ref()?.start.offset;
            Some(
                start
                    + source
                        .get(start..)?
                        .find(['\r', '\n'])
                        .unwrap_or(source.len() - start),
            )
        }
        _ => node.position().map(|p| p.end.offset),
    }
}
fn root(schema: &Schema, nodes: Vec<Node>) -> Result<Node, String> {
    schema
        .node("doc", None, Fragment::from(nodes), vec![])
        .map_err(message)
}
impl Projection {
    /// The raw-block edit gate may admit this explicit resource form only
    /// after its AST target, values and complete resulting tree were checked.
    pub fn verified_resource_transaction(
        &self,
        next: &Node,
        applied: &[Transaction],
    ) -> Result<bool, String> {
        Ok(resources::transaction_edits(self, next, applied)?.is_some())
    }
    pub fn verified_table_transaction(
        &self,
        next: &Node,
        applied: &[Transaction],
    ) -> Result<bool, String> {
        Ok(table_structures::transaction_edits(self, next, applied)?.is_some())
    }
    /// Replacing the selected visible label keeps its existing resource. The
    /// upstream non-inclusive link mark deliberately does not extend at a
    /// cursor, but its generic marks_across also drops a whole selected label.
    /// Restore it only for a nonempty, text-only replacement wholly inside one
    /// identical link. Explicit unlink/format steps never enter this path.
    pub fn inherit_replacement_link(&self, tr: &mut Transaction) -> Result<(), String> {
        if tr.steps.len() != 1 || tr.docs.first() != Some(&self.doc) {
            return Ok(());
        }
        let Some(step) = tr.steps[0].as_any().downcast_ref::<ReplaceStep>() else {
            return Ok(());
        };
        if step.structure
            || step.from >= step.to
            || step.slice.open_start != 0
            || step.slice.open_end != 0
            || step.slice.content.child_count() == 0
            || !(0..step.slice.content.child_count()).all(|i| {
                step.slice.content.child(i).is_text()
                    && !step
                        .slice
                        .content
                        .child(i)
                        .marks()
                        .iter()
                        .any(|m| m.ty.name() == "link")
            })
        {
            return Ok(());
        }
        let (from, to, length) = (step.from, step.to, step.slice.content.size());
        let mut mark = None;
        let mut valid = true;
        self.doc.nodes_between(from, to, &mut |node, _, _, _| {
            if node.is_text() {
                let link = node.marks().iter().find(|m| m.ty.name() == "link");
                match (&mark, link) {
                    (None, Some(link)) => mark = Some(link.clone()),
                    (Some(old), Some(link)) if old == link => {}
                    _ => valid = false,
                }
            } else if node.is_leaf() {
                valid = false;
            }
            true
        });
        if valid && let Some(mark) = mark {
            tr.add_mark(from, from + length, mark).map_err(message)?;
        }
        Ok(())
    }
    pub fn rich_position(&self, byte: usize) -> Option<usize> {
        let mut base = 0;
        for block in &self.blocks {
            if let Some(found) = block.text_map.rich_position(byte) {
                return Some(base + found);
            }
            if let Some((pos, _)) = list_items::empty_positions(block, &self.source)
                .into_iter()
                .chain(table_rows::empty_positions(block, &self.source))
                .find(|(_, at)| *at == byte)
            {
                return Some(base + pos);
            }
            if block.node.content().size() == 0 && Some(byte) == empty_position(block, &self.source)
            {
                return Some(base + 1);
            }
            base += block.node.node_size();
        }
        None
    }
    /// Syntax delimiters and expanded entities may have no exact visible
    /// boundary. A mode switch stays in the same source block, choosing its
    /// closest editable text boundary without changing any document bytes.
    pub fn nearest_rich_position(&self, byte: usize) -> Option<usize> {
        if let Some(exact) = self.rich_position(byte) {
            return Some(exact);
        }
        let mut base = 0;
        for block in &self.blocks {
            if byte >= block.source.start && byte <= block.source.end {
                let nearest = block
                    .text_map
                    .units
                    .iter()
                    .flat_map(|u| [(u.source.start, u.rich.start), (u.source.end, u.rich.end)])
                    .chain(
                        list_items::empty_positions(block, &self.source)
                            .into_iter()
                            .map(|(pos, byte)| (byte, pos)),
                    )
                    .min_by_key(|(at, _)| at.abs_diff(byte));
                return nearest
                    .map(|(_, pos)| base + pos)
                    .or_else(|| (block.node.is_textblock()).then_some(base + 1));
            }
            base += block.node.node_size();
        }
        // A normal terminal EOL or a separator belongs to source trivia, not
        // to a text leaf. Keep its caret beside the closest surviving block.
        let mut base = 0;
        let mut nearest = None;
        for block in &self.blocks {
            for unit in &block.text_map.units {
                for (at, pos) in [
                    (unit.source.start, unit.rich.start),
                    (unit.source.end, unit.rich.end),
                ] {
                    let distance = at.abs_diff(byte);
                    if nearest.is_none_or(|(best, _)| distance < best) {
                        nearest = Some((distance, base + pos));
                    }
                }
            }
            base += block.node.node_size();
        }
        nearest.map(|(_, pos)| pos)
    }
    /// A single plain-text ReplaceStep supplies the identity of the edited
    /// source range. Validate the complete projection before preferring it to
    /// a structural serializer/diff; never infer newline identity from a diff.
    pub fn transaction_edits(
        &self,
        next: &Node,
        applied: &[Transaction],
    ) -> Result<Vec<SourceEdit>, String> {
        self.plan_transaction_edits(next, applied, &mut None)
    }

    pub fn transaction_edits_with_projection(
        &self,
        next: &Node,
        applied: &[Transaction],
    ) -> Result<(Vec<SourceEdit>, Option<Self>), String> {
        let mut projection = None;
        let edits = self.plan_transaction_edits(next, applied, &mut projection)?;
        Ok((edits, projection))
    }

    fn plan_transaction_edits(
        &self,
        next: &Node,
        applied: &[Transaction],
        projection: &mut Option<Self>,
    ) -> Result<Vec<SourceEdit>, String> {
        if let Some(edits) = code_actions::transaction_edits(self, next, applied)? {
            return Ok(edits);
        }
        if let Some(edits) = resources::transaction_edits(self, next, applied)? {
            return Ok(edits);
        }
        if let Some(edits) = table_structures::transaction_edits(self, next, applied)? {
            return Ok(edits);
        }
        if let Some(edits) = list_items::transaction_edits(self, next, applied)? {
            return Ok(edits);
        }
        if let Some(edits) = block_styles::edits(self, next)? {
            return Ok(edits);
        }
        if &self.doc == next {
            return Ok(vec![]);
        }
        let steps: Vec<_> = applied.iter().flat_map(|t| t.steps.iter()).collect();
        if steps.len() == 1 {
            if let Some(step) = steps[0].as_any().downcast_ref::<ReplaceStep>() {
                if let Some(edit) = self.delete_opaque_block(next, step)? {
                    return Ok(vec![edit]);
                }
                if let Some((range, text)) = table_rows::append_edit(self, next, step)? {
                    let mut raw = self.source.clone();
                    raw.replace_range(range.clone(), &text);
                    if Self::parse(next.ty().schema(), raw).is_ok_and(|p| &p.doc == next) {
                        return Ok(vec![(range, text)]);
                    }
                    return Err("新增表格行暂不能保真往返，原文未变".into());
                }
                if step.structure
                    && step.from == step.to
                    && step.slice.open_start == 2
                    && step.slice.open_end == 2
                {
                    let mut base = 0;
                    let in_bullet = self.blocks.iter().any(|block| {
                        let end = base + block.node.node_size();
                        let inside = block.node.ty().name() == "bulletList"
                            && step.from > base
                            && step.from < end;
                        base = end;
                        inside
                    });
                    if in_bullet {
                        let (range, text) = list_items::split_edit(self, next, step)
                            .ok_or("该列表拆分暂不能保真映射，原文未变")?;
                        let mut raw = self.source.clone();
                        raw.replace_range(range.clone(), &text);
                        if Self::parse(next.ty().schema(), raw).is_ok_and(|p| &p.doc == next) {
                            return Ok(vec![(range, text)]);
                        }
                        return Err("该列表拆分不能保真往返，原文未变".into());
                    }
                }
                if !step.structure
                    && step.slice.open_start == 0
                    && step.slice.open_end == 0
                    && (0..step.slice.content.child_count())
                        .all(|i| step.slice.content.child(i).is_text())
                {
                    if let Some((edit, updated)) = plain_insert::project(self, next, step) {
                        *projection = Some(updated);
                        return Ok(vec![edit]);
                    }
                    if let Some((edit, verified)) = local_patch::text_step(self, next, step) {
                        *projection = Some(verified);
                        return Ok(vec![edit]);
                    }
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
                            if step.from == step.to && !text.is_empty() {
                                let mut base = 0;
                                let adjacent_fence = self.blocks.iter().any(|block| {
                                    let matches = step.from == base + 1
                                        && adjacent_empty_fence_at(block, &self.source) == Some(a);
                                    base += block.node.node_size();
                                    matches
                                });
                                if adjacent_fence {
                                    // Materialize the missing body/closing-fence separator.
                                    // All existing bytes and EOL provenance stay outside the edit.
                                    let replacement = format!("{text}\n");
                                    let mut raw = self.source.clone();
                                    raw.replace_range(a..h, &replacement);
                                    if Self::parse(next.ty().schema(), raw)
                                        .is_ok_and(|p| &p.doc == next)
                                    {
                                        return Ok(vec![(a..h, replacement)]);
                                    }
                                    return Err("该空围栏输入暂不能保真往返，原文未变".into());
                                }
                            }
                            if step.from == step.to {
                                let mut base = 0;
                                let empty_container = self.blocks.iter().any(|block| {
                                    let empty = list_items::empty_positions(block, &self.source)
                                        .into_iter()
                                        .chain(table_rows::empty_positions(block, &self.source))
                                        .any(|(pos, _)| base + pos == step.from);
                                    base += block.node.node_size();
                                    empty
                                });
                                if empty_container {
                                    return Err("该空结构中的输入暂不能保真写回，原文未变".into());
                                }
                            }
                        }
                    }
                }
            }
        }
        self.source_edits(next)
    }
    /// Only an exact, closed deletion of one parser-owned raw block enters
    /// this seam. Remove separator trivia only when the complete projection
    /// requires it; retained bytes never pass through a serializer or diff.
    fn delete_opaque_block(
        &self,
        next: &Node,
        step: &ReplaceStep,
    ) -> Result<Option<(Range<usize>, String)>, String> {
        if step.slice.open_start != 0 || step.slice.open_end != 0 {
            return Ok(None);
        }
        let mut base = 0;
        let target = self.blocks.iter().enumerate().find_map(|(index, block)| {
            let end = base + block.node.node_size();
            let exact = block.opaque && step.from == base && step.to == end;
            base = end;
            exact.then_some((index, block))
        });
        let Some((index, block)) = target else {
            return Ok(None);
        };
        let sole_empty_paragraph = if self.doc.child_count() == 1
            && next.child_count() == 1
            && step.slice.content.child_count() == 1
        {
            let empty = next
                .ty()
                .schema()
                .node("paragraph", None, Fragment::empty(), vec![])
                .map_err(message)?;
            step.slice.content.child(0) == &empty && next.child(0) == &empty
        } else {
            false
        };
        if step.slice.content.size() != 0 && !sole_empty_paragraph {
            return Ok(None);
        }
        let before = index
            .checked_sub(1)
            .map(|i| self.blocks[i].source.end)
            .unwrap_or(0);
        let after = self
            .blocks
            .get(index + 1)
            .map(|b| b.source.start)
            .unwrap_or(self.source.len());
        let eols = |range: Range<usize>| {
            self.source[range]
                .bytes()
                .all(|b| matches!(b, b'\r' | b'\n'))
        };
        let mut candidates = vec![block.source.clone()];
        if eols(block.source.end..after) {
            candidates.push(block.source.start..after);
        }
        if eols(before..block.source.start) {
            candidates.push(before..block.source.end);
        }
        if eols(before..block.source.start) && eols(block.source.end..after) {
            candidates.push(before..after);
        }
        candidates.sort_by_key(|range| range.len());
        candidates.dedup();
        for range in candidates {
            let mut raw = self.source.clone();
            raw.replace_range(range.clone(), "");
            if Self::parse(next.ty().schema(), raw).is_ok_and(|p| &p.doc == next) {
                return Ok(Some((range, String::new())));
            }
        }
        Err("该原样块删除暂不能保真往返，原文未变".into())
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
        let trace =
            std::env::var_os("TEXZI_TRACE_IME").as_deref() == Some(std::ffi::OsStr::new("1"));
        if trace {
            log::info!("RICH_PROJECTION action=full_parse bytes={}", source.len());
        }
        let ast_started = trace.then(std::time::Instant::now);
        let mut tree = markdown::to_mdast(&source, &parse_options()).map_err(message)?;
        let import_started = trace.then(std::time::Instant::now);
        list_items::recognize_empty_tasks(&mut tree, &source);
        let mut blocks = Vec::new();
        let definitions: Vec<_> = tree
            .children()
            .ok_or("missing Markdown root")?
            .iter()
            .filter_map(|n| {
                if let mdast::Node::Definition(d) = n {
                    Some(d)
                } else {
                    None
                }
            })
            .collect();
        for child in tree.children().ok_or("missing Markdown root")? {
            let heading = heading_import::prepare(child, &source);
            let child = heading
                .as_ref()
                .map(|heading| &heading.ast)
                .unwrap_or(child);
            let p = child.position().ok_or("missing source span")?;
            let mut span = p.start.offset..p.end.offset;
            // A list container includes terminal separator EOLs beyond its
            // last actual item. Keep these parser-owned trivia bytes outside
            // the block so the existing empty-paragraph projection can see
            // them. The source itself, including every EOL, stays unchanged.
            if matches!(child, mdast::Node::List(_))
                && let Some(end) = list_content_end(child, &source)
            {
                span.end = end;
            }
            // The parser includes a footnote definition's terminating EOL in
            // its container span. It remains source trivia, like the terminal
            // EOL of a paragraph, rather than part of the literal block text.
            if matches!(child, mdast::Node::FootnoteDefinition(_)) {
                while span.end > span.start
                    && matches!(source.as_bytes()[span.end - 1], b'\n' | b'\r')
                {
                    span.end -= 1;
                }
            }
            let raw = source
                .get(span.clone())
                .ok_or("invalid parser byte boundary")?;
            // Unsupported syntax is represented visibly as a local source block.
            // It is never silently discarded by the Markdown importer.
            let mut opaque = contains_opaque(child) || raw.trim_start().starts_with(":::");
            let parsed = if opaque {
                None
            } else if let Some(heading) = heading.as_ref() {
                heading.import(schema, &source, &definitions)
            } else if list_items::has_empty_tasks(child) {
                // A recognized empty-task import must remove every owned
                // placeholder by AST identity. Failure stays a complete raw
                // block, with its original source map and local source entry.
                list_items::import_empty_tasks(schema, child, &source, &definitions)
            } else {
                references::import_source(child, &source, &definitions)
                    .and_then(|input| MarkdownFormat::default().parse(schema, &input).ok())
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
                source: span.clone(),
                text_map: text_map::TextMap::build(&node, child, &source, opaque, span),
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
                text_map: text_map::TextMap::default(),
            });
        }
        let doc = root(schema, blocks.iter().map(|b| b.node.clone()).collect())?;
        doc.check().map_err(message)?;
        if let (Some(ast), Some(import)) = (ast_started, import_started) {
            log::info!(
                "RICH_PROJECTION action=phase_timing ast_us={} import_us={}",
                import.duration_since(ast).as_micros(),
                import.elapsed().as_micros()
            );
        }
        if std::env::var_os("TEXZI_TRACE_PROJECTION").as_deref() == Some(std::ffi::OsStr::new("1"))
        {
            use std::hash::{DefaultHasher, Hash, Hasher};
            let mut digest = DefaultHasher::new();
            doc.to_json().to_string().hash(&mut digest);
            for block in &blocks {
                block.source.hash(&mut digest);
                block.opaque.hash(&mut digest);
                for unit in &block.text_map.units {
                    unit.rich.hash(&mut digest);
                    unit.source.hash(&mut digest);
                }
            }
            log::info!(
                "RICH_PROJECTION action=fingerprint value={:016x} blocks={}",
                digest.finish(),
                blocks.len()
            );
        }
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
                let mut found = b.text_map.source_positions(pos - base).last().copied();
                if found.is_none() {
                    found = list_items::empty_positions(b, &self.source)
                        .into_iter()
                        .chain(table_rows::empty_positions(b, &self.source))
                        .find_map(|(at, byte)| (pos == base + at).then_some(byte));
                }
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
            if let Some(patch) = local_patch::block_patch(self, first, next) {
                let mut raw = self.source.clone();
                raw.replace_range(patch.0.clone(), &patch.1);
                if Self::parse(next.ty().schema(), raw).is_ok_and(|p| &p.doc == next) {
                    return Ok(Some(patch));
                }
            }
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
            // A heading or other block can be followed by a paragraph with
            // only one LF. The projection needs four LFs for an empty
            // paragraph between existing blocks. Extend that seam without
            // serializing either neighboring block.
            if first > 0
                && new_end > first
                && (first..new_end).all(|i| {
                    let node = next.child(i);
                    node.ty().name() == "paragraph" && node.content().size() == 0
                })
                && &self.source[self.blocks[first - 1].source.end..start] == "\n"
            {
                text.push('\n');
            }
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
        text_map: text_map::TextMap::default(),
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
    fn plain_insert_reuses_verified_source_and_positions() {
        let m = manager();
        let raw = "开头😀正文\r\n\r\n[保留][ref]\n\n[ref]: /original\n";
        let p = Projection::parse(m.schema(), raw.into()).unwrap();
        let state =
            EditorState::create(EditorStateConfig::new(m.schema().clone()).doc(p.doc.clone()))
                .unwrap();
        let byte = raw.find("正文").unwrap();
        let pos = p.rich_position(byte).unwrap();
        let mut tr = state.tr();
        tr.insert_text("Z", Some(pos), Some(pos));
        let (next, applied) = state.apply_transaction(tr).unwrap();
        let (edits, verified) = p
            .transaction_edits_with_projection(next.doc(), &applied)
            .unwrap();
        assert_eq!(edits, vec![(byte..byte, "Z".into())]);
        let verified = verified.unwrap();
        let expected = raw.replace("正文", "Z正文");
        let full = Projection::parse(m.schema(), expected.clone()).unwrap();
        assert_eq!(verified.source, expected);
        assert_eq!(verified.doc, full.doc);
        for pos in 0..=full.doc.content().size() {
            assert_eq!(verified.source_position(pos), full.source_position(pos));
        }
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
    fn ast_text_map_distinguishes_repeated_leaves_and_entity_boundaries() {
        let m = manager();
        let raw = "**foo** foo &amp; \\*甲😀\\*";
        let p = Projection::parse(m.schema(), raw.into()).unwrap();
        for needle in ["**foo", "foo &", "&amp;", "甲😀"] {
            let byte = raw.find(needle).unwrap() + if needle.starts_with("**") { 2 } else { 0 };
            let pos = p.rich_position(byte).unwrap();
            assert_eq!(p.source_position(pos), Some(byte), "{needle}");
        }
        let amp = p.rich_position(raw.find("&amp;").unwrap()).unwrap();
        assert_eq!(
            p.source_position(amp + 1),
            Some(raw.find("&amp;").unwrap() + 5)
        );
        let n = edited(&p, amp, amp + 1, "和");
        assert_eq!(apply(&p, &n), raw.replace("&amp;", "和"));
    }
    #[test]
    fn code_span_backtick_edit_changes_only_necessary_node_fence() {
        let m = manager();
        let raw = "原 \\*样\\* &amp; `code` 后 **保留**\n";
        let p = Projection::parse(m.schema(), raw.into()).unwrap();
        let from = p.rich_position(raw.find("code").unwrap() + 2).unwrap();
        let n = edited(&p, from, from, "`");
        let result = apply(&p, &n);
        assert_eq!(result, raw.replace("`code`", "``co`de``"));
        assert_eq!(Projection::parse(m.schema(), result).unwrap().doc, n);
    }
    #[test]
    fn heading_level_change_preserves_existing_style_text_escape_and_trivia() {
        let m = manager();
        for (raw, level, expected) in [
            (
                "  ##  中文 &amp; \\*字\\* ##\n",
                3,
                "  ###  中文 &amp; \\*字\\* ##\n",
            ),
            (
                "中文 &amp; \\*字\\*\n=======\n",
                2,
                "中文 &amp; \\*字\\*\n-------\n",
            ),
        ] {
            let p = Projection::parse(m.schema(), raw.into()).unwrap();
            let state =
                EditorState::create(EditorStateConfig::new(m.schema().clone()).doc(p.doc.clone()))
                    .unwrap();
            let mut tr = state.tr();
            tr.set_node_attribute(0, "level", json!(level)).unwrap();
            let next = state.apply(tr).unwrap();
            assert_eq!(apply(&p, next.doc()), expected);
        }
    }
    #[test]
    fn imported_nested_code_and_references_map_to_original_source_text() {
        let m = manager();
        for raw in [
            "> ~~~rust\n> alpha\n> beta\n> ~~~\n",
            "+ outer\n  + **alpha** &amp; alpha\n",
            "[alpha &amp; 甲][ref]\n\n[ref]: <https://example.invalid> \"原题\"\n",
        ] {
            let p = Projection::parse(m.schema(), raw.into()).unwrap();
            let byte = raw.rfind("alpha").unwrap();
            let pos = p.rich_position(byte).unwrap();
            assert_eq!(p.source_position(pos), Some(byte));
            let state =
                EditorState::create(EditorStateConfig::new(m.schema().clone()).doc(p.doc.clone()))
                    .unwrap();
            let mut tr = state.tr();
            tr.insert_text("A", Some(pos), Some(pos + 1));
            let (next, applied) = state.apply_transaction(tr).unwrap();
            let mut result = raw.to_owned();
            for (r, t) in p
                .transaction_edits(next.doc(), &applied)
                .unwrap()
                .into_iter()
                .rev()
            {
                result.replace_range(r, &t);
            }
            let mut expected = raw.to_owned();
            expected.replace_range(byte..byte + 1, "A");
            assert_eq!(result, expected);
        }
    }
    #[test]
    fn unmarked_textblock_edge_input_preserves_link_spellings_and_mixed_eols() {
        let m = manager();
        let fixture = "[普通](target.md) <https://example.invalid> [引用][id]\n\n[id]: target.md\n";
        assert_eq!(fixture.len(), 76);
        for raw in [
            fixture,
            "[普通](target.md) <https://example.invalid> [引用][id]\r\n\r\n[id]: target.md\n\n<custom>stay</custom>\r\n",
        ] {
            let p = Projection::parse(m.schema(), raw.into()).unwrap();
            assert_eq!(p.doc.child(0).ty().name(), "paragraph");
            assert_eq!(p.blocks[0].source, 0..58);
            for (pos, byte) in [(1, 0), (p.doc.child(0).node_size() - 1, 58)] {
                let state = EditorState::create(
                    EditorStateConfig::new(m.schema().clone()).doc(p.doc.clone()),
                )
                .unwrap();
                let mut tr = state.tr();
                tr.set_stored_marks(Some(vec![]));
                tr.insert_text("甲", Some(pos), Some(pos));
                let (next, applied) = state.apply_transaction(tr).unwrap();
                let edits = p.transaction_edits(next.doc(), &applied).unwrap();
                assert_eq!(edits, vec![(byte..byte, "甲".to_owned())]);
                let mut result = raw.to_owned();
                for (range, text) in edits.into_iter().rev() {
                    result.replace_range(range, &text);
                }
                let mut expected = raw.to_owned();
                expected.insert_str(byte, "甲");
                assert_eq!(result, expected);
                assert_eq!(result.len(), raw.len() + 3);
                assert_eq!(
                    Projection::parse(m.schema(), result).unwrap().doc,
                    *next.doc()
                );
            }
        }
    }
    #[test]
    fn marked_reference_label_replacement_verifies_full_definition_context() {
        let m = manager();
        let raw = "[普通](target.md) <https://example.invalid> [引用][id]\r\n\r\n[id]: target.md\n\n<custom>stay</custom>\r\n";
        let p = Projection::parse(m.schema(), raw.into()).unwrap();
        for label in ["普通", "引用"] {
            let byte = raw.find(label).unwrap();
            let end = byte + label.len();
            let from = p.rich_position(byte).unwrap();
            let to = p.rich_position(end).unwrap();
            let state =
                EditorState::create(EditorStateConfig::new(m.schema().clone()).doc(p.doc.clone()))
                    .unwrap();
            let mut tr = state.tr();
            tr.set_stored_marks(Some(vec![]));
            tr.insert_text("甲", Some(from), Some(to));
            p.inherit_replacement_link(&mut tr).unwrap();
            assert!(
                tr.steps.len() > 1,
                "link inheritance uses the real added mark step"
            );
            let (next, applied) = state.apply_transaction(tr).unwrap();
            let expected_edit = (byte..end, "甲".to_owned());
            assert_eq!(
                local_patch::block_patch(&p, 0, next.doc()),
                Some(expected_edit.clone())
            );
            assert_eq!(
                p.transaction_edits(next.doc(), &applied).unwrap(),
                vec![expected_edit]
            );
            let mut result = raw.to_owned();
            result.replace_range(byte..end, "甲");
            assert_eq!(
                Projection::parse(m.schema(), result).unwrap().doc,
                *next.doc()
            );
        }
    }
    #[test]
    fn math_footnote_and_custom_extension_display_complete_literal_source_blocks() {
        let m = manager();
        let raw = "anchor\n\n$x_1 + y^2$\n\n$$\na + b = c\n$$\n\ntext[^note]\n\n[^note]: note body\n\n::: custom\nopaque body\n:::\n\nend\n";
        let p = Projection::parse(m.schema(), raw.into()).unwrap();
        let literal: Vec<_> = p
            .blocks
            .iter()
            .filter(|b| b.opaque)
            .map(|b| b.node.text_content())
            .collect();
        for expected in [
            "$x_1 + y^2$",
            "$$\na + b = c\n$$",
            "text[^note]",
            "[^note]: note body",
            "::: custom\nopaque body\n:::",
        ] {
            assert!(
                literal.iter().any(|text| text == expected),
                "missing {expected:?}, literal blocks {literal:?}"
            );
        }
        assert_eq!(p.source, raw);
    }
    #[test]
    fn whole_link_label_replacement_preserves_resource_before_source_patch() {
        let m = manager();
        for raw in [
            "[apple](<https://example.invalid/inline> \"keep title\")",
            "+ [ ] **outer**\n  + [X] [apple](./target \"keep\")\n  + third",
        ] {
            let p = Projection::parse(m.schema(), raw.into()).unwrap();
            let state =
                EditorState::create(EditorStateConfig::new(m.schema().clone()).doc(p.doc.clone()))
                    .unwrap();
            let from = p.rich_position(raw.find("apple").unwrap()).unwrap();
            let to = p.rich_position(raw.find("apple").unwrap() + 5).unwrap();
            let mut tr = state.tr();
            tr.insert_text("Apple", Some(from), Some(to));
            p.inherit_replacement_link(&mut tr).unwrap();
            let (next, applied) = state.apply_transaction(tr).unwrap();
            let mut found = false;
            next.doc().descendants(&mut |node, _, _, _| {
                found |= node.text() == Some("Apple")
                    && node.marks().iter().any(|m| m.ty.name() == "link");
                true
            });
            assert!(found);
            let mut result = raw.to_owned();
            for (range, text) in p
                .transaction_edits(next.doc(), &applied)
                .unwrap()
                .into_iter()
                .rev()
            {
                result.replace_range(range, &text);
            }
            assert_eq!(result, raw.replacen("apple", "Apple", 1));
        }
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
            | mdast::Node::Math(_)
            | mdast::Node::InlineMath(_)
            | mdast::Node::Definition(_)
            | mdast::Node::FootnoteDefinition(_)
            | mdast::Node::FootnoteReference(_)
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

fn adjacent_empty_fence_at(b: &Block, source: &str) -> Option<usize> {
    if b.opaque || b.node.ty().name() != "codeBlock" || b.node.content().size() != 0 {
        return None;
    }
    let raw = source.get(b.source.clone())?;
    let (opening, closing) = raw.split_once('\n')?;
    // Runtime projections have canonical LF; do not reinterpret raw CR bytes.
    if raw.contains('\r') || closing.contains('\n') {
        return None;
    }
    let indent = opening.bytes().take_while(|b| *b == b' ').count();
    let marker = *opening.as_bytes().get(indent)?;
    if indent > 3 || !matches!(marker, b'`' | b'~') {
        return None;
    }
    let run = opening.as_bytes()[indent..]
        .iter()
        .take_while(|b| **b == marker)
        .count();
    let close_indent = closing.bytes().take_while(|b| *b == b' ').count();
    let close_run = closing.as_bytes()[close_indent..]
        .iter()
        .take_while(|b| **b == marker)
        .count();
    if run < 3
        || close_indent > 3
        || close_run < run
        || !closing.as_bytes()[close_indent + close_run..]
            .iter()
            .all(|b| matches!(*b, b' ' | b'\t'))
    {
        return None;
    }
    Some(b.source.start + opening.len() + 1)
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
    if b.node.ty().name() == "heading" {
        let raw = source.get(b.source.clone())?;
        let indent = raw.bytes().take_while(|b| *b == b' ').count();
        let run = raw[indent..].bytes().take_while(|b| *b == b'#').count();
        if run > 0 {
            let after = indent + run;
            let spacing = raw[after..]
                .bytes()
                .take_while(|b| matches!(b, b' ' | b'\t'))
                .count();
            return Some(b.source.start + after + spacing);
        }
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
    // Bind the marker to the actual list prefix; never search the item's text.
    let bullet = line.trim_start_matches(|c| c == ' ' || c == '\t');
    if !bullet
        .as_bytes()
        .first()
        .is_some_and(|b| matches!(*b, b'-' | b'+' | b'*'))
    {
        return Err("任务列表前缀不可映射，原文未变".into());
    }
    let spacing = &bullet[1..];
    if !spacing
        .as_bytes()
        .first()
        .is_some_and(|b| *b == b' ' || *b == b'\t')
    {
        return Err("任务列表间隔不可映射，原文未变".into());
    }
    let task = spacing.trim_start_matches(|c| c == ' ' || c == '\t');
    let matches = if old {
        task.starts_with("[x]") || task.starts_with("[X]")
    } else {
        task.starts_with("[ ]")
    };
    if !matches
        || !task
            .as_bytes()
            .get(3)
            .is_none_or(|b| *b == b' ' || *b == b'\t')
    {
        return Err("任务标记源语法不可映射，原文未变".into());
    }
    let marker = line.len() - task.len();
    let at = b.source.start + start + marker + 1;
    Ok(Some((at..at + 1, if new { "x" } else { " " }.into())))
}
