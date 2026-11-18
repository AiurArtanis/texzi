//! Resource forms issue ordinary transactions; only verified AST-owned ranges
//! become Store edits. Reference resources update their shared definition.
use super::{Projection, message, parse_options};
use markdown::mdast;
use nsh_base::rich_editor::{
    model::Node,
    state::{EditorState, EditorStateConfig, TextSelection, Transaction, TransactionOrigin},
};
use serde_json::json;
use std::ops::Range;

const META: &str = "texzi.resource";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    Link,
    Image,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceValues {
    pub url: String,
    pub title: Option<String>,
    pub alt: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceTarget {
    pub kind: ResourceKind,
    pub source: Range<usize>,
    pub values: ResourceValues,
    pub reference: Option<String>,
    pub visible: String,
    /// A new link owns this exact visible text selection, not an existing AST
    /// resource. Its source span is independently reconstructed by the guard.
    pub selection: Option<Range<usize>>,
}

pub(super) fn destination(value: &str) -> String {
    value.replace('&', "&amp;").replace('\\', "\\\\")
}
pub(super) fn title(value: &str) -> String {
    destination(value).replace('"', "\\\"")
}
fn label(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch == '&' {
            out.push_str("&amp;");
        } else {
            if ch.is_ascii_punctuation() {
                out.push('\\');
            }
            out.push(ch);
        }
    }
    out
}
fn key(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
fn definitions<'a>(node: &'a mdast::Node, out: &mut Vec<&'a mdast::Definition>) {
    if let mdast::Node::Definition(definition) = node {
        out.push(definition);
    }
    if let Some(children) = node.children() {
        for child in children {
            definitions(child, out);
        }
    }
}
fn target(node: &mdast::Node, defs: &[&mdast::Definition]) -> Option<ResourceTarget> {
    let p = node.position()?;
    let (kind, url, title, alt, reference) = match node {
        mdast::Node::Link(link) => (
            ResourceKind::Link,
            link.url.clone(),
            link.title.clone(),
            String::new(),
            None,
        ),
        mdast::Node::Image(image) => (
            ResourceKind::Image,
            image.url.clone(),
            image.title.clone(),
            image.alt.clone(),
            None,
        ),
        mdast::Node::LinkReference(link) => {
            let definition = defs
                .iter()
                .find(|d| key(&d.identifier) == key(&link.identifier))?;
            (
                ResourceKind::Link,
                definition.url.clone(),
                definition.title.clone(),
                String::new(),
                Some(link.identifier.clone()),
            )
        }
        mdast::Node::ImageReference(image) => {
            let definition = defs
                .iter()
                .find(|d| key(&d.identifier) == key(&image.identifier))?;
            (
                ResourceKind::Image,
                definition.url.clone(),
                definition.title.clone(),
                image.alt.clone(),
                Some(image.identifier.clone()),
            )
        }
        _ => return None,
    };
    let visible = match node {
        mdast::Node::Link(link) => link.children.iter().map(ToString::to_string).collect(),
        mdast::Node::LinkReference(link) => link.children.iter().map(ToString::to_string).collect(),
        _ => alt.clone(),
    };
    Some(ResourceTarget {
        kind,
        source: p.start.offset..p.end.offset,
        values: ResourceValues { url, title, alt },
        reference,
        visible,
        selection: None,
    })
}

fn link_destination(values: &ResourceValues) -> Result<String, String> {
    if values.url.is_empty()
        || values.url.contains(['\n', '\r', '<', '>'])
        || values
            .title
            .as_ref()
            .is_some_and(|v| v.contains(['\n', '\r']))
    {
        return Err("地址不能为空，地址和标题须为单行文字".into());
    }
    let suffix = values
        .title
        .as_ref()
        .map(|v| format!(" \"{}\"", title(v)))
        .unwrap_or_default();
    let destination = format!("<{}>{suffix}", destination(&values.url));
    let tree =
        markdown::to_mdast(&format!("[x]({destination})"), &parse_options()).map_err(message)?;
    let mut parsed = Vec::new();
    targets(&tree, &[], &mut parsed);
    if !parsed
        .first()
        .is_some_and(|t| t.values.url == values.url && t.values.title == values.title)
    {
        return Err("此地址或标题不能按当前 Markdown 语法保真写入".into());
    }
    Ok(destination)
}

