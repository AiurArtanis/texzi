//! Shared command entry points. Forms reuse the existing view controllers;
//! document actions use the same session, source bridge and Store as typing.
use crate::{
    editor_owner::NativeView,
    rich_owner::ModeGate,
    rich_projection::Projection,
    table_controls::{TableAction, action_transaction, cell_position},
};
use nsh_base::{
    editor::ExternalDocument,
    rich_editor::{
        commands::toggle_mark,
        state::{EditorState, TextSelection, Transaction, TransactionOrigin},
        tables::{cell_around, selected_rect},
    },
};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FormRequest {
    Link,
    CodeLanguage,
    TableAlignment,
    Table,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Applied { body_changed: bool },
    Form(FormRequest),
}

const COMMAND_IDS: &[&str] = &[
    "texzi.markdown.bold",
    "texzi.markdown.italic",
    "texzi.markdown.strikethrough",
    "texzi.markdown.inlineCode",
    "texzi.markdown.toggleTask",
    "texzi.markdown.exitCodeBlock",
    "texzi.markdown.insertLink",
    "texzi.markdown.setCodeLanguage",
    "texzi.markdown.table.insertRow",
    "texzi.markdown.table.deleteRow",
    "texzi.markdown.table.insertColumn",
    "texzi.markdown.table.deleteColumn",
    "texzi.markdown.table.alignColumn",
];
pub(crate) fn handles(id: &str) -> bool {
    COMMAND_IDS.contains(&id)
}

fn mark_name(id: &str) -> Option<&'static str> {
    match id {
        "texzi.markdown.bold" => Some("bold"),
        "texzi.markdown.italic" => Some("italic"),
        "texzi.markdown.strikethrough" => Some("strike"),
        "texzi.markdown.inlineCode" => Some("code"),
        _ => None,
    }
}
fn current(
    gate: &ModeGate,
    source: &NativeView,
    id: &str,
    writable: bool,
) -> Result<(EditorState, Projection), String> {
    if !handles(id) {
        return Err("此 Markdown 命令尚未接入；当前文档未变".into());
    }
    current_context(gate, source, writable)
}
fn current_context(
    gate: &ModeGate,
    source: &NativeView,
    writable: bool,
) -> Result<(EditorState, Projection), String> {
    let state = current_state(gate, writable)?;
    let snapshot = source.snapshot()?;
    let projection = Projection::parse(state.schema(), snapshot.rope.to_string())?;
    if state.doc() != &projection.doc {
        return Err("正文版本已变化，请重新执行命令".into());
    }
    Ok((state, projection))
}

fn current_state(gate: &ModeGate, writable: bool) -> Result<EditorState, String> {
    if gate.busy() {
        return Err("请先完成或取消中文组合，再执行 Markdown 命令".into());
    }
    if !gate.rich_mode.get() {
        return Err("此命令只适用于所见即所得正文，请先切换模式".into());
    }
    if !writable {
        return Err("当前文档为只读，不能执行此 Markdown 命令".into());
    }
    let session = gate.session();
    if session.composing() {
        return Err("请先完成或取消中文组合，再执行 Markdown 命令".into());
    }
    Ok(session.state())
}

fn task_target(state: &EditorState) -> Result<(usize, bool), String> {
    let selection = state.selection();
    let at = |position: &nsh_base::rich_editor::model::ResolvedPos| {
        (1..=position.depth).rev().find_map(|depth| {
            let node = position.node(depth);
            (node.ty().name() == "taskItem").then(|| {
                (
                    position.before(depth),
                    node.attrs()
                        .get("checked")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                )
            })
        })
    };
    let anchor = at(selection.resolved_anchor()).ok_or("请先把光标放入一个任务项")?;
    let head = at(selection.resolved_head()).ok_or("请先把光标放入一个任务项")?;
    if anchor.0 != head.0 {
        return Err("切换任务状态需要位于同一个任务项内".into());
    }
    Ok(head)
}

