//! Source-aware code commands. Language changes own one opening-fence token;
//! indentation owns only leading spaces; exit owns only new terminal trivia.
use super::{Projection, message, parse_options};
use markdown::mdast;
use nsh_base::rich_editor::{
    commands::exit_code,
    model::{Fragment, Node},
    state::{EditorState, TextSelection, Transaction, TransactionOrigin},
    transform::ReplaceStep,
};
use serde_json::{Value, json};
use std::ops::Range;

const META: &str = "texzi.code";
const TAB_SIZE: usize = 4;

#[derive(Clone, Debug)]
pub struct CodeTarget {
    pub position: usize,
    pub source: Range<usize>,
    pub language: Option<String>,
    pub fenced: bool,
    pub info: Option<String>,
}

fn code_nodes<'a>(node: &'a mdast::Node, out: &mut Vec<&'a mdast::Code>) {
    if let mdast::Node::Code(code) = node {
        out.push(code);
    }
    if let Some(children) = node.children() {
        for child in children {
            code_nodes(child, out);
        }
    }
}
fn current_position(state: &EditorState) -> Result<usize, String> {
    let from = state.selection().resolved_from();
    if from.parent().ty().name() != "codeBlock"
        || !from.same_parent(state.selection().resolved_to())
    {
        return Err("请先把光标放入代码块".into());
    }
    Ok(from.before(from.depth))
}
fn language_edit(
    p: &Projection,
    target: &CodeTarget,
    language: Option<&str>,
) -> Result<Vec<(Range<usize>, String)>, String> {
    if !target.fenced {
        return Err("缩进代码块没有语言标记，请保留原形式并在源码编辑".into());
    }
    if language.is_some_and(|s| s.is_empty() || s.chars().any(char::is_whitespace)) {
        return Err("语言标记须是单个非空词；留空表示清除语言".into());
    }
    if target.language.as_deref() == language {
        return Ok(vec![]);
    }
    if language.is_none() && target.info.is_some() {
        return Err("此围栏有附加信息，清空语言会改变其含义；可修改语言或编辑源码".into());
    }
    let raw = p
        .source
        .get(target.source.clone())
        .ok_or("代码块源范围无效")?;
    let line = raw.split('\n').next().ok_or("代码围栏首行不存在")?;
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    let marker = *line.as_bytes().get(indent).ok_or("代码围栏不存在")?;
    let run = line.as_bytes()[indent..]
        .iter()
        .take_while(|b| **b == marker)
        .count();
    let start = indent + run;
    let padding = line[start..]
        .bytes()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count();
    let a = start + padding;
    let end = a + line[a..]
        .find(char::is_whitespace)
        .unwrap_or(line.len() - a);
    let text = language
        .unwrap_or("")
        .replace('&', "&amp;")
        .replace('\\', "\\\\");
    Ok(vec![(
        target.source.start + a..target.source.start + end,
        text,
    )])
}
fn apply(p: &Projection, next: &Node, edits: &[(Range<usize>, String)]) -> Result<(), String> {
    let mut raw = p.source.clone();
    for (range, text) in edits.iter().rev() {
        raw.replace_range(range.clone(), text);
    }
    if Projection::parse(next.ty().schema(), raw)?.doc != *next {
        return Err("该代码操作不能保真往返，原文未变".into());
    }
    Ok(())
}
fn exit_edits(p: &Projection, next: &Node) -> Result<Vec<(Range<usize>, String)>, String> {
    for separator in ["\n", "\n\n", "\n\n\n", "\n\n\n\n"] {
        let edits = vec![(p.source.len()..p.source.len(), separator.to_owned())];
        if apply(p, next, &edits).is_ok() {
            return Ok(edits);
        }
    }
    Err("此代码块尚不能在文末保真退出；请检查围栏是否闭合，或切到源码编辑".into())
}