fn format_node(node: &mdast::Node) -> bool {
    matches!(
        node,
        mdast::Node::Emphasis(_) | mdast::Node::Strong(_) | mdast::Node::Delete(_)
    )
}

// Preserve the original selected bytes, including escapes/entities and mark
// spelling. Fully selected emphasis delimiters belong inside the new link.
// A range that crosses only one edge of an emphasis splits that same spelling
// at the edge; complete tree equality below proves that no formatting moved.
fn link_edges(p: &Projection, range: &Range<usize>) -> Result<(String, String, String), String> {
    fn walk(
        node: &mdast::Node,
        raw: &str,
        range: &Range<usize>,
        start: &mut Vec<(String, String)>,
        end: &mut Vec<(String, String)>,
    ) -> Result<(), String> {
        if format_node(node) {
            let position = node.position().ok_or("格式范围缺少源码位置")?;
            let a = position.start.offset;
            let z = position.end.offset;
            let at_start = a < range.start && range.start < z;
            let at_end = a < range.end && range.end < z;
            if at_start != at_end {
                let children = node.children().ok_or("格式内容缺少源码位置")?;
                let first = children
                    .first()
                    .and_then(mdast::Node::position)
                    .ok_or("格式起点不能映射")?;
                let last = children
                    .last()
                    .and_then(mdast::Node::position)
                    .ok_or("格式终点不能映射")?;
                let open = raw.get(a..first.start.offset).ok_or("格式起点不能映射")?;
                let close = raw.get(last.end.offset..z).ok_or("格式终点不能映射")?;
                if open.is_empty()
                    || close.is_empty()
                    || !open
                        .bytes()
                        .chain(close.bytes())
                        .all(|b| matches!(b, b'*' | b'_' | b'~'))
                {
                    return Err("此格式边界请在源码中插入链接".into());
                }
                if at_start {
                    start.push((open.into(), close.into()));
                } else {
                    end.push((open.into(), close.into()));
                }
            }
        }
        if let Some(children) = node.children() {
            for child in children {
                walk(child, raw, range, start, end)?;
            }
        }
        Ok(())
    }
    let tree = markdown::to_mdast(&p.source, &parse_options()).map_err(message)?;
    let (mut start, mut end) = (Vec::new(), Vec::new());
    walk(&tree, &p.source, range, &mut start, &mut end)?;
    let prefix = format!(
        "{}[{}",
        start.iter().rev().map(|v| v.1.as_str()).collect::<String>(),
        start.iter().map(|v| v.0.as_str()).collect::<String>()
    );
    let suffix = format!(
        "{}]",
        end.iter().rev().map(|v| v.1.as_str()).collect::<String>()
    );
    Ok((
        prefix,
        suffix,
        end.iter().map(|v| v.0.as_str()).collect::<String>(),
    ))
}

fn creation_transaction(
    state: &EditorState,
    selection: &Range<usize>,
    values: &ResourceValues,
    text: &str,
) -> Result<Transaction, String> {
    let mut select = state.tr();
    select.set_selection(TextSelection::create(
        state.doc(),
        selection.start,
        Some(selection.end),
    ));
    let selected = state.apply(select).map_err(message)?;
    let ty = state.schema().mark_type("link").ok_or("链接格式尚未安装")?;
    let attrs = std::collections::BTreeMap::from([
        ("href".into(), json!(values.url)),
        ("title".into(), json!(values.title)),
    ]);
    let command = nsh_base::rich_editor::extensions::link::set_link(ty, attrs, vec![]);
    let mut made = None;
    if !command(&selected, Some(&mut |tr| made = Some(tr))) {
        return Err("此地址或文字上下文不能创建链接".into());
    }
    let mut tr = made.ok_or("链接命令没有生成事务")?;
    if selection.is_empty() {
        if text.is_empty() || text.contains(['\n', '\r']) {
            return Err("请输入单行链接文字".into());
        }
        tr.insert_text(text, None, None);
        tr.set_stored_marks(None);
    }
    Ok(tr)
}