fn table_target(state: &EditorState) -> Result<(usize, usize, usize), String> {
    let selection = state.selection();
    if selection.as_text().is_none() {
        return Err("请先把文字光标放入一个表格单元格".into());
    }
    let anchor = cell_around(selection.resolved_anchor()).ok_or("请先把光标放入表格单元格")?;
    let head = cell_around(selection.resolved_head()).ok_or("请先把光标放入表格单元格")?;
    if anchor.pos != head.pos {
        return Err("此表格命令需要同一单元格内的选区".into());
    }
    let rect = selected_rect(state).ok_or("请先把光标放入表格单元格")?;
    if !rect.map.problems.is_empty()
        || rect.rect.right != rect.rect.left + 1
        || rect.rect.bottom != rect.rect.top + 1
    {
        return Err("此命令仅支持没有合并单元格的简单表格".into());
    }
    let mut base = 0;
    let block = (0..state.doc().child_count())
        .find(|i| {
            let at = base;
            base += state.doc().child(*i).node_size();
            at + 1 == rect.table_start
        })
        .ok_or("嵌套表格请编辑源码")?;
    Ok((block, rect.rect.top, rect.rect.left))
}

fn check_context(state: &EditorState, projection: &Projection, id: &str) -> Result<(), String> {
    if let Some(name) = mark_name(id) {
        let selection = state.selection();
        if selection.as_text().is_none()
            || !selection
                .resolved_from()
                .same_parent(selection.resolved_to())
        {
            return Err("格式命令当前需要同一文字段内的连续选区；跨块请编辑源码".into());
        }
        let mark = state.schema().mark_type(name).ok_or("此格式尚未安装")?;
        if !selection
            .resolved_from()
            .parent()
            .ty()
            .allows_mark_type(&mark)
        {
            return Err("当前文字块不支持此格式".into());
        }
        let mut base = 0;
        for block in &projection.blocks {
            let end = base + block.node.node_size();
            if block.opaque && selection.from() < end && selection.to() > base {
                return Err("原样块请使用该块的“编辑源码”".into());
            }
            base = end;
        }
    } else if id == "texzi.markdown.toggleTask" {
        task_target(state)?;
    } else if id.starts_with("texzi.markdown.table.") {
        table_target(state)?;
    } else if id == "texzi.markdown.exitCodeBlock" || id == "texzi.markdown.setCodeLanguage" {
        let target = projection.current_code_target(state)?;
        if id == "texzi.markdown.setCodeLanguage" && !target.fenced {
            return Err("缩进代码块没有语言标记，请编辑源码".into());
        }
    } else if id == "texzi.markdown.insertLink" {
        projection.link_target(state)?;
    }
    Ok(())
}

/// Read-only context availability: no command is dispatched or run as a probe.
pub(crate) fn availability(
    gate: &ModeGate,
    source: &NativeView,
    id: &str,
    writable: bool,
) -> Result<(), String> {
    let (state, projection) = current(gate, source, id, writable)?;
    check_context(&state, &projection, id)
}

/// Only visible palette rows use the mounted projection for their current revision.
/// A command still revalidates the live source when invoked.
pub(crate) fn availability_all<'a>(
    gate: &ModeGate,
    source: &NativeView,
    writable: bool,
    ids: &[&'a str],
) -> BTreeMap<&'a str, Result<(), String>> {
    const STALE: &str = "正文版本已变化，请重新执行命令";
    let results = current_state(gate, writable).and_then(|state| {
        let snapshot = source.body_snapshot().ok_or(STALE)?;
        gate.with_visible_projection(snapshot.revision.0, |projection| {
            if state.doc() != &projection.doc {
                return Err(STALE.to_owned());
            }
            Ok(ids
                .iter()
                .map(|&id| (id, check_context(&state, projection, id)))
                .collect())
        })
        .ok_or_else(|| STALE.to_owned())?
    });
    results.unwrap_or_else(|error| ids.iter().map(|&id| (id, Err(error.clone()))).collect())
}

