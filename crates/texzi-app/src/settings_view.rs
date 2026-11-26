use super::{divider, frame, secondary_foreground, selected_surface};
use crate::{native_menu, settings};
use nsh_core::{
    element::{input, scroll},
    prelude::*,
};
use serde_json::Value;
use std::{path::PathBuf, rc::Rc};

const CATEGORIES: &[&str] = &[
    "常用",
    "编辑器",
    "外观",
    "文件与恢复",
    "Markdown",
    "窗口与标签",
    "快捷键",
];
const COMMON: &[&str] = &[
    settings::THEME,
    settings::FONT_FAMILY,
    settings::FONT_SIZE,
    settings::WRAP_TEXT,
    settings::LINE_NUMBERS,
    settings::MARKDOWN_VIEW,
    settings::AUTO_SAVE,
    settings::RESTORE,
];

struct Binding {
    name: String,
    id: &'static str,
    key: String,
    source: &'static str,
    context: &'static str,
}

pub(super) struct SettingsView {
    section: Signal<&'static str>,
    query: Signal<String>,
    folder_scope: Signal<bool>,
    pub search: ElementRef,
    bindings: Vec<Binding>,
}

impl SettingsView {
    pub fn new(chords: &'static [(&'static str, &'static str)]) -> Self {
        let names = native_menu::commands();
        let rows = native_menu::SHELL_SHORTCUTS
            .iter()
            .map(|&(key, id)| {
                (
                    pretty_key(key),
                    id,
                    "窗口快捷键",
                    "窗口聚焦，模态与组字优先",
                )
            })
            .chain(chords.iter().map(|&(key, id)| {
                (
                    format!("Ctrl+K {}", pretty_key(key)),
                    id,
                    "窗口续键",
                    "窗口聚焦，等待 Ctrl+K 续键",
                )
            }));
        let bindings = rows
            .filter_map(|(key, id, source, context)| {
                let name = names.iter().find(|command| command.id == id)?.name.clone();
                Some(Binding {
                    name,
                    id,
                    key,
                    source,
                    context,
                })
            })
            .collect();
        Self {
            section: signal(CATEGORIES[0]),
            query: signal(String::new()),
            folder_scope: signal(false),
            search: element_ref(),
            bindings,
        }
    }
}

