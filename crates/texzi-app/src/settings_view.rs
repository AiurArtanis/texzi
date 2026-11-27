use super::{divider, frame, secondary_foreground, selected_surface};
use crate::origami::{CONTROL_STATES, border_width, color, font_size, metric};
use crate::{native_menu, settings};
use nsh_core::{
    AccessibilityRole,
    element::{input, scroll},
    prelude::*,
};
use serde_json::Value;
use std::{path::PathBuf, rc::Rc};

const THEME_CARD_WIDTH: f32 = 108.0;
const THEME_CARD_GAP: f32 = 10.0;
const THEME_OPTIONS_WIDTH: f32 = 3.0 * THEME_CARD_WIDTH + 2.0 * THEME_CARD_GAP;

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
    target: Signal<Option<&'static str>>,
    field: ElementRef,
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
            target: signal(None),
            field: element_ref(),
            search: element_ref(),
            bindings,
        }
    }

    pub(super) fn locate(&self, id: &'static str) {
        let category = settings::definition(id)["category"].as_str().unwrap();
        self.section.set(category);
        self.target.set(Some(id));
        self.query.set(id.into());
    }

    pub(super) fn focus(&self) {
        if self.target.get_untracked().is_some() && self.field.node().is_some() {
            self.field.focus();
        } else {
            self.search.focus();
        }
    }
}

