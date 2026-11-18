//! Enter navigates a simple table without changing the authoritative body.
//! The caller checks live IME composition before invoking this command.
use crate::table_controls::cell_position;
use nsh_base::rich_editor::{
    commands::{BoxCommand, command},
    state::{EditorState, TextSelection, TransactionOrigin},
    tables::{cell_around, selected_rect},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EnterTarget {
    OutsideTable,
    NextRow { position: usize },
    LastRow,
}

/// Only ordinary text selections wholly within one cell participate. Other
/// selections return control to the editor's existing key bindings.
pub(crate) fn target(state: &EditorState) -> EnterTarget {
    let selection = state.selection();
    if selection.as_text().is_none() {
        return EnterTarget::OutsideTable;
    }
    let (Some(anchor), Some(head)) = (
        cell_around(selection.resolved_anchor()),
        cell_around(selection.resolved_head()),
    ) else {
        return EnterTarget::OutsideTable;
    };
    if anchor.pos != head.pos {
        return EnterTarget::OutsideTable;
    }
    let Some(rect) = selected_rect(state) else {
        return EnterTarget::OutsideTable;
    };
    if !rect.map.problems.is_empty()
        || rect.rect.right != rect.rect.left + 1
        || rect.rect.bottom != rect.rect.top + 1
    {
        return EnterTarget::OutsideTable;
    }
    let mut base = 0;
    let block = (0..state.doc().child_count()).find(|index| {
        let start = base;
        base += state.doc().child(*index).node_size();
        start + 1 == rect.table_start
    });
    let Some(block) = block else {
        return EnterTarget::OutsideTable;
    };
    let row = rect.rect.top + 1;
    if row >= rect.map.height {
        return EnterTarget::LastRow;
    }
    match cell_position(state.doc(), block, row, rect.rect.left) {
        Some(position) => EnterTarget::NextRow { position },
        None => EnterTarget::OutsideTable,
    }
}

/// `on_boundary` supplies the owner's explicit last-row feedback. A command
/// availability query (`dispatch=None`) never invokes it or changes state.
pub(crate) fn enter(on_boundary: impl Fn() + Send + Sync + 'static) -> BoxCommand {
    command(move |state, dispatch| match target(state) {
        EnterTarget::OutsideTable => false,
        EnterTarget::LastRow => {
            if dispatch.is_some() {
                on_boundary();
            }
            true
        }
        EnterTarget::NextRow { position } => {
            if let Some(dispatch) = dispatch {
                let mut tr = state.tr();
                tr.set_selection(TextSelection::create(state.doc(), position, None));
                tr.set_origin(TransactionOrigin::User);
                tr.set_add_to_history(false);
                tr.scroll_into_view();
                dispatch(tr);
            }
            true
        }
    })
}
