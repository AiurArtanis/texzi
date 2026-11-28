//! Revision-bound table actions. The Store remains the only body and history.
use crate::{editor_owner::NativeView, rich_owner::ModeGate, rich_projection::Projection};
use nsh_base::{
    editor::ExternalDocument,
    rich_editor::{
        commands::BoxCommand,
        model::{Fragment, Node},
        state::{EditorState, TextSelection, Transaction, TransactionOrigin},
        tables::{CellSelection, Rect, TableRect, commands as tables, selected_rect},
        view::TableAnchor,
    },
};
use nsh_core::{
    element::{scroll, view as box_view},
    prelude::*,
};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};

pub(crate) const META: &str = "texzi.table";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TableAction {
    RowBefore,
    RowAfter,
    DeleteRow,
    ColumnBefore,
    ColumnAfter,
    DeleteColumn,
    AlignDefault,
    AlignLeft,
    AlignCenter,
    AlignRight,
    Header,
    CancelHeader,
    DeleteTable,
}
impl TableAction {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::RowBefore => "rowBefore",
            Self::RowAfter => "rowAfter",
            Self::DeleteRow => "deleteRow",
            Self::ColumnBefore => "columnBefore",
            Self::ColumnAfter => "columnAfter",
            Self::DeleteColumn => "deleteColumn",
            Self::AlignDefault => "alignDefault",
            Self::AlignLeft => "alignLeft",
            Self::AlignCenter => "alignCenter",
            Self::AlignRight => "alignRight",
            Self::Header => "header",
            Self::CancelHeader => "cancelHeader",
            Self::DeleteTable => "deleteTable",
        }
    }
    pub(crate) fn parse(value: &str) -> Option<Self> {
        if value == Self::CancelHeader.name() {
            return Some(Self::CancelHeader);
        }
        ACTIONS
            .into_iter()
            .map(|(_, _, action)| action)
            .find(|action| action.name() == value)
    }
}

pub(crate) fn cell_position(doc: &Node, block: usize, row: usize, col: usize) -> Option<usize> {
    if block >= doc.child_count() {
        return None;
    }
    let table = doc.child(block);
    if table.ty().name() != "table"
        || row >= table.child_count()
        || col >= table.child(row).child_count()
    {
        return None;
    }
    Some(
        (0..block).map(|i| doc.child(i).node_size()).sum::<usize>()
            + 4
            + (0..row).map(|i| table.child(i).node_size()).sum::<usize>()
            + (0..col)
                .map(|i| table.child(row).child(i).node_size())
                .sum::<usize>(),
    )
}