fn creation_edits(
    p: &Projection,
    wanted: &ResourceTarget,
    values: &ResourceValues,
    text: &str,
) -> Result<Vec<(Range<usize>, String)>, String> {
    let selection = wanted.selection.as_ref().ok_or("新链接选区缺失")?;
    if p.new_link_target(selection.clone())? != *wanted {
        return Err("链接选区已变化，请重新打开".into());
    }
    let destination = link_destination(values)?;
    if selection.is_empty() {
        if text.is_empty() || text.contains(['\n', '\r']) {
            return Err("请输入单行链接文字".into());
        }
        return Ok(vec![(
            wanted.source.clone(),
            format!("[{}]({destination})", label(text)),
        )]);
    }
    if text != wanted.visible {
        return Err("选中文字保持原样；修改文字请先返回正文".into());
    }
    let (prefix, close, reopen) = link_edges(p, &wanted.source)?;
    Ok(vec![
        (wanted.source.start..wanted.source.start, prefix),
        (
            wanted.source.end..wanted.source.end,
            format!("{close}({destination}){reopen}"),
        ),
    ])
}
fn targets(node: &mdast::Node, defs: &[&mdast::Definition], out: &mut Vec<ResourceTarget>) {
    if let Some(target) = target(node, defs) {
        out.push(target);
    }
    if let Some(children) = node.children() {
        for child in children {
            targets(child, defs, out);
        }
    }
}

fn bracket_end(raw: &str, start: usize) -> Option<usize> {
    let mut escaped = false;
    let mut depth = 1;
    for (at, ch) in raw[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '[' {
            depth += 1;
        }
        if ch == ']' {
            depth -= 1;
            if depth == 0 {
                return Some(start + at);
            }
        }
    }
    None
}
fn smallest(range: Range<usize>, old: &str, new: String) -> Option<(Range<usize>, String)> {
    if old == new {
        return None;
    }
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, z)| a == z)
        .map(|(ch, _)| ch.len_utf8())
        .sum();
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, z)| a == z)
        .map(|(ch, _)| ch.len_utf8())
        .sum();
    Some((
        range.start + prefix..range.end - suffix,
        new[prefix..new.len() - suffix].to_owned(),
    ))
}
fn edits(
    p: &Projection,
    wanted: &ResourceTarget,
    values: &ResourceValues,
) -> Result<Vec<(Range<usize>, String)>, String> {
    if values.url.is_empty()
        || values.url.contains(['\n', '\r', '<', '>'])
        || values
            .title
            .as_ref()
            .is_some_and(|t| t.contains(['\n', '\r']))
        || values.alt.contains(['\n', '\r'])
    {
        return Err("地址不能为空，地址和属性须为单行文字".into());
    }
    let tree = markdown::to_mdast(&p.source, &parse_options()).map_err(message)?;
    let mut defs = Vec::new();
    definitions(&tree, &mut defs);
    let mut all = Vec::new();
    targets(&tree, &defs, &mut all);
    let actual = all
        .into_iter()
        .find(|a| a.source == wanted.source && a.kind == wanted.kind)
        .ok_or("该链接或图片已变化，请重新打开属性")?;
    if actual != *wanted {
        return Err("该链接或图片已变化，请重新打开属性".into());
    }
    if actual.values == *values {
        return Ok(vec![]);
    }
    let suffix = values
        .title
        .as_ref()
        .map(|v| format!(" \"{}\"", title(v)))
        .unwrap_or_default();
    let destination = format!("<{}>{suffix}", destination(&values.url));
    // Check that Markdown reads exactly what the form requested, before any
    // transaction is published (no silent URL/entity coercion).
    let check =
        markdown::to_mdast(&format!("[x]({destination})"), &parse_options()).map_err(message)?;
    let mut parsed = Vec::new();
    targets(&check, &[], &mut parsed);
    if !parsed
        .first()
        .is_some_and(|t| t.values.url == values.url && t.values.title == values.title)
    {
        return Err("此地址或标题不能按当前 Markdown 语法保真写入".into());
    }
    let raw = &p.source[actual.source.clone()];
    let mut changes = Vec::new();
    if let Some(id) = &actual.reference {
        if actual.values.url != values.url || actual.values.title != values.title {
            let definition = defs
                .iter()
                .find(|d| key(&d.identifier) == key(id))
                .ok_or("引用定义已不存在")?;
            let position = definition.position.as_ref().ok_or("引用定义缺少源范围")?;
            let span = position.start.offset..position.end.offset;
            let original = &p.source[span.clone()];
            let end = bracket_end(original, 1).ok_or("引用定义前缀不能映射")?;
            if original.as_bytes().get(end + 1) != Some(&b':') {
                return Err("引用定义前缀不能映射".into());
            }
            let prefix = end
                + 2
                + original[end + 2..]
                    .bytes()
                    .take_while(|b| matches!(b, b' ' | b'\t'))
                    .count();
            if let Some(edit) = smallest(
                span,
                original,
                format!("{}{destination}", &original[..prefix]),
            ) {
                changes.push(edit);
            }
        }
        if actual.kind == ResourceKind::Image && actual.values.alt != values.alt {
            let replacement = format!("![{}][{id}]", label(&values.alt));
            if let Some(edit) = smallest(actual.source.clone(), raw, replacement) {
                changes.push(edit);
            }
        }
    } else {
        let replacement = if actual.kind == ResourceKind::Image {
            let alt = if actual.values.alt == values.alt {
                let end = bracket_end(raw, 2).ok_or("图片文字不能映射")?;
                raw[2..end].to_owned()
            } else {
                label(&values.alt)
            };
            format!("![{alt}]({destination})")
        } else {
            let text = if raw.starts_with('[') {
                let end = bracket_end(raw, 1).ok_or("链接文字不能映射")?;
                raw[1..end].to_owned()
            } else {
                label(&actual.visible)
            };
            format!("[{text}]({destination})")
        };
        if let Some(edit) = smallest(actual.source.clone(), raw, replacement) {
            changes.push(edit);
        }
    }
    changes.sort_by_key(|(range, _)| range.start);
    Ok(changes)
}