impl Projection {
    pub fn code_target(&self, position: usize) -> Result<CodeTarget, String> {
        let mut base = 0;
        for block in &self.blocks {
            let end = base + block.node.node_size();
            if position >= base && position < end {
                if block.opaque {
                    return Err("原样块请使用该块的“编辑源码”".into());
                }
                let ast = markdown::to_mdast(&self.source, &parse_options()).map_err(message)?;
                let mut codes = Vec::new();
                code_nodes(&ast, &mut codes);
                codes.retain(|code| {
                    code.position.as_ref().is_some_and(|p| {
                        p.start.offset >= block.source.start && p.end.offset <= block.source.end
                    })
                });
                let mut rich = Vec::new();
                if block.node.ty().name() == "codeBlock" {
                    rich.push((base, block.node.clone()));
                }
                block.node.descendants(&mut |node, at, _, _| {
                    if node.ty().name() == "codeBlock" {
                        rich.push((base + at + 1, node.clone()));
                    }
                    true
                });
                if codes.len() != rich.len()
                    || codes.iter().zip(&rich).any(|(a, (_, z))| {
                        a.value != z.text_content()
                            || a.lang.as_deref()
                                != z.attrs().get("language").and_then(Value::as_str)
                    })
                {
                    return Err("代码块源位置不能可靠映射，请编辑源码".into());
                }
                let index = rich
                    .iter()
                    .position(|(at, _)| *at == position)
                    .ok_or("当前位置不是代码块")?;
                let code = codes[index];
                let span = code.position.as_ref().ok_or("代码块源位置缺失")?;
                let source = span.start.offset..span.end.offset;
                let first = self.source[source.clone()]
                    .split('\n')
                    .next()
                    .unwrap_or("")
                    .trim_start_matches(' ');
                return Ok(CodeTarget {
                    position,
                    source,
                    language: code.lang.clone(),
                    fenced: first.starts_with("```") || first.starts_with("~~~"),
                    info: code.meta.clone(),
                });
            }
            base = end;
        }
        Err("代码块位置已变化".into())
    }
    pub fn current_code_target(&self, state: &EditorState) -> Result<CodeTarget, String> {
        if state.doc() != &self.doc {
            return Err("代码视图版本已变化".into());
        }
        self.code_target(current_position(state)?)
    }
    pub fn code_language_transaction(
        &self,
        state: &EditorState,
        position: usize,
        language: Option<&str>,
    ) -> Result<Transaction, String> {
        if state.doc() != &self.doc {
            return Err("代码视图版本已变化".into());
        }
        let target = self.code_target(position)?;
        let edits = language_edit(self, &target, language)?;
        let mut tr = state.tr();
        if !edits.is_empty() {
            tr.set_node_attribute(
                position,
                "language",
                language.map_or(Value::Null, Value::from),
            )
            .map_err(message)?;
            apply(self, tr.doc(), &edits)?;
            tr.set_origin(TransactionOrigin::User);
            tr.set_meta(META,json!({"action":"language","position":position,"sourceStart":target.source.start,"sourceEnd":target.source.end,"language":language}));
        }
        Ok(tr)
    }
    pub fn code_exit_transaction(&self, state: &EditorState) -> Result<Transaction, String> {
        let target = self.current_code_target(state)?;
        let node = state
            .doc()
            .node_at(target.position)
            .ok_or("代码块位置已变化")?;
        let after = target.position + node.node_size();
        let mut opaque = Vec::new();
        let mut base = 0;
        for block in &self.blocks {
            let end = base + block.node.node_size();
            if block.opaque {
                opaque.push(base..end);
            }
            base = end;
        }
        let mut destination = None;
        state.doc().descendants(&mut |node, at, _, _| {
            if destination.is_none()
                && at >= after
                && node.is_textblock()
                && !opaque.iter().any(|span| span.contains(&at))
                && self.source_position(at + 1).is_some()
            {
                destination = Some(at + 1);
            }
            true
        });
        if let Some(pos) = destination {
            let mut tr = state.tr();
            tr.set_selection(TextSelection::create(state.doc(), pos, None));
            tr.set_origin(TransactionOrigin::User);
            return Ok(tr);
        }
        let mut tr = if after == state.doc().content().size() {
            let mut made = None;
            if !exit_code(state, Some(&mut |tr| made = Some(tr))) {
                return Err("当前代码块不能退出".into());
            }
            made.ok_or("代码退出命令没有生成事务")?
        } else {
            let mut tr = state.tr();
            let pos = state.doc().content().size();
            let paragraph = state
                .schema()
                .node("paragraph", None, Fragment::empty(), vec![])
                .map_err(message)?;
            tr.insert(pos, Fragment::from(vec![paragraph]))
                .map_err(message)?;
            tr.set_selection(TextSelection::create(tr.doc(), pos + 1, None));
            tr
        };
        exit_edits(self, tr.doc())?;
        tr.set_origin(TransactionOrigin::User);
        tr.set_meta(META,json!({"action":"exit","position":target.position,"sourceStart":target.source.start,"sourceEnd":target.source.end}));
        Ok(tr)
    }
}