/// Run the installed NSH command, maintaining the table's first-row header
/// state and column-wide paragraph alignment. This same deterministic operation is
/// replayed by the source bridge; a metadata label alone grants no write.
pub(crate) fn action_transaction(
    state: &EditorState,
    block: usize,
    row: usize,
    col: usize,
    action: TableAction,
) -> Result<Transaction, String> {
    let position = cell_position(state.doc(), block, row, col).ok_or("表格位置已失效")?;
    let old = state.doc().child(block);
    let rows = old.child_count();
    let columns = old.child(0).child_count();
    if columns == 0
        || (0..rows).any(|r| {
            let cells = old.child(r);
            cells.ty().name() != "tableRow"
                || cells.child_count() != columns
                || (0..columns).any(|c| {
                    let cell = cells.child(c);
                    !matches!(cell.ty().name(), "tableHeader" | "tableCell")
                        || cell.child_count() != 1
                        || cell.child(0).ty().name() != "paragraph"
                        || ["colspan", "rowspan"]
                            .iter()
                            .any(|key| cell.attrs().get(*key).and_then(Value::as_u64) != Some(1))
                })
        })
    {
        return Err("该表格包含跨行、跨列或复杂内容，请编辑源码".into());
    }
    if action == TableAction::CancelHeader {
        return cancel_header_transaction(state, block, row, col, old);
    }
    let mut selection = state.tr();
    selection.set_selection(TextSelection::create(state.doc(), position, None));
    let selected = state.apply(selection).map_err(|error| error.to_string())?;
    let table_pos = (0..block)
        .map(|i| state.doc().child(i).node_size())
        .sum::<usize>();
    let deleting = action == TableAction::DeleteTable
        || action == TableAction::DeleteRow && rows == 1
        || action == TableAction::DeleteColumn && columns == 1;
    let command: Option<BoxCommand> = if deleting {
        Some(tables::delete_table())
    } else {
        match action {
            TableAction::RowBefore => Some(tables::add_row_before()),
            TableAction::RowAfter => Some(tables::add_row_after()),
            TableAction::DeleteRow => Some(tables::delete_row()),
            TableAction::ColumnBefore => Some(tables::add_column_before()),
            TableAction::ColumnAfter => Some(tables::add_column_after()),
            TableAction::DeleteColumn => Some(tables::delete_column()),
            TableAction::Header if row > 0 => {
                Some(tables::move_table_row(Some(table_pos), row, 0, false))
            }
            _ => None,
        }
    };
    let mut tr = match command {
        Some(command) => {
            let mut result = None;
            if !command(&selected, Some(&mut |made| result = Some(made))) {
                return Err("此表格操作当前不可用".into());
            }
            let made = result.ok_or("表格命令没有生成事务")?;
            let mut rebased = state.tr();
            for step in &made.steps {
                rebased
                    .step(step.boxed_clone())
                    .map_err(|e| e.to_string())?;
            }
            rebased
        }
        None => state.tr(),
    };
    if deleting {
        // NSH fills an otherwise empty document itself. In a multi-block
        // document explicitly leave one empty paragraph at the old table.
        if state.doc().child_count() > 1 {
            let empty = state
                .schema()
                .node("paragraph", None, Fragment::empty(), vec![])
                .map_err(|e| e.to_string())?;
            tr.insert(table_pos, Fragment::from(vec![empty]))
                .map_err(|e| e.to_string())?;
        }
        tr.set_selection(TextSelection::create(tr.doc(), table_pos + 1, None));
    } else {
        let mut alignments: Vec<Value> = (0..columns)
            .map(|c| {
                old.child(0)
                    .child(c)
                    .child(0)
                    .attrs()
                    .get("textAlign")
                    .cloned()
                    .unwrap_or(Value::Null)
            })
            .collect();
        match action {
            TableAction::ColumnBefore => alignments.insert(col, Value::Null),
            TableAction::ColumnAfter => alignments.insert(col + 1, Value::Null),
            TableAction::DeleteColumn => {
                alignments.remove(col);
            }
            TableAction::AlignDefault => alignments[col] = Value::Null,
            TableAction::AlignLeft => alignments[col] = json!("left"),
            TableAction::AlignCenter => alignments[col] = json!("center"),
            TableAction::AlignRight => alignments[col] = json!("right"),
            _ => {}
        }
        let header = action == TableAction::Header || row_has_header(old.child(0));
        let table = tr.doc().child(block).clone();
        let mut row_pos = table_pos + 1;
        for r in 0..table.child_count() {
            let rich_row = table.child(r);
            let mut cell_pos = row_pos + 1;
            for (c, alignment) in alignments.iter().enumerate() {
                let cell = rich_row.child(c);
                let name = if header && r == 0 {
                    "tableHeader"
                } else {
                    "tableCell"
                };
                if cell.ty().name() != name {
                    let ty = state.schema().node_type(name).ok_or("缺少表格单元格类型")?;
                    tr.set_node_markup(cell_pos, Some(ty), Some(cell.attrs().clone()), None)
                        .map_err(|e| e.to_string())?;
                }
                let paragraph = cell.child(0);
                if paragraph.attrs().get("textAlign") != Some(alignment) {
                    let mut attrs = paragraph.attrs().clone();
                    attrs.insert("textAlign".into(), alignment.clone());
                    tr.set_node_markup(cell_pos + 1, None, Some(attrs), None)
                        .map_err(|e| e.to_string())?;
                }
                cell_pos += cell.node_size();
            }
            row_pos += rich_row.node_size();
        }
        let current = tr.doc().child(block);
        let target_row = if action == TableAction::Header {
            0
        } else {
            row.min(current.child_count() - 1)
        };
        let target_col = col.min(current.child(0).child_count() - 1);
        let caret =
            cell_position(tr.doc(), block, target_row, target_col).ok_or("表格结果位置无效")?;
        tr.set_selection(TextSelection::create(tr.doc(), caret, None));
    }
    stamp_action(&mut tr, block, row, col, action);
    Ok(tr)
}

fn cancel_header_transaction(
    state: &EditorState,
    block: usize,
    row: usize,
    col: usize,
    table: &Node,
) -> Result<Transaction, String> {
    let rich_row = table.child(row);
    if !row_has_header(rich_row) {
        return Err("当前行没有表头单元格".into());
    }
    let ordinary = state
        .schema()
        .node_type("tableCell")
        .ok_or("缺少普通单元格类型")?;
    let mut at = cell_position(state.doc(), block, row, 0).ok_or("表格位置已失效")? - 2;
    let mut tr = state.tr();
    for column in 0..rich_row.child_count() {
        let cell = rich_row.child(column);
        if cell.ty().name() == "tableHeader" {
            tr.set_node_markup(at, Some(ordinary.clone()), Some(cell.attrs().clone()), None)
                .map_err(|error| error.to_string())?;
        }
        at += cell.node_size();
    }
    // Header and ordinary cells have the same shape. Mapping the existing
    // selection through the transaction keeps an in-cell caret or range.
    stamp_action(&mut tr, block, row, col, TableAction::CancelHeader);
    Ok(tr)
}