impl Projection {
    fn new_link_target(&self, selection: Range<usize>) -> Result<ResourceTarget, String> {
        if selection.start > selection.end || selection.end > self.doc.content().size() {
            return Err("链接选区范围无效".into());
        }
        let a = self.doc.resolve(selection.start).map_err(message)?;
        let z = self.doc.resolve(selection.end).map_err(message)?;
        if !a.same_parent(&z) || !a.parent().is_textblock() || a.parent().ty().name() == "codeBlock"
        {
            return Err("请在同一正文段、标题或单元格内选择链接文字".into());
        }
        let ty = self
            .doc
            .ty()
            .schema()
            .mark_type("link")
            .ok_or("链接格式尚未安装")?;
        if !a.parent().ty().allows_mark_type(&ty) {
            return Err("当前文字块不支持链接".into());
        }
        let mut base = 0;
        let mut found = None;
        for block in &self.blocks {
            let end = base + block.node.node_size();
            if selection.start > base && selection.end < end {
                if block.opaque {
                    return Err("原样块请使用该块的“编辑源码”".into());
                }
                found = Some((block, base));
                break;
            }
            base = end;
        }
        let (block, base) = found.ok_or("链接选区不能映射到正文块")?;
        let mut plain = true;
        if selection.is_empty() {
            plain = !a
                .marks()
                .iter()
                .any(|m| matches!(m.ty.name(), "link" | "code"));
        } else {
            self.doc
                .nodes_between(selection.start, selection.end, &mut |node, _, _, _| {
                    if node.is_text() {
                        plain &= !node
                            .marks()
                            .iter()
                            .any(|m| matches!(m.ty.name(), "link" | "code"));
                    } else if node.is_leaf() {
                        plain = false;
                    }
                    true
                });
        }
        if !plain {
            return Err("请只选择普通或强调文字；已有链接、行内代码和图片请分别编辑".into());
        }
        let source = if selection.is_empty() {
            let at = self
                .source_position(selection.start)
                .ok_or("此光标位置不能保真插入链接，请选择文字后重试")?;
            at..at
        } else {
            let mut at = selection.start;
            let mut range = None::<Range<usize>>;
            for unit in &block.text_map.units {
                let rich = base + unit.rich.start..base + unit.rich.end;
                if rich.start < selection.end && rich.end > selection.start {
                    if rich.start != at || rich.end > selection.end {
                        return Err("链接选区不能拆分转义、实体或Unicode字符".into());
                    }
                    at = rich.end;
                    match &mut range {
                        Some(range) => range.end = unit.source.end,
                        None => range = Some(unit.source.clone()),
                    }
                }
            }
            if at != selection.end {
                return Err("链接选区的文字范围不能保真映射".into());
            }
            let mut range = range.ok_or("请先选择链接文字")?;
            let tree = markdown::to_mdast(&self.source, &parse_options()).map_err(message)?;
            fn widen(
                node: &mdast::Node,
                units: &[super::text_map::Unit],
                selection: &Range<usize>,
                base: usize,
                range: &mut Range<usize>,
            ) {
                if format_node(node)
                    && let Some(position) = node.position()
                {
                    let owned: Vec<_> = units
                        .iter()
                        .filter(|unit| {
                            unit.source.start >= position.start.offset
                                && unit.source.end <= position.end.offset
                        })
                        .collect();
                    if !owned.is_empty()
                        && owned.iter().all(|unit| {
                            base + unit.rich.start >= selection.start
                                && base + unit.rich.end <= selection.end
                        })
                    {
                        range.start = range.start.min(position.start.offset);
                        range.end = range.end.max(position.end.offset);
                    }
                }
                if let Some(children) = node.children() {
                    for child in children {
                        widen(child, units, selection, base, range);
                    }
                }
            }
            widen(&tree, &block.text_map.units, &selection, base, &mut range);
            range
        };
        Ok(ResourceTarget {
            kind: ResourceKind::Link,
            source,
            values: ResourceValues {
                url: String::new(),
                title: None,
                alt: String::new(),
            },
            reference: None,
            visible: self
                .doc
                .text_between(selection.start, selection.end, None, None),
            selection: Some(selection),
        })
    }
    /// The same form edits an existing link or creates one over the exact
    /// selected text. A selection crossing an existing link never edits just
    /// whichever endpoint happens to have a link mark.
    pub fn link_target(&self, state: &EditorState) -> Result<ResourceTarget, String> {
        if state.doc() != &self.doc || state.selection().as_text().is_none() {
            return Err("链接需要当前正文中的文字选区".into());
        }
        let selection = state.selection();
        if let Ok(target) = self
            .resource_at(selection.head(), ResourceKind::Link)
            .or_else(|_| self.resource_at(selection.from(), ResourceKind::Link))
        {
            let a = self.source_position(selection.from());
            let z = self.source_position(selection.to());
            if selection.empty()
                || a.zip(z)
                    .is_some_and(|(a, z)| a >= target.source.start && z <= target.source.end)
            {
                return Ok(target);
            }
        }
        self.new_link_target(selection.from()..selection.to())
    }
    pub fn insert_link_transaction(
        &self,
        state: &EditorState,
        target: &ResourceTarget,
        values: &ResourceValues,
        text: &str,
    ) -> Result<Transaction, String> {
        if state.doc() != &self.doc {
            return Err("属性面板版本已过期，请重新打开".into());
        }
        let changes = creation_edits(self, target, values, text)?;
        let selection = target.selection.as_ref().ok_or("新链接选区缺失")?;
        let mut tr = creation_transaction(state, selection, values, text)?;
        let mut raw = self.source.clone();
        for (range, text) in changes.iter().rev() {
            raw.replace_range(range.clone(), text);
        }
        if Self::parse(state.schema(), raw)?.doc != *tr.doc() {
            return Err("此格式边界不能保真插入链接，请调整选区或使用源码".into());
        }
        tr.set_origin(TransactionOrigin::User);
        tr.set_meta(META,json!({"kind":"insertLink","from":target.source.start,"to":target.source.end,"richFrom":selection.start,"richTo":selection.end,"url":values.url,"title":values.title,"alt":"","text":text}));
        Ok(tr)
    }
    pub fn resource_at(&self, rich: usize, kind: ResourceKind) -> Result<ResourceTarget, String> {
        let byte = self
            .source_position(rich)
            .ok_or("请先把光标放入链接文字或选择图片")?;
        let tree = markdown::to_mdast(&self.source, &parse_options()).map_err(message)?;
        let mut defs = Vec::new();
        definitions(&tree, &mut defs);
        let mut all = Vec::new();
        targets(&tree, &defs, &mut all);
        all.into_iter()
            .filter(|t| t.kind == kind && byte >= t.source.start && byte <= t.source.end)
            .min_by_key(|t| t.source.len())
            .ok_or_else(|| "请先把光标放入链接文字或选择图片".into())
    }
    pub fn resource_transaction(
        &self,
        state: &EditorState,
        target: &ResourceTarget,
        values: &ResourceValues,
    ) -> Result<Transaction, String> {
        if state.doc() != &self.doc {
            return Err("属性面板版本已过期，请重新打开".into());
        }
        let changes = edits(self, target, values)?;
        let mut raw = self.source.clone();
        for (range, text) in changes.iter().rev() {
            raw.replace_range(range.clone(), text);
        }
        let projected = Self::parse(state.schema(), raw)?;
        let mut tr = state.tr();
        if !changes.is_empty() {
            tr.replace_with(
                0,
                state.doc().content().size(),
                projected.doc.content().clone(),
            )
            .map_err(message)?;
            let old_byte = self
                .source_position(state.selection().head())
                .unwrap_or(target.source.start);
            let mut byte = old_byte;
            let mut shift: isize = 0;
            for (range, text) in &changes {
                if range.end <= old_byte {
                    shift += text.len() as isize - range.len() as isize;
                    byte = old_byte.saturating_add_signed(shift);
                } else if range.start <= old_byte {
                    byte = range.start.saturating_add_signed(shift) + text.len();
                    break;
                }
            }
            let pos = projected
                .nearest_rich_position(byte)
                .ok_or("修改后的正文位置不能映射")?;
            tr.set_selection(TextSelection::create(&projected.doc, pos, None));
            tr.set_origin(TransactionOrigin::User);
            tr.set_meta(META, json!({"kind": if target.kind == ResourceKind::Link {"link"} else {"image"}, "from":target.source.start,"to":target.source.end,"url":values.url,"title":values.title,"alt":values.alt}));
        }
        Ok(tr)
    }
}

