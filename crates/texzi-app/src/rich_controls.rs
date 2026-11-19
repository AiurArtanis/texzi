//! Link/image property dialogs. Drafts are local UI state, never a second body
//! or history, and every submission goes through the live EditorSession.
use crate::{
    editor_owner::NativeView,
    rich_owner::ModeGate,
    rich_projection::{Projection, ResourceKind, ResourceTarget, ResourceValues},
};
use nsh_base::editor::ExternalDocument;
use nsh_base::rich_editor::model::Node;
use nsh_core::{
    element::{input, view as box_view},
    prelude::*,
};
use std::{cell::RefCell, rc::Rc};

struct Draft {
    revision: u64,
    doc: Node,
    target: ResourceTarget,
}
pub struct ResourceControls {
    gate: Rc<ModeGate>,
    source: Rc<NativeView>,
    draft: RefCell<Option<Draft>>,
    pub open: Signal<bool>,
    pub url: Signal<String>,
    pub title: Signal<String>,
    pub alt: Signal<String>,
    pub kind: Signal<ResourceKind>,
    pub reference: Signal<bool>,
    pub creating: Signal<bool>,
    pub text: Signal<String>,
    pub empty_selection: Signal<bool>,
}
impl ResourceControls {
    pub fn new(gate: Rc<ModeGate>, source: Rc<NativeView>) -> Rc<Self> {
        Rc::new(Self {
            gate,
            source,
            draft: RefCell::new(None),
            open: signal(false),
            url: signal(String::new()),
            title: signal(String::new()),
            alt: signal(String::new()),
            kind: signal(ResourceKind::Link),
            reference: signal(false),
            creating: signal(false),
            text: signal(String::new()),
            empty_selection: signal(false),
        })
    }
    pub fn begin(&self, kind: ResourceKind) -> Result<(), String> {
        if self.gate.busy() {
            return Err("请先完成或取消中文组合，再编辑属性".into());
        }
        if !self.gate.rich_mode.get() {
            return Err("请先切换到正文模式".into());
        }
        let snapshot = self.source.snapshot()?;
        let state = self.gate.session().state();
        let projection = Projection::parse(state.schema(), snapshot.rope.to_string())?;
        if &projection.doc != state.doc() {
            return Err("正文版本已变化，请稍后重新打开属性".into());
        }
        let target = if kind == ResourceKind::Link {
            projection.link_target(&state)?
        } else {
            projection
                .resource_at(state.selection().head(), kind)
                .or_else(|_| projection.resource_at(state.selection().from(), kind))?
        };
        self.url.set(target.values.url.clone());
        self.title
            .set(target.values.title.clone().unwrap_or_default());
        self.alt.set(target.values.alt.clone());
        self.kind.set(kind);
        self.reference.set(target.reference.is_some());
        self.creating.set(target.selection.is_some());
        self.empty_selection.set(
            target
                .selection
                .as_ref()
                .is_some_and(std::ops::Range::is_empty),
        );
        self.text.set(target.visible.clone());
        *self.draft.borrow_mut() = Some(Draft {
            revision: snapshot.revision,
            doc: state.doc().clone(),
            target,
        });
        self.gate.status.set(String::new());
        self.open.set(true);
        Ok(())
    }
    pub fn cancel(&self) {
        self.draft.borrow_mut().take();
        self.open.set(false);
    }
    pub fn apply(&self) -> Result<(), String> {
        if self.gate.busy() {
            return Err("请先完成或取消中文组合，再应用属性".into());
        }
        if !self.gate.rich_mode.get() {
            return Err("模式已变化，请重新打开属性".into());
        }
        let snapshot = self.source.snapshot()?;
        let session = self.gate.session();
        let state = session.state();
        let draft_guard = self.draft.borrow();
        let draft = draft_guard.as_ref().ok_or("请先打开链接或图片属性")?;
        if snapshot.revision != draft.revision || state.doc() != &draft.doc {
            return Err("属性面板版本已过期，请取消后重新打开".into());
        }
        let projection = Projection::parse(state.schema(), snapshot.rope.to_string())?;
        let title = self.title.get();
        let values = ResourceValues {
            url: self.url.get(),
            title: (!title.is_empty()).then_some(title),
            alt: self.alt.get(),
        };
        let tr = if draft.target.selection.is_some() {
            projection.insert_link_transaction(&state, &draft.target, &values, &self.text.get())?
        } else {
            projection.resource_transaction(&state, &draft.target, &values)?
        };
        self.gate.status.set(String::new());
        session.dispatch(tr).map_err(|e| e.to_string())?;
        let status = self.gate.status.get();
        if !status.is_empty() {
            return Err(status);
        }
        drop(draft_guard);
        self.cancel();
        Ok(())
    }
    fn report(&self, result: Result<(), String>) {
        if let Err(error) = result {
            self.gate.status.set(error);
        }
    }
}