pub(super) fn spotlight_entries() -> impl Iterator<Item = nsh_base::CommandItem> {
    settings::ENABLED.iter().map(|&id| {
        let name = settings::definition(id)["name"].as_str().unwrap();
        nsh_base::CommandItem::new(format!("setting:{id}"), format!("{name} · {id}"))
            .keywords([id])
            .group("设置")
    })
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

fn choice_control(selected: impl Fn() -> bool + Clone + 'static) -> nsh_base::ButtonElement {
    let fill = selected.clone();
    let ink = selected.clone();
    nsh_base::button()
        .h(px(metric("field")))
        .px(px(10.0))
        .items_center()
        .justify_center()
        .app_class(CONTROL_STATES)
        .rounded(px(6.0))
        .bg(move || if fill() { selected_surface() } else { frame() })
        .text_color(move || color(if ink() { "selected-text" } else { "ink-1" }))
        .selected(selected)
}

fn choice(
    label: impl Into<SharedString>,
    selected: impl Fn() -> bool + Clone + 'static,
) -> nsh_base::ButtonElement {
    choice_control(selected).child(text(label.into()))
}

fn theme_choice(
    label: &'static str,
    mode: &str,
    selected: impl Fn() -> bool + Clone + 'static,
) -> nsh_base::ButtonElement {
    let chosen = selected.clone();
    let sample = crate::origami::sample_color("paper");
    let first = if mode == "dark" {
        sample.dark
    } else {
        sample.light
    };
    let second = if mode == "light" {
        sample.light
    } else {
        sample.dark
    };
    choice_control(selected)
        .w(px(THEME_CARD_WIDTH))
        .max_w(relative(1.0))
        .h(px(78.0))
        .p(px(6.0))
        .gap(px(5.0))
        .border(border_width)
        .border_color(move || color(if chosen() { "iris" } else { "chrome-edge" }))
        .accessibility_label(label)
        .child(
            view()
                .w_full()
                .h(px(42.0))
                .rounded(px(4.0))
                .bg(first)
                .flex_row()
                .child(view().flex_1())
                .child(view().flex_1().bg(second).rounded_r(px(4.0))),
        )
        .child(text(label).text_size(px(font_size("meta"))))
}

pub(super) fn sidebar(state: Rc<SettingsView>, compact: bool) -> impl IntoElement {
    let section = state.section;
    let query = state.query;
    let target = state.target;
    let labels = if compact {
        view().flex_row().items_center()
    } else {
        view()
    };
    let categories = labels
        .p(px(6.0))
        .gap(px(2.0))
        .children(CATEGORIES.iter().copied().map(move |category| {
            choice(category, move || section.get() == category)
                .flex_shrink_0()
                .items_start()
                .on_press(move |_: &()| {
                    section.set(category);
                    target.set(None);
                    query.set(String::new());
                })
        }));
    if compact {
        view()
            .h(px(44.0))
            .w_full()
            .flex_shrink_0()
            .border_b(border_width)
            .border_color(divider)
            .child(scroll().horizontal().child(categories))
    } else {
        view()
            .w(px(metric("sidebar")))
            .flex_shrink_0()
            .min_h(px(0.0))
            .bg(|| color("mica-sidebar"))
            .border_r(border_width)
            .border_color(divider)
            .child(scroll().flex_1().child(categories))
    }
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
    if id == settings::DEFAULT_ENCODING {
        return dynamic(move || {
            let scope = selected_root(&state, root);
            Some(encoding_picker(
                settings.clone(),
                state.clone(),
                root,
                scope,
            ))
        })
        .into_element();
    }
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
            .h(px(metric("field")))
            .w(px(if spec["type"] == "integer" {
                100.0
            } else {
                240.0
            }))
            .max_w(relative(1.0))
            .px(px(8.0))
            .bg(frame)
            .border(border_width)
            .border_color(divider)
            .app_class(CONTROL_STATES)
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
        .gap(px(if id == settings::THEME {
            THEME_CARD_GAP
        } else {
            4.0
        }))
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
                    let is_selected = move || {
                        read.effective(id, selected_root(&read_scope, root).as_deref())
                            .0
                            == selected
                    };
                    let button = if id == settings::THEME {
                        theme_choice(label, value.as_str().unwrap_or_default(), is_selected)
                    } else {
                        choice(label, is_selected)
                    };
                    button.on_press(move |_: &()| {
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

fn encoding_picker(
    prefs: Rc<settings::Settings>,
    state: Rc<SettingsView>,
    root: Signal<Option<PathBuf>>,
    scope: Option<PathBuf>,
) -> impl IntoElement {
    let definition = settings::definition(settings::DEFAULT_ENCODING);
    let options = definition["options"].as_array().unwrap();
    let read = prefs.clone();
    let read_scope = scope.clone();
    let selected = computed(move || {
        let value = read
            .effective(settings::DEFAULT_ENCODING, read_scope.as_deref())
            .0;
        SharedString::from(value.as_str().unwrap_or("utf-8").to_owned())
    });
    let choose = Rc::new(move |encoding: &str| {
        if selected_root(&state, root) != scope {
            prefs
                .error
                .set(Some("设置范围已改变，未提交原选择。".into()));
            return;
        }
        prefs.apply(
            settings::DEFAULT_ENCODING,
            scope.as_deref(),
            Some(encoding.into()),
        );
    });
    let choose_from_picker = choose.clone();
    view()
        .flex_row()
        .flex_wrap()
        .gap(px(4.0))
        .child(
            nsh_base::select()
                .items(
                    options
                        .iter()
                        .map(|option| {
                            nsh_base::SelectItem::new(
                                option["value"].as_str().unwrap(),
                                option["label"].as_str().unwrap(),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
                .accessibility_label("新文件默认编码")
                .value(move || nsh_base::SelectValue::from(selected.get()))
                .popup_class(|| {
                    Some(SharedString::from(format!(
                        "rounded-md border-[{}px] border-(--chrome-edge) bg-(--glass-solid) text-(--glass-label)",
                        border_width().0
                    )))
                })
                .trigger(
                    view()
                        .h(px(metric("field")))
                        .px(px(9.0))
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .app_class(CONTROL_STATES)
                        .rounded(px(6.0))
                        .border(border_width)
                        .border_color(divider)
                        .bg(frame)
                        .child(text(move || {
                            let value = selected.get();
                            options
                                .iter()
                                .find(|option| option["value"] == value.as_ref())
                                .and_then(|option| option["label"].as_str())
                                .unwrap_or_default()
                        }))
                        .child(text("▾").text_color(secondary_foreground)),
                )
                .on_change(move |value: &Option<SharedString>| {
                    if let Some(value) = value {
                        choose_from_picker(value);
                    }
                }),
        )
        .children(
            [
                ("utf-8-bom", "UTF-8 BOM"),
                ("gb18030", "GB18030"),
                ("utf-16le", "UTF-16 LE"),
            ]
            .into_iter()
            .map(move |(value, label)| {
                let choose = choose.clone();
                choice(label, move || selected.get().as_ref() == value)
                    .on_press(move |_: &()| choose(value))
            }),
        )
}

fn setting_description(id: &str) -> &'static str {
    match id {
        settings::RESTORE => {
            "下次启动恢复窗口、标签和未保存的临时文本。关闭此设置会保留已有恢复记录。"
        }
        _ => settings::definition(id)["description"].as_str().unwrap(),
    }
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
        .border_b(border_width)
        .border_color(divider)
        .flex_row()
        .gap(px(16.0))
        .items_start()
        .when(state.target.get() == Some(id), |row| {
            row.element_ref(&state.field)
                .focusable(true)
                .role(AccessibilityRole::Group)
                .label(spec["name"].as_str().unwrap())
                .app_class("focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring")
        })
        .style_when(move || narrow.get(), |style| style.flex_col())
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(4.0))
                .child(text(spec["name"].as_str().unwrap()))
                .child(
                    text(setting_description(id))
                        .text_size(px(font_size("meta")))
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
                    .text_size(px(font_size("meta")))
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
                    .text_size(px(font_size("meta")))
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
                    .w(px(if matches!(id, settings::THEME | settings::DEFAULT_ENCODING) {
                        THEME_OPTIONS_WIDTH
                    } else {
                        240.0
                    }))
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
                            .child(text("重置此项").text_size(px(font_size("meta")))),
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
    let target = state.target;
    let notice = settings.error;
    let rows = state.clone();
    let search = state.clone();
    let term = computed(move || query.with(|query| query.trim().to_lowercase()));
    let results = computed(move || matching_rows(&search, &term.get()));
    let retry_settings = settings.clone();
    let retry_scope = state.clone();
    view().size_full().min_h(px(0.0)).items_center().child(
        view()
            .w_full()
            .max_w(px(1040.0))
            .flex_1()
            .min_h(px(0.0))
            .p(move || px(if narrow.get() { 12.0 } else { 32.0 }))
            .gap(px(16.0))
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .style_when(move || narrow.get(), |style| style.flex_col().items_start())
                    .child(text("设置").text_size(px(font_size("section"))))
                    .child(
                        view()
                            .w(px(320.0))
                            .max_w(relative(1.0))
                            .gap(px(4.0))
                            .child(
                                input()
                                    .element_ref(&state.search)
                                    .auto_focus(true)
                                    .app_class(CONTROL_STATES)
                                    .label("搜索设置")
                                    .placeholder("搜索设置")
                                    .h(px(metric("field")))
                                    .w_full()
                                    .px(px(8.0))
                                    .bg(frame)
                                    .border(border_width)
                                    .border_color(divider)
                                    .value(move || query.get())
                                    .on_change(move |value: &str| {
                                        target.set(None);
                                        query.set(value.into());
                                    }),
                            )
                            .child(when(
                                move || !term.get().is_empty(),
                                move || {
                                    text(move || {
                                        results.with(|results| format!("{} 项匹配", results.len()))
                                    })
                                    .text_color(secondary_foreground)
                                    .text_size(px(font_size("meta")))
                                },
                            )),
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
                let term = term.get();
                let results = results.get();
                if results.len() == 0 {
                    return if term.is_empty() {
                        text("此分类暂未接入可调设置")
                            .text_color(secondary_foreground)
                            .into_element()
                    } else {
                        no_results(term, query)
                    };
                }
                if let SettingsRows::Bindings(indices) = results {
                    let mut items = view().gap(px(6.0));
                    for index in indices {
                        let binding = &rows.bindings[index];
                        items = items.child(
                            view()
                                .flex_row()
                                .py(px(10.0))
                                .gap(px(12.0))
                                .border_b(border_width)
                                .border_color(divider)
                                .style_when(move || narrow.get(), |style| style.flex_col())
                                .child(
                                    view().flex_1().child(text(binding.name.clone())).child(
                                        text(format!("{} · {}", binding.source, binding.context))
                                            .text_size(px(font_size("meta")))
                                            .text_color(secondary_foreground),
                                    ),
                                )
                                .child(text(binding.key.clone())),
                        );
                    }
                    return items.into_element();
                }
                let SettingsRows::Fields(selected) = results else {
                    unreachable!()
                };
                let mut list = view();
                if term.is_empty() {
                    list = list.child(text(section.get()).text_size(px(16.0)));
                }
                for id in selected {
                    list = list.child(row(settings.clone(), id, rows.clone(), root, narrow));
                }
                list.into_element()
            }))),
    )
}

#[derive(Clone, PartialEq)]
enum SettingsRows {
    Fields(Vec<&'static str>),
    Bindings(Vec<usize>),
}

impl SettingsRows {
    fn len(&self) -> usize {
        match self {
            Self::Fields(fields) => fields.len(),
            Self::Bindings(bindings) => bindings.len(),
        }
    }
}

fn matching_rows(state: &SettingsView, term: &str) -> SettingsRows {
    if let Some(id) = state.target.get() {
        return SettingsRows::Fields(vec![id]);
    }
    let category = state.section.get();
    if category == "快捷键" {
        return SettingsRows::Bindings(
            state
                .bindings
                .iter()
                .enumerate()
                .filter_map(|(index, binding)| {
                    [
                        binding.name.as_str(),
                        binding.id,
                        binding.key.as_str(),
                        binding.context,
                        binding.source,
                    ]
                    .iter()
                    .any(|part| part.to_lowercase().contains(term))
                    .then_some(index)
                })
                .collect(),
        );
    }
    SettingsRows::Fields(
        settings::ENABLED
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
                [*id, spec["name"].as_str().unwrap(), setting_description(id)]
                    .iter()
                    .any(|part| part.to_lowercase().contains(term))
            })
            .collect(),
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

    #[component]
    fn DefaultEncodingFixture() -> impl IntoElement {
        let settings = Rc::new(settings::Settings::new());
        let view = Rc::new(SettingsView::new(&[]));
        view.locate(settings::DEFAULT_ENCODING);
        content(settings, view, signal(None), signal(false))
    }

    #[test]
    fn default_encoding_combobox_contains_all_seven_real_encodings() {
        use nsh_core::{
            ApproxTextEngine, Core, EventKind, EventPayload, MountItem, Size,
            element::{Environment, Root},
            platform::Os,
        };
        let size = Size::new(px(800.0), px(640.0));
        let mut core = Core::new(ApproxTextEngine, size);
        let mut environment = Environment::default();
        environment.os = Os::Windows;
        environment.window = size;
        core.set_environment(environment);
        core.mount_root(&Root::new::<DefaultEncodingFixture>(|| ()));
        let mut trigger = None;
        for _ in 0..5 {
            core.run_component_tasks();
            if let Some(batch) = core.commit() {
                for item in batch.items {
                    if let MountItem::ViewProps { id, props } = item
                        && props.accessibility.role == AccessibilityRole::Combobox
                        && props.accessibility.label.as_deref() == Some("新文件默认编码")
                    {
                        trigger = Some(id);
                    }
                }
            }
        }
        let trigger = trigger.unwrap();
        let frame = core.measure(trigger).unwrap().frame;
        assert!(core.component_event(trigger, EventKind::Layout, &EventPayload::Layout(frame)));
        assert!(core.component_event(trigger, EventKind::Press, &EventPayload::None));
        let mut labels = std::collections::BTreeSet::new();
        for _ in 0..6 {
            core.run_component_tasks();
            if let Some(batch) = core.commit() {
                for item in batch.items {
                    match item {
                        MountItem::ViewProps { props, .. } => {
                            if let Some(label) = props.accessibility.label {
                                labels.insert(label.to_string());
                            }
                        }
                        MountItem::AccessibilityLabel {
                            label: Some(label), ..
                        } => {
                            labels.insert(label.to_string());
                        }
                        _ => {}
                    }
                }
            }
        }
        for expected in settings::definition(settings::DEFAULT_ENCODING)["options"]
            .as_array()
            .unwrap()
        {
            let label = expected["label"].as_str().unwrap();
            assert!(
                labels.contains(label),
                "missing {label}; observed {labels:?}"
            );
        }
    }
}