fn pretty_key(stroke: &str) -> String {
    stroke
        .split('+')
        .map(|part| match part {
            "mod" | "ctrl" => "Ctrl".into(),
            "alt" => "Alt".into(),
            "shift" => "Shift".into(),
            value => value.to_ascii_uppercase(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn choice(
    label: impl Into<SharedString>,
    selected: impl Fn() -> bool + Clone + 'static,
) -> nsh_base::ButtonElement {
    let fill = selected.clone();
    nsh_base::button()
        .h(px(34.0))
        .px(px(10.0))
        .items_center()
        .justify_center()
        .app_class("rounded-sm hover:bg-accent focus-visible:bg-accent")
        .bg(move || if fill() { selected_surface() } else { frame() })
        .selected(selected)
        .child(text(label.into()))
}

pub(super) fn sidebar(state: Rc<SettingsView>) -> impl IntoElement {
    let section = state.section;
    let query = state.query;
    view()
        .w(px(232.0))
        .flex_shrink_0()
        .min_h(px(0.0))
        .bg(frame)
        .border_r(px(1.0))
        .border_color(divider)
        .child(
            scroll()
                .flex_1()
                .child(
                    view()
                        .p(px(8.0))
                        .gap(px(2.0))
                        .children(CATEGORIES.iter().copied().map(move |category| {
                            choice(category, move || section.get() == category)
                                .h(px(36.0))
                                .items_start()
                                .on_press(move |_: &()| {
                                    section.set(category);
                                    query.set(String::new());
                                })
                        })),
                ),
        )
}

fn selected_root(state: &SettingsView, root: Signal<Option<PathBuf>>) -> Option<PathBuf> {
    if state.folder_scope.get() {
        root.get()
    } else {
        None
    }
}

#[derive(Clone, PartialEq)]
struct FieldDraft {
    root: Option<PathBuf>,
    text: String,
}

fn commit_field(
    settings: &settings::Settings,
    id: &'static str,
    current_root: Option<&std::path::Path>,
    draft: Signal<Option<FieldDraft>>,
) {
    let Some(pending) = draft.get_untracked() else {
        return;
    };
    if pending.root.as_deref() != current_root {
        draft.set(None);
        settings
            .error
            .set(Some("设置范围已改变，未提交原输入。".into()));
        return;
    }
    let value = pending.text.trim();
    let parsed = if settings::definition(id)["type"] == "integer" {
        value
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| value.into())
    } else {
        value.into()
    };
    if settings.apply(id, pending.root.as_deref(), Some(parsed)) {
        draft.set(None);
    }
}

fn control(
    settings: Rc<settings::Settings>,
    id: &'static str,
    state: Rc<SettingsView>,
    root: Signal<Option<PathBuf>>,
) -> AnyElement {
    let spec = settings::definition(id);
    if matches!(spec["type"].as_str(), Some("integer" | "string")) {
        let draft = signal(None::<FieldDraft>);
        let show = settings.clone();
        let show_scope = state.clone();
        let edit_scope = state.clone();
        let watch_scope = state.clone();
        effect(move || {
            let active_root = selected_root(&watch_scope, root);
            if draft
                .get_untracked()
                .is_some_and(|pending| pending.root != active_root)
            {
                draft.set(None);
            }
        });
        let commit = Rc::new(move || {
            commit_field(&settings, id, selected_root(&state, root).as_deref(), draft);
        });
        let blur = commit.clone();
        return input()
            .h(px(34.0))
            .w(px(if spec["type"] == "integer" {
                100.0
            } else {
                240.0
            }))
            .max_w(relative(1.0))
            .px(px(8.0))
            .bg(frame)
            .border(px(1.0))
            .border_color(divider)
            .label(spec["name"].as_str().unwrap())
            .value(move || {
                let active_root = selected_root(&show_scope, root);
                draft
                    .get()
                    .filter(|pending| pending.root == active_root)
                    .map(|pending| pending.text)
                    .unwrap_or_else(|| {
                        let value = show.effective(id, active_root.as_deref()).0;
                        value
                            .as_str()
                            .map_or_else(|| value.to_string(), str::to_owned)
                    })
            })
            .on_change(move |value: &str| {
                draft.update(|pending| {
                    let draft = pending.get_or_insert_with(|| FieldDraft {
                        root: selected_root(&edit_scope, root),
                        text: String::new(),
                    });
                    draft.text = value.into();
                })
            })
            .on_submit(move |_: &str| commit())
            .on_blur(move |_: &()| blur())
            .into_element();
    }
    view()
        .flex_row()
        .flex_wrap()
        .gap(px(4.0))
        .children(
            spec["options"]
                .as_array()
                .unwrap()
                .iter()
                .map(move |option| {
                    let label = match option["label"].as_str().unwrap() {
                        "on" => "开启",
                        "off" => "关闭",
                        label => label,
                    };
                    let value = option["value"].clone();
                    let selected = value.clone();
                    let apply = settings.clone();
                    let read = settings.clone();
                    let read_scope = state.clone();
                    let apply_scope = state.clone();
                    choice(label, move || {
                        read.effective(id, selected_root(&read_scope, root).as_deref())
                            .0
                            == selected
                    })
                    .on_press(move |_: &()| {
                        apply.apply(
                            id,
                            selected_root(&apply_scope, root).as_deref(),
                            Some(value.clone()),
                        );
                    })
                }),
        )
        .into_element()
}

fn row(
    settings: Rc<settings::Settings>,
    id: &'static str,
    state: Rc<SettingsView>,
    root: Signal<Option<PathBuf>>,
    narrow: Signal<bool>,
) -> impl IntoElement {
    let spec = settings::definition(id);
    let source = settings.clone();
    let source_scope = state.clone();
    let reset = settings.clone();
    let reset_scope = state.clone();
    let limited_scope = state.clone();
    let font = settings.clone();
    let font_scope = state.clone();
    view()
        .py(px(12.0))
        .border_b(px(1.0))
        .border_color(divider)
        .flex_row()
        .gap(px(16.0))
        .items_start()
        .style_when(move || narrow.get(), |style| style.flex_col())
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(4.0))
                .child(text(spec["name"].as_str().unwrap()))
                .child(
                    text(spec["description"].as_str().unwrap())
                        .text_size(px(12.0))
                        .text_color(secondary_foreground),
                )
                .child(
                    text(move || {
                        format!(
                            "来源 · {}",
                            source
                                .effective(id, selected_root(&source_scope, root).as_deref())
                                .1
                        )
                    })
                    .text_size(px(12.0))
                    .text_color(secondary_foreground),
                )
                .child(when(id == settings::FONT_FAMILY, move || {
                    let font = font.clone();
                    let font_scope = font_scope.clone();
                    text(move || {
                        let effective = font
                            .effective(id, selected_root(&font_scope, root).as_deref())
                            .0;
                        let stack = settings::source_font_stack(effective.as_str().unwrap());
                        let installed = nsh_platform_windows::installed_font_family(&stack);
                        format!(
                            "当前字体 · {}",
                            installed.as_deref().unwrap_or("Windows 字形回退")
                        )
                    })
                    .text_size(px(12.0))
                    .text_color(secondary_foreground)
                })),
        )
        .child(
            when(
                move || {
                    !settings::folder_supported(id) && selected_root(&limited_scope, root).is_some()
                },
                || text("此设置只支持用户范围").text_color(secondary_foreground),
            )
            .otherwise(move || {
                let reset = reset.clone();
                let reset_scope = reset_scope.clone();
                view()
                    .gap(px(6.0))
                    .w(px(240.0))
                    .max_w(relative(1.0))
                    .child(control(settings.clone(), id, state.clone(), root))
                    .child(
                        nsh_base::button()
                            .items_start()
                            .text_color(secondary_foreground)
                            .accessibility_label(format!("重置{}", spec["name"].as_str().unwrap()))
                            .on_press(move |_: &()| {
                                reset.apply(id, selected_root(&reset_scope, root).as_deref(), None);
                            })
                            .child(text("重置此项").text_size(px(12.0))),
                    )
            }),
        )
}

pub(super) fn content(
    settings: Rc<settings::Settings>,
    state: Rc<SettingsView>,
    root: Signal<Option<PathBuf>>,
    narrow: Signal<bool>,
) -> impl IntoElement {
    let query = state.query;
    let section = state.section;
    let folder = state.folder_scope;
    let notice = settings.error;
    let rows = state.clone();
    let retry_settings = settings.clone();
    let retry_scope = state.clone();
    view().size_full().min_h(px(0.0)).items_center().child(
        view()
            .w_full()
            .max_w(px(1040.0))
            .flex_1()
            .min_h(px(0.0))
            .p(px(32.0))
            .gap(px(16.0))
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .style_when(move || narrow.get(), |style| style.flex_col().items_start())
                    .child(text("设置").text_size(px(22.0)))
                    .child(
                        input()
                            .element_ref(&state.search)
                            .auto_focus(true)
                            .label("搜索设置")
                            .placeholder("搜索设置")
                            .h(px(34.0))
                            .w(px(320.0))
                            .max_w(relative(1.0))
                            .px(px(8.0))
                            .bg(frame)
                            .border(px(1.0))
                            .border_color(divider)
                            .value(move || query.get())
                            .on_change(move |value: &str| query.set(value.into())),
                    ),
            )
            .child(
                view()
                    .flex_row()
                    .gap(px(6.0))
                    .child(
                        choice("用户", move || !folder.get() || root.get().is_none())
                            .on_press(move |_: &()| folder.set(false)),
                    )
                    .child(when(
                        move || root.get().is_some(),
                        move || {
                            choice("当前文件夹", move || folder.get())
                                .on_press(move |_: &()| folder.set(true))
                        },
                    )),
            )
            .child(when(
                move || notice.get().is_some(),
                move || text(move || notice.get().unwrap_or_default()),
            ))
            .child(dynamic(move || {
                let scope = selected_root(&retry_scope, root);
                let pending = retry_settings.pending_for_scope(scope.as_deref());
                (!pending.is_empty()).then(|| {
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(6.0))
                        .children(pending.into_iter().map(|entry| {
                            let settings = retry_settings.clone();
                            let state = retry_scope.clone();
                            let label = entry.label();
                            nsh_base::button()
                                .child(text(label))
                                .on_press(move |_: &()| {
                                    settings.retry(&entry, selected_root(&state, root).as_deref());
                                })
                        }))
                })
            }))
            .child(scroll().flex_1().min_h(px(0.0)).child(dynamic(move || {
                let term = query.get().trim().to_lowercase();
                let category = section.get();
                if category == "快捷键" {
                    let mut items = view().gap(px(6.0));
                    let mut found = false;
                    for binding in &rows.bindings {
                        if ![
                            binding.name.as_str(),
                            binding.id,
                            binding.key.as_str(),
                            binding.context,
                            binding.source,
                        ]
                        .iter()
                        .any(|part| part.to_lowercase().contains(&term))
                        {
                            continue;
                        }
                        found = true;
                        items = items.child(
                            view()
                                .flex_row()
                                .py(px(10.0))
                                .gap(px(12.0))
                                .border_b(px(1.0))
                                .border_color(divider)
                                .style_when(move || narrow.get(), |style| style.flex_col())
                                .child(
                                    view().flex_1().child(text(binding.name.clone())).child(
                                        text(format!("{} · {}", binding.source, binding.context))
                                            .text_size(px(12.0))
                                            .text_color(secondary_foreground),
                                    ),
                                )
                                .child(text(binding.key.clone())),
                        );
                    }
                    return if found {
                        items.into_element()
                    } else {
                        no_results(term, query)
                    };
                }
                let selected = settings::ENABLED
                    .iter()
                    .copied()
                    .filter(|id| {
                        let spec = settings::definition(id);
                        if term.is_empty() {
                            return if category == "常用" {
                                COMMON.contains(id)
                            } else {
                                spec["category"] == category
                            };
                        }
                        [
                            *id,
                            spec["name"].as_str().unwrap(),
                            spec["description"].as_str().unwrap(),
                        ]
                        .iter()
                        .any(|part| part.to_lowercase().contains(&term))
                    })
                    .collect::<Vec<_>>();
                if selected.is_empty() {
                    return if term.is_empty() {
                        text("此分类暂未接入可调设置")
                            .text_color(secondary_foreground)
                            .into_element()
                    } else {
                        no_results(term, query)
                    };
                }
                let mut list = view();
                if term.is_empty() {
                    list = list.child(text(category).text_size(px(16.0)));
                }
                for id in selected {
                    list = list.child(row(settings.clone(), id, rows.clone(), root, narrow));
                }
                list.into_element()
            }))),
    )
}

