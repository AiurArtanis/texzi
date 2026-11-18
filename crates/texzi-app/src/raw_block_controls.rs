//! Local, revision-bound source entries for blocks retained as raw syntax.
use crate::rich_owner::{ModeGate, RawBlockAction};
use nsh_core::prelude::*;
use std::rc::Rc;

pub fn raw_block_controls(
    gate: Rc<ModeGate>,
    owner: Owner,
    on_source: impl Fn() + Clone + 'static,
) -> impl IntoElement {
    let entries = gate.clone();
    view().child(each(
        move || entries.raw_blocks(),
        |action: &RawBlockAction| (action.revision, action.source.start, action.source.end),
        move |action, _| {
            let action = action.get_untracked();
            let gate = gate.clone();
            let on_source = on_source.clone();
            let select_gate = gate.clone();
            let select_action = action.clone();
            let select_focus = on_source.clone();
            view()
                .flex_row()
                .gap(px(8.0))
                .p(px(6.0))
                .child(text(format!(
                    "原样块 · 第 {} 行 · {}",
                    action.line, action.preview
                )))
                .child(
                    nsh_base::button()
                        .child(text("选择整块"))
                        .on_press(move |_: &()| {
                            owner.run(|| match select_gate.select_raw_block(&select_action) {
                                Ok(()) => select_focus(),
                                Err(error) => select_gate.status.set(error),
                            })
                        }),
                )
                .child(
                    nsh_base::button()
                        .child(text("编辑源码"))
                        .on_press(move |_: &()| {
                            owner.run(|| match gate.edit_raw_block(&action) {
                                Ok(()) => on_source(),
                                Err(error) => gate.status.set(error),
                            })
                        }),
                )
        },
    ))
}
