//! Source-bound code actions using the live rich session and shared Store.
use crate::{editor_owner::NativeView, rich_owner::ModeGate, rich_projection::Projection};
use nsh_base::{
    editor::ExternalDocument,
    rich_editor::{
        model::Node,
        state::{KeyEvent, Transaction, TransactionOrigin},
    },
};
use nsh_core::{
    element::{input, view as box_view},
    prelude::*,
};
use std::{cell::RefCell, rc::Rc};

struct Draft {
    revision: u64,
    doc: Node,
    position: usize,
}
pub struct CodeControls {
    gate: Rc<ModeGate>,
    source: Rc<NativeView>,
    draft: RefCell<Option<Draft>>,
    pub open: Signal<bool>,
    pub language: Signal<String>,
    pub info: Signal<String>,
}
impl CodeControls {
    pub fn new(gate: Rc<ModeGate>, source: Rc<NativeView>) -> Rc<Self> {
        Rc::new(Self {
            gate,
            source,
            draft: RefCell::new(None),
            open: signal(false),
            language: signal(String::new()),
            info: signal(String::new()),
        })
    }
    fn ready(&self) -> Result<(u64, Projection), String> {
        if self.gate.busy() {
            return Err("请先完成或取消中文组合，再操作代码块".into());
        }
        if !self.gate.rich_mode.get() {
            return Err("请先切换到正文模式".into());
        }
        let snapshot = self.source.snapshot()?;
        let state = self.gate.session().state();
        let projection = Projection::parse(state.schema(), snapshot.rope.to_string())?;
        projection.current_code_target(&state)?;
        Ok((snapshot.revision, projection))
    }
    fn publish(&self, tr: Transaction) -> Result<(), String> {
        self.gate.status.set(String::new());
        self.gate
            .session()
            .dispatch(tr)
            .map_err(|e| e.to_string())?;
        let status = self.gate.status.get();
        if status.is_empty() {
            Ok(())
        } else {
            Err(status)
        }
    }
    pub fn begin(&self) -> Result<(), String> {
        let (revision, projection) = self.ready()?;
        let state = self.gate.session().state();
        let target = projection.current_code_target(&state)?;
        if !target.fenced {
            return Err("缩进代码块没有语言标记，请保留原形式并在源码编辑".into());
        }
        self.language.set(target.language.unwrap_or_default());
        self.info.set(target.info.unwrap_or_default());
        *self.draft.borrow_mut() = Some(Draft {
            revision,
            doc: state.doc().clone(),
            position: target.position,
        });
        self.open.set(true);
        self.gate.status.set(String::new());
        Ok(())
    }
    pub fn cancel(&self) {
        self.draft.borrow_mut().take();
        self.open.set(false);
    }
    pub fn apply_language(&self) -> Result<(), String> {
        let (revision, projection) = self.ready()?;
        let state = self.gate.session().state();
        let draft_guard = self.draft.borrow();
        let draft = draft_guard.as_ref().ok_or("请先打开代码语言面板")?;
        if revision != draft.revision || state.doc() != &draft.doc {
            return Err("代码语言面板已过期，请重新打开".into());
        }
        let language = self.language.get();
        let tr = projection.code_language_transaction(
            &state,
            draft.position,
            if language.is_empty() {
                None
            } else {
                Some(&language)
            },
        )?;
        self.publish(tr)?;
        drop(draft_guard);
        self.cancel();
        Ok(())
    }
    pub fn exit(&self) -> Result<(), String> {
        let (_, projection) = self.ready()?;
        let state = self.gate.session().state();
        let tr = projection.code_exit_transaction(&state)?;
        self.publish(tr)
    }
    pub fn indent(&self, outdent: bool) -> Result<(), String> {
        self.ready()?;
        let state = self.gate.session().state();
        let mut key = KeyEvent::new("Tab");
        key.shift = outdent;
        let mut made = None;
        if !state
            .plugins()
            .iter()
            .any(|plugin| plugin.handle_key_down(&state, &key, &mut |tr| made = Some(tr)))
        {
            return Err("代码缩进快捷键尚未启用".into());
        }
        let mut tr = made.ok_or("代码缩进命令没有生成事务")?;
        tr.set_origin(TransactionOrigin::User);
        self.publish(tr)
    }
    fn report(&self, result: Result<(), String>) {
        if let Err(error) = result {
            self.gate.status.set(error);
        }
    }
}

pub fn view(controls: Rc<CodeControls>, focus: Rc<dyn Fn()>) -> impl IntoElement {
    let language = controls.clone();
    let exit = controls.clone();
    let indent = controls.clone();
    let outdent = controls.clone();
    let modal = controls.clone();
    let exit_focus = focus.clone();
    let indent_focus = focus.clone();
    let outdent_focus = focus.clone();
    box_view()
        .gap(px(6.0))
        .child(
            box_view()
                .flex_row()
                .flex_wrap()
                .min_w(px(0.0))
                .gap(px(6.0))
                .child(
                    nsh_base::button()
                        .child(text("代码语言"))
                        .on_press(move |_: &()| language.report(language.begin())),
                )
                .child(
                    nsh_base::button()
                        .child(text("退出代码块"))
                        .on_press(move |_: &()| {
                            exit.report(exit.exit());
                            exit_focus();
                        }),
                )
                .child(
                    nsh_base::button()
                        .child(text("缩进 (Tab)"))
                        .on_press(move |_: &()| {
                            indent.report(indent.indent(false));
                            indent_focus();
                        }),
                )
                .child(
                    nsh_base::button()
                        .child(text("反缩进"))
                        .on_press(move |_: &()| {
                            outdent.report(outdent.indent(true));
                            outdent_focus();
                        }),
                ),
        )
        .child(dynamic(move || {
            if !modal.open.get() {
                return None;
            }
            let dismiss = modal.clone();
            let dismiss_focus = focus.clone();
            let cancel = modal.clone();
            let cancel_focus = focus.clone();
            let apply = modal.clone();
            let apply_focus = focus.clone();
            let language = modal.language;
            let info = modal.info;
            let status = modal.gate.status;
            Some(
                nsh_base::dialog()
                    .label("代码语言")
                    .on_cancel(move |_: &()| {
                        dismiss.cancel();
                        dismiss_focus();
                    })
                    .child(
                        box_view()
                            .w(px(360.0))
                            .p(px(16.0))
                            .gap(px(8.0))
                            .bg(crate::app_shell::background)
                            .text_color(crate::app_shell::foreground)
                            .child(text("代码语言"))
                            .child(
                                input()
                                    .value(move || language.get())
                                    .placeholder("例如 rust、python；可留空")
                                    .on_change(move |value: &str| language.set(value.into())),
                            )
                            .child(text(move || {
                                let value = info.get();
                                if value.is_empty() {
                                    String::new()
                                } else {
                                    format!("附加围栏信息保持原样：{value}")
                                }
                            }))
                            .child(text(move || status.get()))
                            .child(
                                box_view()
                                    .flex_row()
                                    .gap(px(8.0))
                                    .child(nsh_base::button().child(text("取消")).on_press(
                                        move |_: &()| {
                                            cancel.cancel();
                                            cancel_focus();
                                        },
                                    ))
                                    .child(nsh_base::button().child(text("应用")).on_press(
                                        move |_: &()| {
                                            let result = apply.apply_language();
                                            let done = result.is_ok();
                                            apply.report(result);
                                            if done {
                                                apply_focus();
                                            }
                                        },
                                    )),
                            ),
                    )
                    .into_element(),
            )
        }))
}
