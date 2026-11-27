use super::*;
use nsh_base::{
    editor::{EditorDecoration, EditorPosition, EditorRange},
    rich_editor::state::Decoration,
};
use texzi_document::Snapshot;
use texzi_search::Rope;

pub(in crate::app_shell) fn overlay(s: Rc<Shell>, doc: Rc<Document>) -> impl IntoElement {
    let show = s.clone();
    let bound = doc.clone();
    when(
        move || active(&show, &bound),
        move || {
            let s = s.clone();
            let doc = doc.clone();
            bindings(&s);
            let observer = s.clone();
            let rich_doc = doc.clone();
            effect(move || {
                if rich_doc.displayed_rich()
                    && let Some(api) = rich_doc.rich_reference.api::<RichEditorApi>()
                    && let Some(range) = api.visible_document_range()
                {
                    untracked(|| viewport_changed(&observer, &rich_doc, range));
                }
            });
            let observer = s.clone();
            let rich_doc = doc.clone();
            effect(move || {
                observer.find_panel.marks_rev.get();
                if rich_doc.displayed_rich()
                    && let Some(api) = rich_doc.rich_reference.api::<RichEditorApi>()
                {
                    let st = &observer.find_panel;
                    if !rich_doc
                        .source
                        .search_snapshot()
                        .is_ok_and(|(snap, _)| st.version.get() == Some((snap.revision.0, true)))
                    {
                        api.set_external_decorations(Vec::new());
                        return;
                    }
                    let current = st.current_mark.borrow().clone();
                    let mut marks = Vec::new();
                    let mut points = Vec::new();
                    for range in st
                        .visible
                        .borrow()
                        .iter()
                        .filter(|r| Some(*r) != current.as_ref())
                        .chain(current.iter())
                    {
                        if !range.is_empty() {
                            let style = highlight_class(Some(range) == current.as_ref(), dark());
                            marks.push(Decoration::inline(
                                range.start,
                                range.end,
                                [("class".into(), serde_json::json!(style))]
                                    .into_iter()
                                    .collect(),
                            ));
                        } else if Some(range) != current.as_ref() {
                            points.push((range.start, outline_color(false, dark())));
                        }
                    }
                    api.set_external_decorations_with_outline(
                        marks,
                        current.map(|r| (r, outline_color(true, dark()))),
                        points,
                    );
                }
            });
            let st = &s.find_panel;
            let query = st.query;
            let value = st.replacement;
            let input_ref = st.query_input;
            let r_comp = st.composing_replacement;
            let mode = s.panel;
            let status = st.status;
            let edit = s.clone();
            let preedit = s.clone();
            let submit = s.clone();
            let expand = s.clone();
            let prev = s.clone();
            let next = s.clone();
            let close = s.clone();
            let all = s.clone();
            let r_actions = s.clone();
            let scoped = st.scoped;
            let glyph = move || {
                if mode.get() == "replace" {
                    "⌄"
                } else {
                    "›"
                }
            };
            let input_row = view()
                .flex_row()
                .items_center()
                .gap(px(2.0))
                .child(
                    action("显示或收起替换")
                        .child(text(glyph))
                        .on_press(move |_: &()| {
                            if expand.panel.get() == "replace" {
                                expand.panel.set("find".into());
                            } else {
                                open(&expand, true);
                            }
                        }),
                )
                .child(
                    input()
                        .element_ref(&input_ref)
                        .auto_focus(true)
                        .label("查找文字")
                        .placeholder("查找文字")
                        .flex_1()
                        .min_w(px(0.0))
                        .h(px(28.0))
                        .value(move || query.get())
                        .on_composition_change(move |e: &Composition| {
                            let was = preedit.find_panel.composing_query.get();
                            preedit.find_panel.composing_query.set(e.composing);
                            if e.composing {
                                preedit.find_panel.supersede();
                            }
                            if was && !e.composing {
                                refresh(&preedit, Intent::Count, true);
                            }
                        })
                        .on_change(move |v: &str| query_changed(&edit, v))
                        .on_submit(move |_: &str| navigate(&submit, false)),
                )
                .child(
                    text(move || {
                        let scope = if scoped.get() {
                            "选区"
                        } else if doc.displayed_rich() {
                            "正文"
                        } else {
                            "源码"
                        };
                        short_status(&status.get(), scope)
                    })
                    .text_size(px(11.0))
                    .text_color(foreground)
                    .max_w(px(151.0))
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail),
                )
                .child(toggle(&s, Toggle::Case, "Aa", "区分大小写"))
                .child(toggle(&s, Toggle::Word, "ab", "整词匹配"))
                .child(toggle(&s, Toggle::Regex, ".*", "正则表达式"))
                .child(toggle(&s, Toggle::Selection, "▧", "仅在选区内查找"))
                .child(
                    action("选中全部匹配")
                        .child(text("≣"))
                        .on_press(move |_: &()| select_all(&all)),
                )
                .child(
                    action("上一个")
                        .child(text("↑"))
                        .on_press(move |_: &()| navigate(&prev, true)),
                )
                .child(
                    action("下一个")
                        .child(text("↓"))
                        .on_press(move |_: &()| navigate(&next, false)),
                )
                .child(
                    action("关闭文内查找")
                        .child(text("×"))
                        .on_press(move |_: &()| close.close_panel()),
                );
            let controls = view()
                .w(px(396.0))
                .max_w(relative(1.0))
                .p(px(5.0))
                .gap(px(4.0))
                .border(px(1.0))
                .border_color(divider)
                .bg(frame)
                .app_class("rounded-md")
                .child(input_row)
                .child(when(
                    move || mode.get() == "replace",
                    move || {
                        let s = r_actions.clone();
                        let replacement = s.clone();
                        let submit_replacement = s.clone();
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .pl(px(24.0))
                            .child(
                                input()
                                    .label("替换为")
                                    .placeholder("替换为")
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .h(px(28.0))
                                    .value(move || value.get())
                                    .on_composition_change(move |e: &Composition| {
                                        r_comp.set(e.composing)
                                    })
                                    .on_submit(move |_: &str| replace(&submit_replacement, false))
                                    .on_change(move |v: &str| {
                                        value.set(v.into());
                                        if replacement.find_panel.status.get() == "正在准备替换…"
                                        {
                                            replacement.find_panel.supersede();
                                            replacement
                                                .find_panel
                                                .status
                                                .set("替换内容已更新".into());
                                        }
                                    }),
                            )
                            .child(button("替换当前", s.clone(), "texzi.find.replaceOne"))
                            .child(button("全部替换", s.clone(), "texzi.find.replaceAll"))
                    },
                ))
                .child(when(
                    move || is_message(&status.get()),
                    move || {
                        text(move || status.get())
                            .text_size(px(11.0))
                            .text_color(foreground)
                    },
                ));
            view()
                .absolute()
                .top(px(42.0))
                .left(px(4.0))
                .right(px(4.0))
                .items_end()
                .z_index(3)
                .pointer_events(PointerEvents::BoxNone)
                .child(controls)
        },
    )
}
fn is_message(status: &str) -> bool {
    !status.contains(" · ")
        && !matches!(
            status,
            "" | "请输入查找文字" | "正在查找…" | "已找到匹配，正在计数…"
        )
}
fn short_status(status: &str, active_scope: &str) -> String {
    if let Some((scope, rest)) = status.split_once(" · ") {
        let number = rest.split(' ').next().unwrap_or(rest);
        format!("{scope} {number}")
    } else {
        match status {
            "正在查找…" => format!("{active_scope} 查找中"),
            "已找到匹配，正在计数…" => format!("{active_scope} 计数中"),
            _ => active_scope.into(),
        }
    }
}
fn action(label: &'static str) -> nsh_base::ButtonElement {
    nsh_base::button()
        .accessibility_label(label)
        .w(px(21.0))
        .h(px(26.0))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .app_class("rounded-sm hover:bg-accent focus-visible:bg-accent")
}
fn toggle(
    s: &Rc<Shell>,
    which: Toggle,
    label: &'static str,
    name: &'static str,
) -> impl IntoElement {
    let run = s.clone();
    let flag = match which {
        Toggle::Case => s.find_panel.case,
        Toggle::Word => s.find_panel.word,
        Toggle::Regex => s.find_panel.regex,
        Toggle::Selection => s.find_panel.scoped,
    };
    let available = s.find_panel.has_scope;
    action(name)
        .selected(move || flag.get())
        .disabled(move || which == Toggle::Selection && !available.get())
        .accessibility_hint(if which == Toggle::Selection {
            "打开查找前先在正文选择文字"
        } else {
            name
        })
        .bg(move || {
            if flag.get() {
                selected_surface()
            } else {
                frame()
            }
        })
        .child(text(label).text_size(px(12.0)).text_color(foreground))
        .on_press(move |_: &()| change(&run, which))
}
fn bindings(s: &Rc<Shell>) {
    let shell = s.clone();
    effect(move || {
        if shell.chord.get() {
            return;
        }
        for (key, command) in [
            ("escape", "texzi.find.close"),
            ("shift+escape", "texzi.find.close"),
            ("shift+enter", "texzi.find.previous"),
            ("alt+enter", "texzi.find.selectAllMatches"),
            ("alt+c", "texzi.find.toggleCase"),
            ("alt+w", "texzi.find.toggleWholeWord"),
            ("alt+r", "texzi.find.toggleRegex"),
            ("alt+l", "texzi.find.toggleSelection"),
            ("mod+shift+1", "texzi.find.replaceOne"),
            ("mod+alt+enter", "texzi.find.replaceAll"),
        ] {
            let s = shell.clone();
            on_key_with(
                key,
                KeyOptions {
                    when: nsh_core::keymap::When::Always,
                    pass_through: command == "texzi.find.close",
                    ..Default::default()
                },
                move |_| {
                    if !is_open(&s.panel.get())
                        || s.chord.get()
                        || s.escape_blocked()
                        || s.find_panel.editing()
                    {
                        return;
                    }
                    s.run(command);
                },
            );
        }
    });
}