fn parameter<'a>(args: &'a Value, name: &str, choices: &[&str]) -> Result<&'a str, String> {
    let object = args.as_object().ok_or("命令参数必须是对象")?;
    if object.len() != 1 || !object.contains_key(name) {
        return Err(format!("此命令需要明确的 {name} 参数，不能静默选择默认值"));
    }
    let value = object[name]
        .as_str()
        .ok_or_else(|| format!("{name} 参数必须是文字"))?;
    if !choices.contains(&value) {
        return Err(format!("无效的 {name} 参数：{value}"));
    }
    Ok(value)
}

fn build(
    state: &EditorState,
    projection: &Projection,
    id: &str,
    args: Option<&Value>,
) -> Result<Result<Transaction, FormRequest>, String> {
    if args.is_some()
        && !matches!(
            id,
            "texzi.markdown.table.insertRow"
                | "texzi.markdown.table.insertColumn"
                | "texzi.markdown.table.alignColumn"
        )
    {
        return Err("此命令不接受直接参数，请使用普通入口或现有属性面板".into());
    }
    if let Some(name) = mark_name(id) {
        let mark = state.schema().mark_type(name).ok_or("此格式尚未安装")?;
        let mut made = None;
        if !toggle_mark(mark, None, true)(state, Some(&mut |tr| made = Some(tr))) {
            return Err("当前选区不能切换此格式".into());
        }
        return Ok(Ok(made.ok_or("格式命令未生成事务")?));
    }
    if id == "texzi.markdown.toggleTask" {
        let (position, checked) = task_target(state)?;
        let mut tr = state.tr();
        tr.set_node_attribute(position, "checked", Value::Bool(!checked))
            .map_err(|e| e.to_string())?;
        return Ok(Ok(tr));
    }
    if id == "texzi.markdown.exitCodeBlock" {
        return Ok(Ok(projection.code_exit_transaction(state)?));
    }
    if id == "texzi.markdown.insertLink" {
        return Ok(Err(FormRequest::Link));
    }
    if id == "texzi.markdown.setCodeLanguage" {
        return Ok(Err(FormRequest::CodeLanguage));
    }
    let (block, row, column) = table_target(state)?;
    let action = match id {
        "texzi.markdown.table.insertRow" | "texzi.markdown.table.insertColumn" => {
            // The approved registry explicitly defines position=after when
            // omitted. Explicit parameters still pass the strict validator.
            let before = match args {
                Some(args) => parameter(args, "position", &["before", "after"])? == "before",
                None => false,
            };
            match (id.ends_with("insertRow"), before) {
                (true, true) if row == 0 => {
                    return Err("不能在表头之前插入正文行，请选择表头之后".into());
                }
                (true, true) => TableAction::RowBefore,
                (true, false) => TableAction::RowAfter,
                (false, true) => TableAction::ColumnBefore,
                (false, false) => TableAction::ColumnAfter,
            }
        }
        "texzi.markdown.table.deleteRow" => TableAction::DeleteRow,
        "texzi.markdown.table.deleteColumn" => TableAction::DeleteColumn,
        "texzi.markdown.table.alignColumn" => {
            let Some(args) = args else {
                return Ok(Err(FormRequest::TableAlignment));
            };
            match parameter(args, "alignment", &["none", "left", "center", "right"])? {
                "none" => TableAction::AlignDefault,
                "left" => TableAction::AlignLeft,
                "center" => TableAction::AlignCenter,
                "right" => TableAction::AlignRight,
                _ => unreachable!(),
            }
        }
        _ => return Err("此 Markdown 命令尚未接入".into()),
    };
    let mut tr = action_transaction(state, block, row, column, action)?;
    // The command registry promises focus in the newly inserted row/column.
    // The common table source action and META remain unchanged.
    let inserted = match action {
        TableAction::RowBefore => Some((row, column)),
        TableAction::RowAfter => Some((row + 1, column)),
        TableAction::ColumnBefore => Some((row, column)),
        TableAction::ColumnAfter => Some((row, column + 1)),
        _ => None,
    };
    if let Some((row, column)) = inserted {
        let at = cell_position(tr.doc(), block, row, column).ok_or("新单元格位置不能映射")?;
        tr.set_selection(TextSelection::create(tr.doc(), at, None));
    }
    Ok(Ok(tr))
}