pub(super) fn transaction_edits(
    p: &Projection,
    next: &Node,
    applied: &[Transaction],
) -> Result<Option<Vec<(Range<usize>, String)>>, String> {
    let Some(meta) = applied.iter().find_map(|tr| tr.get_meta(META)) else {
        return Ok(None);
    };
    if applied
        .iter()
        .filter(|tr| tr.get_meta(META).is_some())
        .count()
        != 1
    {
        return Err("资源操作包含多个属性提交，原文未变".into());
    }
    if meta["kind"].as_str() == Some("insertLink") {
        let position = |key: &str| {
            meta[key]
                .as_u64()
                .and_then(|v| usize::try_from(v).ok())
                .ok_or("新链接范围无效")
        };
        let selection = position("richFrom")?..position("richTo")?;
        let target = p.new_link_target(selection.clone())?;
        if target.source != (position("from")?..position("to")?) {
            return Err("新链接源码范围与真实选区不一致".into());
        }
        let values = ResourceValues {
            url: meta["url"].as_str().ok_or("链接地址无效")?.into(),
            title: if meta["title"].is_null() {
                None
            } else {
                Some(meta["title"].as_str().ok_or("链接标题无效")?.into())
            },
            alt: String::new(),
        };
        let text = meta["text"].as_str().ok_or("链接文字无效")?;
        let changes = creation_edits(p, &target, &values, text)?;
        let state = EditorState::create(
            EditorStateConfig::new(p.doc.ty().schema().clone()).doc(p.doc.clone()),
        )
        .map_err(message)?;
        let expected = creation_transaction(&state, &selection, &values, text)?;
        if expected.doc() != next {
            return Err("新链接操作包含非目标文字改动，原文未变".into());
        }
        let mut raw = p.source.clone();
        for (range, text) in changes.iter().rev() {
            raw.replace_range(range.clone(), text);
        }
        if Projection::parse(next.ty().schema(), raw)?.doc != *next {
            return Err("新链接操作不能保真往返，原文未变".into());
        }
        return Ok(Some(changes));
    }
    let kind = match meta["kind"].as_str() {
        Some("link") => ResourceKind::Link,
        Some("image") => ResourceKind::Image,
        _ => return Err("资源操作类型无效".into()),
    };
    let from = meta["from"]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or("资源源范围无效")?;
    let to = meta["to"]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or("资源源范围无效")?;
    let tree = markdown::to_mdast(&p.source, &parse_options()).map_err(message)?;
    let mut defs = Vec::new();
    definitions(&tree, &mut defs);
    let mut all = Vec::new();
    targets(&tree, &defs, &mut all);
    let target = all
        .into_iter()
        .find(|t| t.kind == kind && t.source == (from..to))
        .ok_or("资源源范围已变化")?;
    let values = ResourceValues {
        url: meta["url"].as_str().ok_or("资源地址无效")?.into(),
        title: if meta["title"].is_null() {
            None
        } else {
            Some(meta["title"].as_str().ok_or("资源标题无效")?.into())
        },
        alt: meta["alt"].as_str().ok_or("图片替代文本无效")?.into(),
    };
    let changes = edits(p, &target, &values)?;
    let mut raw = p.source.clone();
    for (range, text) in changes.iter().rev() {
        raw.replace_range(range.clone(), text);
    }
    if Projection::parse(next.ty().schema(), raw)?.doc != *next {
        return Err("资源操作不能保真往返，原文未变".into());
    }
    Ok(Some(changes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_base::rich_editor::state::{EditorState, EditorStateConfig};
    #[test]
    fn link_creation_guard_binds_real_text_range_and_refuses_metadata_plus_an_unrelated_edit() {
        let manager = super::super::manager();
        let p =
            Projection::parse(manager.schema(), "plain\n\n<custom>stay</custom>\n".into()).unwrap();
        let state = EditorState::create(
            EditorStateConfig::new(manager.schema().clone())
                .doc(p.doc.clone())
                .selection(TextSelection::create(&p.doc, 1, Some(6))),
        )
        .unwrap();
        let target = p.link_target(&state).unwrap();
        let values = ResourceValues {
            url: "./new".into(),
            title: None,
            alt: String::new(),
        };
        let tr = p
            .insert_link_transaction(&state, &target, &values, "plain")
            .unwrap();
        let meta = tr.get_meta(META).unwrap().clone();
        let requested = tr.doc().clone();
        let (next, applied) = state.apply_transaction(tr).unwrap();
        assert!(
            p.verified_resource_transaction(next.doc(), &applied)
                .unwrap()
        );
        let mut forged = state.tr();
        forged
            .replace_with(0, state.doc().content().size(), requested.content().clone())
            .unwrap();
        let at = forged.doc().content().size() - 2;
        forged.insert_text("X", Some(at), Some(at));
        forged.set_meta(META, meta.clone());
        let (next, applied) = state.apply_transaction(forged).unwrap();
        assert!(
            p.verified_resource_transaction(next.doc(), &applied)
                .is_err()
        );
        let mut wrong_range = state.tr();
        wrong_range
            .replace_with(0, state.doc().content().size(), requested.content().clone())
            .unwrap();
        let mut altered = meta;
        altered["from"] = json!(1);
        wrong_range.set_meta(META, altered);
        let (next, applied) = state.apply_transaction(wrong_range).unwrap();
        assert!(
            p.verified_resource_transaction(next.doc(), &applied)
                .is_err()
        );
    }
    #[test]
    fn resource_guard_requires_real_ast_target_and_the_complete_requested_tree() {
        let manager = super::super::manager();
        let p = Projection::parse(
            manager.schema(),
            "[label](./old)\n\n<custom>stay</custom>".into(),
        )
        .unwrap();
        let state = EditorState::create(
            EditorStateConfig::new(manager.schema().clone()).doc(p.doc.clone()),
        )
        .unwrap();
        let target = p.resource_at(2, ResourceKind::Link).unwrap();
        let mut values = target.values.clone();
        values.url = "./new".into();
        let tr = p.resource_transaction(&state, &target, &values).unwrap();
        let (next, applied) = state.apply_transaction(tr).unwrap();
        assert!(
            p.verified_resource_transaction(next.doc(), &applied)
                .unwrap()
        );
        let mut unrelated = state.tr();
        unrelated.insert_text("X", Some(2), Some(2));
        unrelated.set_meta(META, json!({"kind":"link","from":target.source.start,"to":target.source.end,"url":"./new","title":null,"alt":""}));
        let (next, applied) = state.apply_transaction(unrelated).unwrap();
        assert!(
            p.verified_resource_transaction(next.doc(), &applied)
                .is_err()
        );
        let mut ordinary = state.tr();
        ordinary.insert_text("X", Some(2), Some(2));
        let (next, applied) = state.apply_transaction(ordinary).unwrap();
        assert!(
            !p.verified_resource_transaction(next.doc(), &applied)
                .unwrap()
        );
    }
}