fn no_results(term: String, query: Signal<String>) -> AnyElement {
    view()
        .gap(px(8.0))
        .items_start()
        .child(text(format!("没有与「{term}」匹配的项目")))
        .child(
            nsh_base::button()
                .on_press(move |_: &()| query.set(String::new()))
                .child(text("清除搜索")),
        )
        .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[component]
    fn DraftScopeFixture() -> impl IntoElement {
        let preferences = settings::Settings::new();
        let old_root = PathBuf::from(r"C:\texzi-test\original-folder");
        let view = SettingsView::new(&[]);
        view.folder_scope.set(true);
        let root = signal(Some(old_root.clone()));
        let draft = signal(Some(FieldDraft {
            root: selected_root(&view, root),
            text: "23".into(),
        }));
        root.set(None);
        commit_field(
            &preferences,
            settings::FONT_SIZE,
            selected_root(&view, root).as_deref(),
            draft,
        );
        assert!(draft.get().is_none());
        assert_eq!(
            preferences.effective(settings::FONT_SIZE, None).0,
            Value::from(16)
        );
        assert_eq!(
            preferences
                .effective(settings::FONT_SIZE, Some(&old_root))
                .0,
            Value::from(16)
        );
        assert!(preferences.error.get().is_some());
        draft.set(Some(FieldDraft {
            root: selected_root(&view, root),
            text: "19".into(),
        }));
        commit_field(&preferences, settings::FONT_SIZE, None, draft);
        assert_eq!(
            preferences.effective(settings::FONT_SIZE, None).0,
            Value::from(19)
        );
        text("")
    }

    #[test]
    fn closing_folder_does_not_commit_its_number_draft_to_user_settings() {
        use nsh_core::{ApproxTextEngine, Core, Size, element::Root};
        let mut core = Core::new(
            ApproxTextEngine,
            Size {
                width: px(800.0),
                height: px(600.0),
            },
        );
        core.mount_root(&Root::new::<DraftScopeFixture>(|| ()));
        core.run_component_tasks();
        let _ = core.commit();
    }
}