pub(crate) fn active_header_cell(state: &EditorState, table: &TableRect) -> Option<Rect> {
    if let Some(cells) = state.selection().as_custom::<CellSelection>() {
        let offset = cells.head_cell().pos.checked_sub(table.table_start)?;
        table.map.try_find_cell(offset)
    } else {
        Some(table.rect)
    }
}

fn row_has_header(row: &Node) -> bool {
    (0..row.child_count()).any(|column| row.child(column).ty().name() == "tableHeader")
}

fn stamp_action(tr: &mut Transaction, block: usize, row: usize, col: usize, action: TableAction) {
    tr.set_meta(
        META,
        json!({"block":block,"row":row,"column":col,"action":action.name()}),
    );
    tr.set_origin(TransactionOrigin::User);
}

struct Draft {
    revision: u64,
    doc: Node,
    block: usize,
    row: usize,
    column: usize,
    header_cell: (usize, usize),
    header_action: TableAction,
    header_enabled: bool,
}
pub(crate) struct TableControls {
    gate: Rc<ModeGate>,
    source: Rc<NativeView>,
    draft: RefCell<Option<Draft>>,
    anchor: Signal<Option<TableAnchor>>,
}
impl TableControls {
    pub(crate) fn new(gate: Rc<ModeGate>, source: Rc<NativeView>) -> Rc<Self> {
        Rc::new(Self {
            gate,
            source,
            draft: RefCell::new(None),
            anchor: signal(None),
        })
    }
    pub(crate) fn begin(&self) -> Result<(), String> {
        if self.gate.busy() {
            return Err("请先完成或取消中文组合，再编辑表格".into());
        }
        if !self.gate.rich_mode.get() {
            return Err("请先切换到正文模式".into());
        }
        let state = self.gate.session().state();
        let snapshot = self.source.snapshot()?;
        let projection = Projection::parse(state.schema(), snapshot.rope.to_string())?;
        if &projection.doc != state.doc() {
            return Err("表格版本已变化，请重新打开面板".into());
        }
        let rect = selected_rect(&state).ok_or("请先将光标放在表格单元格中")?;
        let mut base = 0;
        let block = (0..state.doc().child_count())
            .find(|i| {
                let at = base;
                base += state.doc().child(*i).node_size();
                at + 1 == rect.table_start
            })
            .ok_or("嵌套表格请编辑源码")?;
        let active = active_header_cell(&state, &rect).ok_or("表格选区位置已失效")?;
        let (row, column) = (rect.rect.top, rect.rect.left);
        let header_cell = (active.top, active.left);
        let cell = cell_position(state.doc(), block, active.top, active.left)
            .and_then(|position| position.checked_sub(2))
            .ok_or("无法定位当前表格单元格")?;
        let header_action = if row_has_header(state.doc().child(block).child(active.top)) {
            TableAction::CancelHeader
        } else {
            TableAction::Header
        };
        let header_enabled = (header_action == TableAction::Header || active.top == 0)
            && projection.supports_table_header_edit(block);
        *self.draft.borrow_mut() = Some(Draft {
            revision: snapshot.revision,
            doc: state.doc().clone(),
            block,
            row,
            column,
            header_cell,
            header_action,
            header_enabled,
        });
        self.gate.status.set(String::new());
        self.anchor.set(Some(TableAnchor {
            revision: state.revision(),
            table: rect.table_start - 1,
            cell,
        }));
        Ok(())
    }
    pub(crate) fn cancel(&self) {
        if self.draft.borrow_mut().take().is_some() {
            self.anchor.set(None);
        }
    }
    pub(crate) fn is_open(&self) -> bool {
        self.anchor().is_some()
    }
    pub(crate) fn anchor(&self) -> Option<TableAnchor> {
        self.anchor.get()
    }
    pub(crate) fn status(&self) -> Signal<String> {
        self.gate.status
    }
    pub(crate) fn column_alignment(&self) -> Option<TableAction> {
        let draft = self.draft.borrow();
        let draft = draft.as_ref()?;
        let first_row = draft.doc.maybe_child(draft.block)?.maybe_child(0)?;
        let cell = first_row.maybe_child(draft.column)?;
        let paragraph = cell.maybe_child(0)?;
        match paragraph.attrs().get("textAlign").and_then(Value::as_str) {
            Some("left") => Some(TableAction::AlignLeft),
            Some("center") => Some(TableAction::AlignCenter),
            Some("right") => Some(TableAction::AlignRight),
            _ => None,
        }
    }

