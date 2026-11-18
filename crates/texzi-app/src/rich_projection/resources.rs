//! Resource forms issue ordinary transactions; only verified AST-owned ranges
//! become Store edits. Reference resources update their shared definition.
use super::{Projection, message, parse_options};
use markdown::mdast;
use nsh_base::rich_editor::{
    model::Node,
    state::{EditorState, TextSelection, Transaction, TransactionOrigin},
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
    })
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