/// Mount this beside the rich editor. The optional focus callback is invoked
/// after Apply/Cancel so the application can restore its existing editor ref.
pub fn view(controls: Rc<ResourceControls>, focus: Rc<dyn Fn()>) -> impl IntoElement {
    let link = controls.clone();
    let image = controls.clone();
    let modal = controls.clone();
    box_view()
        .max_w(relative(1.0))
        .min_w(px(0.0))
        .gap(px(4.0))
        .child(
            box_view()
                .flex_row()
                .flex_wrap()
                .max_w(relative(1.0))
                .min_w(px(0.0))
                .gap(px(6.0))
                .child(
                    nsh_base::button()
                        .child(text("插入或编辑链接"))
                        .on_press(move |_: &()| link.report(link.begin(ResourceKind::Link))),
                )
                .child(
                    nsh_base::button()
                        .child(text("图片属性"))
                        .on_press(move |_: &()| image.report(image.begin(ResourceKind::Image))),
                ),
        )
        .child(dynamic(move || {
            if !modal.open.get() {
                return None;
            }
            let cancel = modal.clone();
            let cancel_focus = focus.clone();
            let dismiss = modal.clone();
            let dismiss_focus = focus.clone();
            let apply = modal.clone();
            let apply_focus = focus.clone();
            let url = modal.url;
            let title = modal.title;
            let alt = modal.alt;
            let link_text = modal.text;
            let create = modal.creating.get();
            let empty_selection = modal.empty_selection.get();
            let image = modal.kind.get() == ResourceKind::Image;
            let status = modal.gate.status;
            Some(
                nsh_base::dialog()
                    .label(if image {
                        "图片属性"
                    } else {
                        if create {
                            "插入链接"
                        } else {
                            "链接属性"
                        }
                    })
                    .on_cancel(move |_: &()| {
                        dismiss.cancel();
                        dismiss_focus();
                    })
                    .child(
                        box_view()
                            .w(px(420.0))
                            .p(px(16.0))
                            .gap(px(8.0))
                            .bg(crate::app_shell::background)
                            .text_color(crate::app_shell::foreground)
                            .child(text(if image {
                                "图片属性"
                            } else {
                                if create {
                                    "插入链接"
                                } else {
                                    "链接属性"
                                }
                            }))
                            .child(when(create, move || {
                                let body = box_view().gap(px(4.0)).child(text("链接文字"));
                                if empty_selection {
                                    body.child(
                                        input().value(move || link_text.get()).on_change(
                                            move |value: &str| link_text.set(value.into()),
                                        ),
                                    )
                                    .into_element()
                                } else {
                                    body.child(text(move || link_text.get()))
                                        .child(text("所选文字及原格式保持不变。"))
                                        .into_element()
                                }
                            }))
                            .child(when(modal.reference, || {
                                text("此修改会更新相应引用定义，并用于它的所有引用。")
                            }))
                            .child(text(if image {
                                "图片地址（本地或远程）"
                            } else {
                                "链接地址"
                            }))
                            .child(
                                input()
                                    .value(move || url.get())
                                    .on_change(move |v: &str| url.set(v.into())),
                            )
                            .child(when(image, move || {
                                box_view().gap(px(4.0)).child(text("替代文本")).child(
                                    input()
                                        .value(move || alt.get())
                                        .on_change(move |v: &str| alt.set(v.into())),
                                )
                            }))
                            .child(text("标题（可留空）"))
                            .child(
                                input()
                                    .value(move || title.get())
                                    .on_change(move |v: &str| title.set(v.into())),
                            )
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
                                            let result = apply.apply();
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

#[cfg(test)]
#[path = "rich_controls/link_insert_tests.rs"]
mod link_insert_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_base::{
        editor::{ExternalEditKind, ExternalSelections},
        rich_editor::state::TextSelection,
    };

    fn run(f: impl FnOnce()) {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        f();
    }
    fn audit(source: &NativeView) -> String {
        source.inspect(|store, id| {
            let view = store.view(id).unwrap();
            let snapshot = store.snapshot(view.doc).unwrap();
            format!(
                "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                snapshot.body.to_utf8(),
                snapshot.token,
                snapshot.body.origins(),
                store.history_depth(view.doc).unwrap(),
                store.dirty(view.doc).unwrap(),
                view
            )
        })
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
    fn at(source: &Rc<NativeView>, needle: &str) -> (Rc<ModeGate>, Rc<ResourceControls>) {
        let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
        gate.to_rich();
        assert!(gate.rich_mode.get(), "{}", gate.status.get());
        let session = gate.session();
        let state = session.state();
        let raw = source.snapshot().unwrap().rope.to_string();
        let projection = Projection::parse(state.schema(), raw.clone()).unwrap();
        let pos = projection.rich_position(raw.find(needle).unwrap()).unwrap();
        let mut tr = state.tr();
        tr.set_selection(TextSelection::create(state.doc(), pos, None));
        session.dispatch(tr).unwrap();
        assert!(gate.status.get().is_empty(), "{}", gate.status.get());
        let controls = ResourceControls::new(gate.clone(), source.clone());
        (gate, controls)
    }
    fn roundtrip(gate: &ModeGate, source: &NativeView, raw: &str, expected: &str) {
        assert_eq!(bytes(source), expected.as_bytes());
        source.inspect(|store, id| {
            let doc = store.view(id).unwrap().doc;
            assert_eq!(store.history_depth(doc).unwrap(), (1, 0));
            assert!(store.dirty(doc).unwrap());
        });
        gate.shell_history(true).unwrap();
        assert_eq!(bytes(source), raw.as_bytes());
        source.inspect(|store, id| assert!(!store.dirty(store.view(id).unwrap().doc).unwrap()));
        gate.shell_history(false).unwrap();
        assert_eq!(bytes(source), expected.as_bytes());
    }
    #[test]
    fn resource_link_dialog_changes_url_title_and_preserves_label_neighbor_and_store_history() {
        run(|| {
            let raw = "\u{feff}before\r\n\r\n[**la &amp; bel**](<https://old.invalid/a> \"keep\")\n\n<custom>stay</custom>\r\n";
            let source = NativeView::fixture(raw);
            let (gate, controls) = at(&source, "la &amp;");
            controls.begin(ResourceKind::Link).unwrap();
            controls.url.set("https://new.invalid/a?x=1&copy;".into());
            controls.title.set("新 & \"标题\"".into());
            controls.apply().unwrap();
            assert!(!controls.open.get());
            let expected = raw.replace(
                "<https://old.invalid/a> \"keep\"",
                "<https://new.invalid/a?x=1&amp;copy;> \"新 &amp; \\\"标题\\\"\"",
            );
            let mut found = false;
            gate.session()
                .state()
                .doc()
                .descendants(&mut |node, _, _, _| {
                    found |= node.marks().iter().any(|m| {
                        m.ty.name() == "link"
                            && m.attrs.get("href").and_then(serde_json::Value::as_str)
                                == Some("https://new.invalid/a?x=1&copy;")
                    });
                    true
                });
            assert!(found);
            roundtrip(&gate, &source, raw, &expected);
        });
    }
    #[test]
    fn resource_reference_link_changes_only_shared_definition_and_all_visible_targets() {
        run(|| {
            let raw = "before\r\n\r\n[one][shared] and [two][shared]\n\n[shared]: <https://old.invalid> \"keep\"\r\n[other]: /untouched \"title\"\n\n<custom>stay</custom>\r\n";
            let source = NativeView::fixture(raw);
            let (gate, controls) = at(&source, "one");
            controls.begin(ResourceKind::Link).unwrap();
            assert!(controls.reference.get());
            controls.url.set("./new-target".into());
            controls.title.set("新标题".into());
            controls.apply().unwrap();
            let expected = raw.replace(
                "[shared]: <https://old.invalid> \"keep\"",
                "[shared]: <./new-target> \"新标题\"",
            );
            let mut matched = 0;
            gate.session()
                .state()
                .doc()
                .descendants(&mut |node, _, _, _| {
                    if node.text().is_some_and(|t| t == "one" || t == "two")
                        && node.marks().iter().any(|m| {
                            m.ty.name() == "link"
                                && m.attrs.get("href").and_then(serde_json::Value::as_str)
                                    == Some("./new-target")
                        })
                    {
                        matched += 1;
                    }
                    true
                });
            assert_eq!(matched, 2);
            roundtrip(&gate, &source, raw, &expected);
        });
    }
    #[test]
    fn resource_image_dialog_changes_path_and_literal_alt_without_touching_other_source() {
        run(|| {
            let raw = "before\r\n\r\n![old](<./old.png> \"keep\") &amp; **stay**\n\n<custom>stay</custom>\r\n";
            let source = NativeView::fixture(raw);
            let (gate, controls) = at(&source, "![old]");
            controls.begin(ResourceKind::Image).unwrap();
            controls.url.set("../images/new photo.png".into());
            controls.alt.set("新 *字* [x] &".into());
            controls.apply().unwrap();
            let expected = raw.replace(
                "![old](<./old.png> \"keep\")",
                "![新 \\*字\\* \\[x\\] &amp;](<../images/new photo.png> \"keep\")",
            );
            roundtrip(&gate, &source, raw, &expected);
        });
    }
    #[test]
    fn resource_draft_cancel_and_ime_rejection_do_not_edit_body_history_or_view() {
        run(|| {
            let source = NativeView::fixture("[label](./old)\r\n");
            let (gate, controls) = at(&source, "label");
            let before = audit(&source);
            source.set_composing(true);
            assert!(controls.begin(ResourceKind::Link).is_err());
            assert_eq!(audit(&source), before);
            source.set_composing(false);
            controls.begin(ResourceKind::Link).unwrap();
            controls.url.set("./new".into());
            assert_eq!(audit(&source), before);
            controls.cancel();
            assert_eq!(audit(&source), before);
            controls.begin(ResourceKind::Link).unwrap();
            source.set_composing(true);
            assert!(controls.apply().is_err());
            source.set_composing(false);
            controls.cancel();
            assert_eq!(audit(&source), before);
            assert!(gate.rich_mode.get());
        });
    }
    #[test]
    fn resource_draft_rejects_stale_revision_and_invalid_values_without_another_edit() {
        run(|| {
            let source = NativeView::fixture("before\n\n[label](./old)\r\n");
            let (_, controls) = at(&source, "label");
            controls.begin(ResourceKind::Link).unwrap();
            controls.url.set(String::new());
            let before = audit(&source);
            assert!(controls.apply().is_err());
            assert_eq!(audit(&source), before);
            controls.url.set("./new".into());
            let snapshot = source.snapshot().unwrap();
            batch(|| {
                source
                    .edit(
                        snapshot.revision,
                        snapshot.view_epoch,
                        vec![(0..1, "B".into())],
                        ExternalSelections {
                            primary: 0,
                            ranges: vec![(1, 1)],
                        },
                        ExternalEditKind::Other,
                    )
                    .unwrap();
                source.notify();
            });
            let after_external = audit(&source);
            assert!(controls.apply().is_err());
            assert_eq!(audit(&source), after_external);
            controls.cancel();
        });
    }
}