fn preflight(projection: &Projection, tr: &Transaction) -> Result<(), String> {
    tr.doc().check().map_err(|e| e.to_string())?;
    let edits = projection.transaction_edits(tr.doc(), std::slice::from_ref(tr))?;
    if edits.is_empty() && &projection.doc != tr.doc() {
        return Err("此命令不能局部映射到原文，原文未变".into());
    }
    let mut raw = projection.source.clone();
    for (range, text) in edits.iter().rev() {
        raw.replace_range(range.clone(), text);
    }
    let next = Projection::parse(tr.schema(), raw)?;
    if &next.doc != tr.doc() {
        return Err("此命令不能保真往返，原文未变".into());
    }
    let selection = tr.selection();
    if next.source_position(selection.anchor()).is_none()
        || next.source_position(selection.head()).is_none()
    {
        return Err("命令结果的选区不能映射到源码，原文未变".into());
    }
    Ok(())
}

fn execute(
    gate: &ModeGate,
    source: &NativeView,
    id: &str,
    args: Option<&Value>,
    writable: bool,
) -> Result<Outcome, String> {
    let (state, projection) = current(gate, source, id, writable)?;
    check_context(&state, &projection, id)?;
    let mut tr = match build(&state, &projection, id, args)? {
        Ok(tr) => tr,
        Err(form) => return Ok(Outcome::Form(form)),
    };
    tr.set_origin(TransactionOrigin::User);
    preflight(&projection, &tr)?;
    let expected = tr.doc().clone();
    let expected_selection = tr.selection().to_json();
    let expected_marks = tr
        .stored_marks_set()
        .then(|| tr.stored_marks().map(<[_]>::to_vec));
    let body_changed = state.doc() != &expected;
    let revision = source.snapshot()?.revision;
    gate.status.set(String::new());
    let session = gate.session();
    session.dispatch(tr).map_err(|e| e.to_string())?;
    let status = gate.status.get();
    if !status.is_empty() {
        return Err(status);
    }
    let accepted = session.state();
    if accepted.doc() != &expected
        || accepted.selection().to_json() != expected_selection
        || expected_marks.is_some_and(|marks| accepted.stored_marks() != marks.as_deref())
    {
        return Err("正文未确认预期的命令结果，请检查当前选区与状态".into());
    }
    let committed = source.snapshot()?;
    if body_changed && committed.revision != revision.checked_add(1).ok_or("文档版本计数已耗尽")?
        || !body_changed && committed.revision != revision
    {
        return Err("命令的共享文档版本回执与预期不符".into());
    }
    Ok(Outcome::Applied { body_changed })
}

pub(crate) fn run(
    gate: &ModeGate,
    source: &NativeView,
    id: &str,
    writable: bool,
) -> Result<Outcome, String> {
    execute(gate, source, id, None, writable)
}