    // A single row-dependent header button shares one slot in both surfaces.
    pub(crate) fn buttons(
        &self,
    ) -> [(&'static str, &'static str, TableAction, bool); ACTIONS.len()] {
        let draft = self.draft.borrow();
        ACTIONS.map(|(label, short, action)| {
            if action != TableAction::Header {
                return (label, short, action, true);
            }
            match draft.as_ref() {
                Some(draft) if draft.header_action == TableAction::CancelHeader => (
                    "取消当前行表头",
                    "取消表头",
                    TableAction::CancelHeader,
                    draft.header_enabled,
                ),
                Some(draft) => (label, short, action, draft.header_enabled),
                None => (label, short, action, false),
            }
        })
    }

    pub(crate) fn apply(&self, action: TableAction) -> Result<(), String> {
        if self.gate.busy() {
            return Err("请先完成或取消中文组合，再应用表格操作".into());
        }
        if !self.gate.rich_mode.get() {
            return Err("模式已变化，请重新打开表格面板".into());
        }
        let snapshot = self.source.snapshot()?;
        let session = self.gate.session();
        let state = session.state();
        let draft_guard = self.draft.borrow();
        let draft = draft_guard.as_ref().ok_or("请先打开表格面板")?;
        if snapshot.revision != draft.revision
            || self
                .anchor
                .get_untracked()
                .is_none_or(|target| target.revision != state.revision())
            || state.doc() != &draft.doc
        {
            return Err("表格面板版本已过期，请取消后重新打开".into());
        }
        let (row, column) = if matches!(action, TableAction::Header | TableAction::CancelHeader) {
            if action != draft.header_action || !draft.header_enabled {
                return Err("当前行不能保真切换表头，请编辑源码".into());
            }
            draft.header_cell
        } else {
            (draft.row, draft.column)
        };
        let tr = action_transaction(&state, draft.block, row, column, action)?;
        drop(draft_guard);
        self.gate.status.set(String::new());
        session.dispatch(tr).map_err(|e| e.to_string())?;
        let status = self.gate.status.get();
        if !status.is_empty() {
            return Err(status);
        }
        self.cancel();
        Ok(())
    }
    pub(crate) fn report(&self, result: Result<(), String>) {
        if let Err(error) = result {
            self.gate.status.set(error);
        }
    }
}

pub(crate) const ACTIONS: [(&str, &str, TableAction); 12] = [
    ("上方插入行", "上插行", TableAction::RowBefore),
    ("下方插入行", "下插行", TableAction::RowAfter),
    ("删除当前行", "删行", TableAction::DeleteRow),
    ("左侧插入列", "左插列", TableAction::ColumnBefore),
    ("右侧插入列", "右插列", TableAction::ColumnAfter),
    ("删除当前列", "删列", TableAction::DeleteColumn),
    ("当前列默认对齐", "默认对齐", TableAction::AlignDefault),
    ("当前列左对齐", "左对齐", TableAction::AlignLeft),
    ("当前列居中", "居中", TableAction::AlignCenter),
    ("当前列右对齐", "右对齐", TableAction::AlignRight),
    ("将当前行设为表头", "设为表头", TableAction::Header),
    ("删除表格", "删除表格", TableAction::DeleteTable),
];

pub fn dialog(
    controls: Rc<TableControls>,
    fallback: impl Fn() -> bool + 'static,
    return_focus: Rc<dyn Fn()>,
) -> impl IntoElement {
    dynamic(move || {
        if !fallback() || !controls.is_open() {
            return None;
        }
        let dismiss = controls.clone();
        let dismiss_focus = return_focus.clone();
        let cancel = controls.clone();
        let cancel_focus = return_focus.clone();
        let status = controls.gate.status;
        let mut body = box_view()
            .w_full()
            .p(px(16.0))
            .gap(px(6.0))
            .bg(crate::app_shell::background)
            .text_color(crate::app_shell::foreground)
            .child(text("表格操作"))
            .child(text(
                "对当前单元格所在行或列操作；删除最后一行或列会留下可输入的空段。",
            ));
        for (label, _, action, enabled) in controls.buttons() {
            let apply = controls.clone();
            let focus = return_focus.clone();
            body = body.child(
                nsh_base::button()
                    .disabled(!enabled)
                    .child(text(label))
                    .on_press(move |_: &()| {
                        let result = apply.apply(action);
                        let done = result.is_ok();
                        apply.report(result);
                        if done {
                            focus();
                        }
                    }),
            );
        }
        Some(
            nsh_base::dialog()
                .label("表格操作")
                .on_cancel(move |_: &()| {
                    dismiss.cancel();
                    dismiss_focus();
                })
                .child(
                    scroll()
                        .w(px(420.0))
                        .max_w(relative(1.0))
                        .max_h(relative(0.85))
                        .child(
                            body.child(text(move || status.get())).child(
                                nsh_base::button()
                                    .child(text("取消"))
                                    .on_press(move |_: &()| {
                                        cancel.cancel();
                                        cancel_focus();
                                    }),
                            ),
                        ),
                )
                .into_element(),
        )
    })
}