fn indentation(
    p: &Projection,
    next: &Node,
    applied: &[Transaction],
) -> Result<Option<Vec<(Range<usize>, String)>>, String> {
    let steps: Vec<_> = applied.iter().flat_map(|tr| tr.steps.iter()).collect();
    let [step] = steps.as_slice() else {
        return Ok(None);
    };
    let Some(step) = step.as_any().downcast_ref::<ReplaceStep>() else {
        return Ok(None);
    };
    if step.structure
        || step.slice.open_start != 0
        || step.slice.open_end != 0
        || !(0..step.slice.content.child_count()).all(|i| step.slice.content.child(i).is_text())
    {
        return Ok(None);
    }
    let from = p.doc.resolve(step.from).map_err(message)?;
    let to = p.doc.resolve(step.to).map_err(message)?;
    if from.parent().ty().name() != "codeBlock" || !from.same_parent(&to) {
        return Ok(None);
    }
    p.code_target(from.before(from.depth))?;
    let old = p
        .doc
        .text_between(step.from, step.to, Some("\n"), Some("\n"));
    // A caret insertion already has an exact zero-width seam, including the
    // existing empty-fence separator materialization in the normal bridge.
    if old.is_empty() {
        return Ok(None);
    }
    let inserted: String = (0..step.slice.content.child_count())
        .filter_map(|i| step.slice.content.child(i).text())
        .collect();
    let pad = " ".repeat(TAB_SIZE);
    let lines: Vec<_> = old.split('\n').collect();
    let added = lines
        .iter()
        .map(|line| format!("{pad}{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let removed = lines
        .iter()
        .map(|line| {
            &line[line
                .bytes()
                .take_while(|b| *b == b' ')
                .count()
                .min(TAB_SIZE)..]
        })
        .collect::<Vec<_>>()
        .join("\n");
    let adding = inserted == added;
    if !adding && (old.is_empty() || inserted != removed || inserted == old) {
        return Ok(None);
    }
    let mut offset = 0;
    let mut edits = Vec::new();
    for line in lines {
        let rich = step.from + offset;
        let a = p.source_position(rich).ok_or("代码缩进位置不能映射")?;
        if adding {
            edits.push((a..a, pad.clone()));
        } else {
            let count = line
                .bytes()
                .take_while(|b| *b == b' ')
                .count()
                .min(TAB_SIZE);
            if count > 0 {
                let z = p
                    .source_position(rich + count)
                    .ok_or("代码反缩进位置不能映射")?;
                edits.push((a..z, String::new()));
            }
        }
        offset += line.encode_utf16().count() + 1;
    }
    apply(p, next, &edits)?;
    Ok(Some(edits))
}

pub(super) fn transaction_edits(
    p: &Projection,
    next: &Node,
    applied: &[Transaction],
) -> Result<Option<Vec<(Range<usize>, String)>>, String> {
    let Some(meta) = applied.iter().find_map(|tr| tr.get_meta(META)) else {
        return indentation(p, next, applied);
    };
    let position = meta["position"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or("代码操作位置无效")?;
    let target = p.code_target(position)?;
    if meta["sourceStart"].as_u64() != Some(target.source.start as u64)
        || meta["sourceEnd"].as_u64() != Some(target.source.end as u64)
    {
        return Err("代码操作源范围已变化".into());
    }
    let edits = match meta["action"].as_str() {
        Some("language") => {
            let language = if meta["language"].is_null() {
                None
            } else {
                Some(meta["language"].as_str().ok_or("语言标记无效")?)
            };
            language_edit(p, &target, language)?
        }
        Some("exit") => {
            let count = p.doc.child_count();
            if next.child_count() != count + 1
                || (0..count).any(|i| p.doc.child(i) != next.child(i))
                || next.child(count).ty().name() != "paragraph"
                || next.child(count).content().size() != 0
            {
                return Err("代码退出操作改变了其它正文，原文未变".into());
            }
            exit_edits(p, next)?
        }
        _ => return Err("代码操作类型无效".into()),
    };
    apply(p, next, &edits)?;
    Ok(Some(edits))
}