pub(crate) fn run_with(
    gate: &ModeGate,
    source: &NativeView,
    id: &str,
    args: &Value,
    writable: bool,
) -> Result<Outcome, String> {
    if args.as_object().is_none_or(|object| object.is_empty()) {
        return Err("请提供明确的命令参数，当前文档未变".into());
    }
    execute(gate, source, id, Some(args), writable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_owner::NativeView;
    use nsh_core::prelude::*;
    use serde_json::json;
    use std::rc::Rc;

    fn runtime(f: impl FnOnce()) {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        f();
    }
    fn setup(raw: &str) -> (Rc<NativeView>, Rc<ModeGate>) {
        let source = NativeView::fixture(raw);
        let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
        gate.to_rich();
        assert!(gate.rich_mode.get(), "{}", gate.status.get());
        (source, gate)
    }
    fn bytes(source: &NativeView) -> Vec<u8> {
        source.inspect(|store, id| {
            store
                .snapshot(store.view(id).unwrap().doc)
                .unwrap()
                .body
                .to_utf8()
        })
    }
    fn audit(source: &NativeView) -> String {
        source.inspect(|store, id| {
            let doc = store.view(id).unwrap().doc;
            let s = store.snapshot(doc).unwrap();
            format!(
                "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                s.revision,
                s.token,
                s.body.to_utf8(),
                s.body.eols(),
                s.body.origins(),
                store.history_depth(doc),
                store.dirty(doc)
            )
        })
    }
    fn select(source: &NativeView, gate: &ModeGate, needle: &str, whole: bool) {
        let state = gate.session().state();
        let p =
            Projection::parse(state.schema(), source.snapshot().unwrap().rope.to_string()).unwrap();
        let start = p.source.find(needle).unwrap();
        let a = p.rich_position(start).unwrap();
        let h = if whole {
            p.rich_position(start + needle.len()).unwrap()
        } else {
            a
        };
        let mut tr = state.tr();
        tr.set_selection(TextSelection::create(state.doc(), a, Some(h)));
        gate.session().dispatch(tr).unwrap();
        assert!(gate.status.get().is_empty(), "{}", gate.status.get());
    }
    #[test]
    fn dispatcher_formats_share_owner_history_and_preserve_unknown_neighbor_and_eols() {
        runtime(|| {
            let raw = "apple\r\n\r\n<keep data-a='1'>raw</keep>\n";
            for (id, value) in [
                ("bold", "**apple**"),
                ("italic", "*apple*"),
                ("strikethrough", "~~apple~~"),
                ("inlineCode", "`apple`"),
            ] {
                let (source, gate) = setup(raw);
                select(&source, &gate, "apple", true);
                let id = format!("texzi.markdown.{id}");
                let before = audit(&source);
                availability(&gate, &source, &id, true).unwrap();
                assert_eq!(audit(&source), before);
                assert_eq!(
                    run(&gate, &source, &id, true).unwrap(),
                    Outcome::Applied { body_changed: true }
                );
                assert_eq!(
                    bytes(&source),
                    raw.replacen("apple", value, 1).as_bytes(),
                    "{id}"
                );
                gate.to_source();
                gate.shell_history(true).unwrap();
                assert_eq!(bytes(&source), raw.as_bytes());
                gate.shell_history(false).unwrap();
                assert_eq!(bytes(&source), raw.replacen("apple", value, 1).as_bytes());
            }
        });
    }
    #[test]
    fn dispatcher_cursor_format_changes_only_future_style_until_text_is_typed() {
        runtime(|| {
            let raw = "apple\r\n\r\n<keep>raw</keep>\n";
            let (source, gate) = setup(raw);
            select(&source, &gate, "apple", false);
            let before = audit(&source);
            assert_eq!(
                run(&gate, &source, "texzi.markdown.bold", true).unwrap(),
                Outcome::Applied {
                    body_changed: false
                }
            );
            assert_eq!(audit(&source), before);
            let state = gate.session().state();
            assert!(
                state
                    .stored_marks()
                    .unwrap()
                    .iter()
                    .any(|mark| mark.ty.name() == "bold")
            );
            let mut tr = state.tr();
            tr.insert_text("X", None, None);
            gate.session().dispatch(tr).unwrap();
            assert!(gate.status.get().is_empty(), "{}", gate.status.get());
            assert_eq!(
                bytes(&source),
                raw.replacen("apple", "**X**apple", 1).as_bytes()
            );
            gate.shell_history(true).unwrap();
            assert_eq!(bytes(&source), raw.as_bytes());
        });
    }
    #[test]
    fn dispatcher_task_and_code_exit_reuse_existing_source_transactions() {
        runtime(|| {
            let raw = "+ [ ] apple\r\n+ [X] pear\n\n<keep>raw</keep>\r\n";
            let (source, gate) = setup(raw);
            select(&source, &gate, "apple", false);
            assert_eq!(
                run(&gate, &source, "texzi.markdown.toggleTask", true).unwrap(),
                Outcome::Applied { body_changed: true }
            );
            assert_eq!(bytes(&source), raw.replacen("[ ]", "[x]", 1).as_bytes());
            gate.shell_history(true).unwrap();
            assert_eq!(bytes(&source), raw.as_bytes());
            let (source, gate) =
                setup("~~~rust extra\r\nbody\r\n~~~\r\n\r\n<keep>raw</keep>\n\nafter\r\n");
            select(&source, &gate, "body", false);
            let before = audit(&source);
            assert_eq!(
                run(&gate, &source, "texzi.markdown.exitCodeBlock", true).unwrap(),
                Outcome::Applied {
                    body_changed: false
                }
            );
            assert_eq!(audit(&source), before);
            let state = gate.session().state();
            let p = Projection::parse(state.schema(), source.snapshot().unwrap().rope.to_string())
                .unwrap();
            assert_eq!(
                p.source_position(state.selection().head()),
                p.source.find("after")
            );
        });
    }
    #[test]
    fn dispatcher_table_entries_default_to_after_focus_new_cells_and_keep_source_undo() {
        runtime(|| {
            let raw = "| h | v |\r\n| --- | --- |\r\n| a | b |\r\n\r\n<keep>raw</keep>\n";
            let cases = [
                (
                    "insertRow",
                    "| h | v |\r\n| --- | --- |\r\n| a | b |\r\n|  |  |\r\n\r\n<keep>raw</keep>\n",
                ),
                (
                    "insertColumn",
                    "| h |  | v |\r\n| --- | --- | --- |\r\n| a |  | b |\r\n\r\n<keep>raw</keep>\n",
                ),
                (
                    "deleteRow",
                    "| h | v |\r\n| --- | --- |\r\n\r\n<keep>raw</keep>\n",
                ),
                (
                    "deleteColumn",
                    "| v |\r\n| --- |\r\n| b |\r\n\r\n<keep>raw</keep>\n",
                ),
            ];
            for (name, expected) in cases {
                let (source, gate) = setup(raw);
                select(&source, &gate, "a", false);
                let id = format!("texzi.markdown.table.{name}");
                let before = audit(&source);
                availability(&gate, &source, &id, true).unwrap();
                assert_eq!(audit(&source), before);
                let outcome = run(&gate, &source, &id, true).unwrap();
                assert_eq!(outcome, Outcome::Applied { body_changed: true });
                assert_eq!(bytes(&source), expected.as_bytes(), "{id}");
                assert_eq!(
                    source.inspect(|store, view| store
                        .history_depth(store.view(view).unwrap().doc)
                        .unwrap()),
                    (1, 0),
                    "one transaction for {id}"
                );
                let inserted = match name {
                    "insertRow" => Some((2, 0)),
                    "insertColumn" => Some((1, 1)),
                    _ => None,
                };
                let source_caret = inserted.map(|(row, column)| {
                    let state = gate.session().state();
                    let target = cell_position(state.doc(), 0, row, column).unwrap();
                    assert_eq!(
                        (state.selection().anchor(), state.selection().head()),
                        (target, target),
                        "focus new cell for {id}"
                    );
                    let projected = Projection::parse(
                        state.schema(),
                        source.snapshot().unwrap().rope.to_string(),
                    )
                    .unwrap();
                    projected.source_position(target).unwrap()
                });
                gate.to_source();
                if let Some(at) = source_caret {
                    assert_eq!(
                        source.snapshot().unwrap().selections.ranges,
                        vec![(at, at)],
                        "new-cell Source mapping for {id}"
                    );
                }
                gate.shell_history(true).unwrap();
                assert_eq!(bytes(&source), raw.as_bytes());
                gate.shell_history(false).unwrap();
                assert_eq!(bytes(&source), expected.as_bytes());
            }
        });
    }
    #[test]
    fn dispatcher_forms_and_invalid_contexts_never_publish_partial_changes() {
        runtime(|| {
            let (source, gate) = setup("plain\r\n\r\n<keep>raw</keep>\n");
            let before = audit(&source);
            for writable in [true, false] {
                let all = availability_all(&gate, &source, writable, COMMAND_IDS);
                assert_eq!(all.len(), COMMAND_IDS.len());
                for id in COMMAND_IDS {
                    assert_eq!(
                        all.get(*id).unwrap(),
                        &availability(&gate, &source, id, writable),
                        "batch/single availability for {id}"
                    );
                }
                assert_eq!(
                    audit(&source),
                    before,
                    "batch availability must never write to Store"
                );
            }
            for id in [
                "texzi.markdown.toggleTask",
                "texzi.markdown.exitCodeBlock",
                "texzi.markdown.table.deleteRow",
                "texzi.markdown.setCodeLanguage",
            ] {
                assert!(availability(&gate, &source, id, true).is_err(), "{id}");
                assert!(run(&gate, &source, id, true).is_err(), "{id}");
                assert_eq!(audit(&source), before);
            }
            assert!(availability(&gate, &source, "texzi.markdown.insertLink", true).is_ok());
            assert_eq!(
                run(&gate, &source, "texzi.markdown.insertLink", true).unwrap(),
                Outcome::Form(FormRequest::Link)
            );
            assert_eq!(audit(&source), before, "new-link form routing is read-only");
            assert!(run(&gate, &source, "texzi.markdown.bold", false).is_err());
            source.set_composing(true);
            assert!(availability(&gate, &source, "texzi.markdown.bold", true).is_err());
            source.set_composing(false);
            gate.to_source();
            assert!(run(&gate, &source, "texzi.markdown.bold", true).is_err());
            assert_eq!(audit(&source), before);
            for id in [
                "texzi.markdown.copyMarkdown",
                "texzi.markdown.pasteMarkdown",
                "texzi.markdown.toggleSyncScroll",
            ] {
                assert!(!handles(id));
                assert!(run(&gate, &source, id, true).is_err());
            }
            let (source, gate) = setup(
                "[label](target)\r\n\r\n~~~rust\r\nbody\r\n~~~\r\n\r\n| h | v |\r\n| --- | --- |\r\n| a | b |\r\n",
            );
            select(&source, &gate, "label", false);
            let before = audit(&source);
            assert_eq!(
                run(&gate, &source, "texzi.markdown.insertLink", true).unwrap(),
                Outcome::Form(FormRequest::Link)
            );
            assert_eq!(audit(&source), before);
            select(&source, &gate, "body", false);
            let before = audit(&source);
            assert_eq!(
                run(&gate, &source, "texzi.markdown.setCodeLanguage", true).unwrap(),
                Outcome::Form(FormRequest::CodeLanguage)
            );
            assert_eq!(audit(&source), before);
            select(&source, &gate, "a | b", false);
            let before = audit(&source);
            assert_eq!(
                run(&gate, &source, "texzi.markdown.table.alignColumn", true).unwrap(),
                Outcome::Form(FormRequest::TableAlignment)
            );
            for args in [
                Value::Null,
                json!({}),
                json!({"position":"unknown"}),
                json!({"position":"after","extra":1}),
            ] {
                assert!(
                    run_with(
                        &gate,
                        &source,
                        "texzi.markdown.table.insertRow",
                        &args,
                        true
                    )
                    .is_err()
                );
                assert_eq!(audit(&source), before);
            }
            assert!(
                run_with(
                    &gate,
                    &source,
                    "texzi.markdown.table.alignColumn",
                    &json!({"alignment":"unknown"}),
                    true
                )
                .is_err()
            );
            assert_eq!(audit(&source), before);
        });
    }
}