/// Editor offsets are UTF-16 columns in a logical line. Converting with the
/// measured Rope avoids reading the full line, including very long lines.
fn byte(rope: &Rope, p: EditorPosition) -> usize {
    if p.line as usize >= rope.len_lines() {
        return rope.len_bytes();
    }
    let start = rope.line_to_char(p.line as usize);
    let end = if p.line as usize + 1 < rope.len_lines() {
        rope.line_to_char(p.line as usize + 1)
    } else {
        rope.len_chars()
    };
    let u = (rope.char_to_utf16_cu(start) + p.column as usize).min(rope.char_to_utf16_cu(end));
    rope.char_to_byte(rope.utf16_cu_to_char(u))
}
fn position(rope: &Rope, byte: usize) -> EditorPosition {
    let c = rope.byte_to_char(byte.min(rope.len_bytes()));
    let line = rope.char_to_line(c);
    EditorPosition::new(
        line as u32,
        (rope.char_to_utf16_cu(c) - rope.char_to_utf16_cu(rope.line_to_char(line))) as u32,
    )
}
pub(super) fn visible_band(doc: &Document, snap: &Snapshot) -> Option<Range<usize>> {
    if doc.displayed_rich() {
        return doc
            .rich_reference
            .api::<RichEditorApi>()?
            .visible_document_range();
    }
    let range = doc.reference.api::<EditorApi>()?.visible_range();
    Some(byte(snap.body.source(), range.start)..byte(snap.body.source(), range.end))
}
pub(in crate::app_shell) fn source_viewport(s: &Rc<Shell>, doc: &Rc<Document>, range: EditorRange) {
    if !active(s, doc) {
        return;
    }
    if let Ok((snap, _)) = doc.source.search_snapshot() {
        viewport_changed(
            s,
            doc,
            byte(snap.body.source(), range.start)..byte(snap.body.source(), range.end),
        );
    }
}
fn viewport_changed(s: &Rc<Shell>, doc: &Rc<Document>, range: Range<usize>) {
    if *s.find_panel.viewport.borrow() == range {
        return;
    }
    s.find_panel.viewport.replace(range);
    note_present(s, doc);
    request_marks(s, doc, false);
}
pub(super) fn request_marks(s: &Rc<Shell>, doc: &Rc<Document>, immediate: bool) {
    let st = &s.find_panel;
    st.stop_viewport();
    let Some(index) = st.index.borrow().clone() else {
        return;
    };
    let generation = st.generation.get();
    let n = st.viewport_number.get();
    let range = st.viewport.borrow().clone();
    let weak = Rc::downgrade(s);
    let doc = doc.clone();
    let owner = s.owner;
    owner.run(|| {
        set_timeout(
            Duration::from_millis(if immediate { 0 } else { 30 }),
            move || {
                let Some(s) = weak.upgrade() else { return };
                if !valid(&s, &doc, generation) || s.find_panel.viewport_number.get() != n {
                    return;
                }
                let cancel = Arc::new(AtomicBool::new(false));
                s.find_panel.viewport_cancel.replace(Some(cancel.clone()));
                spawn(async move {
                    let result = blocking(move || {
                        index
                            .visible(range, &cancel)
                            .map(|v| to_view(&index, v.iter()))
                    })
                    .await;
                    if valid(&s, &doc, generation)
                        && s.find_panel.viewport_number.get() == n
                        && let Ok(Ok(ranges)) = result
                    {
                        s.find_panel.visible.replace(ranges);
                        s.find_panel.marks_rev.update(|v| *v += 1);
                    }
                });
            },
        )
    });
}
fn highlight_class(current: bool, dark: bool) -> &'static str {
    match (current, dark) {
        (true, true) => "bg-[#955239] text-[#ffffff]",
        (true, false) => "bg-[#eaa077] text-[#25160e]",
        (false, true) => "bg-[#5b432f] text-[#fff0d8]",
        (false, false) => "bg-[#ffdeb9] text-[#332313]",
    }
}
fn outline_color(current: bool, dark: bool) -> Hsla {
    let rgb_code = match (current, dark) {
        (true, false) => 0xe07a48,
        (true, true) => 0xf6b28e,
        (false, false) => 0xecc5a4,
        (false, true) => 0x82634e,
    };
    rgb(rgb_code).into()
}
pub(super) fn note_present(s: &Shell, doc: &Document) {
    let st = &s.find_panel;
    let Some(start) = st.started.get() else {
        return;
    };
    let request = st.generation.get();
    if st.frame_probe.get() == Some(request) {
        return;
    }
    let view = st.viewport.borrow();
    let intersects = |r: &Range<usize>| r.start <= view.end && r.end >= view.start;
    if !st.visible.borrow().iter().any(intersects)
        && !st.current_mark.borrow().as_ref().is_some_and(intersects)
    {
        return;
    }
    st.frame_probe.set(Some(request));
    st.metric("visible_marks_ready", 1);
    let flag = st.cancel.borrow().clone();
    let body = doc.body;
    // This optional probe reports an upper bound from the real window's
    // successful Present counter. No text, path or search term enters the log.
    s.owner.run(|| {
        spawn(async move {
            let presented = wait_for_editor_frame(body).await;
            if presented && flag.is_some_and(|c| !c.load(Ordering::Acquire)) {
                log::info!(
                    "TEXZI_FIND event=first_presentation_upper request={request} elapsed_us={}",
                    start.elapsed().as_micros()
                );
            }
        })
    });
}
pub(in crate::app_shell) fn source_marks(s: &Shell, doc: &Rc<Document>) -> Vec<EditorDecoration> {
    if !active(s, doc) || doc.displayed_rich() {
        return Vec::new();
    }
    let st = &s.find_panel;
    st.marks_rev.get();
    let Ok((snapshot, _)) = doc.source.search_snapshot() else {
        return Vec::new();
    };
    if st.version.get() != Some((snapshot.revision.0, false)) {
        return Vec::new();
    }
    let rope = snapshot.body.source();
    let current = st.current_mark.borrow();
    let ranges = st.visible.borrow();
    ranges
        .iter()
        .filter(|r| Some(*r) != current.as_ref())
        .chain(current.iter())
        .map(|r| {
            let mut d = EditorDecoration::default();
            let is_current = Some(r) == current.as_ref();
            let from = position(rope, r.start);
            let to = position(rope, r.end);
            d.range = EditorRange::new(from, to);
            if !r.is_empty() {
                d.class = Some(highlight_class(is_current, dark()).into());
            }
            if is_current || r.is_empty() {
                d.outline_color = Some(outline_color(is_current, dark()));
            }
            d
        })
        .collect()
}
