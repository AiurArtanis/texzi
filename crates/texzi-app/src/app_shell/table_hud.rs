use super::{CONTROL_STATES, Document, Shell, border_width, color, font_size};
use crate::table_controls::{self, ACTIONS, TableAction};
use nsh_base::rich_editor::view::{RichEditorApi, TableAnchor, TableAnchorGeometry};
use nsh_core::{
    AccessibilityRole, KeyCapture, KeyEvent, PointerEvents,
    element::{layout_rtl, next_frame},
    keymap::Keystroke,
    prelude::*,
};
use std::{cell::Cell, rc::Rc};

const GAP: f32 = 8.0;
const PADDING: f32 = 6.0;
const ROW: f32 = 30.0;
const GRID_GAP: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Placement {
    x: f32,
    y: f32,
    width: f32,
    columns: usize,
}

fn place(cell: Bounds<Points>, viewport: Size<Points>, rtl: bool) -> Option<Placement> {
    let width = (viewport.width.0 - 2.0 * GAP).min(416.0);
    if width < 184.0 {
        return None;
    }
    let columns = if width >= 376.0 {
        4
    } else if width >= 272.0 {
        3
    } else {
        2
    };
    let rows = ACTIONS.len().div_ceil(columns) as f32;
    // The frame, its fixed-height button rows and its status/cancel footer.
    let height = 2.0 * PADDING + (rows + 1.0) * (ROW + GRID_GAP) + 4.0;
    let above = cell.origin.y.0 - GAP - height;
    let below = cell.bottom().0 + GAP;
    let y = if above >= GAP {
        above
    } else if below + height <= viewport.height.0 - GAP {
        below
    } else {
        return None;
    };
    let x = if rtl {
        cell.right().0 - width
    } else {
        cell.origin.x.0
    };
    Some(Placement {
        x: x.clamp(GAP, viewport.width.0 - width - GAP),
        y,
        width,
        columns,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Closed,
    Anchored,
    Dialog,
}

#[derive(Clone, Copy, PartialEq)]
enum AnchorState {
    Stale,
    Composing,
    Missing,
    Placed(Placement),
}

pub(super) struct TableHud {
    owner: Owner,
    shell: Rc<Shell>,
    doc: Rc<Document>,
    second: bool,
    surface: Signal<Surface>,
    placement: Signal<Option<Placement>>,
    parked: Signal<bool>,
    viewport_focused: Signal<bool>,
    tools: ElementRef,
    first: ElementRef,
    viewport: ElementRef,
    tools_focused: Rc<Cell<bool>>,
    open_focus: Cell<bool>,
    closing: Cell<Option<TableAnchor>>,
    surface_epoch: Cell<u64>,
}

impl TableHud {
    pub(super) fn new(shell: Rc<Shell>, doc: Rc<Document>, second: bool) -> Rc<Self> {
        let hud = Rc::new(Self {
            owner: Owner::current(),
            shell,
            doc,
            second,
            surface: signal(Surface::Closed),
            placement: signal(None),
            parked: signal(false),
            viewport_focused: signal(false),
            tools: element_ref(),
            first: element_ref(),
            viewport: element_ref(),
            tools_focused: Rc::default(),
            open_focus: Cell::new(false),
            closing: Cell::new(None),
            surface_epoch: Cell::new(0),
        });
        let controls = hud.doc.controls.table.clone();
        cleanup(move || controls.cancel());
        let focus = hud.tools_focused.clone();
        hud.tools.on_focus_in(move || focus.set(true));
        let focus = hud.tools_focused.clone();
        hud.tools.on_focus_out(move || focus.set(false));

        let state = hud.clone();
        effect(move || {
            let Some(anchor) = state.doc.controls.table.anchor() else {
                state.set_surface(Surface::Closed);
                return;
            };
            match state.anchor_state(anchor) {
                AnchorState::Stale | AnchorState::Composing => state.close(),
                AnchorState::Placed(placement) => match state.surface.get_untracked() {
                    Surface::Closed => {
                        state.open_focus.set(true);
                        state.placement.set(Some(placement));
                        state.set_surface(Surface::Anchored);
                    }
                    Surface::Anchored => {
                        if state.placement.get_untracked() != Some(placement) {
                            state.placement.set(Some(placement));
                        }
                    }
                    Surface::Dialog => {}
                },
                AnchorState::Missing => match state.surface.get_untracked() {
                    Surface::Closed => state.set_surface(Surface::Dialog),
                    Surface::Anchored => state.settle_scroll_out(anchor),
                    Surface::Dialog => {}
                },
            }
        });
        hud
    }

    fn current(&self) -> bool {
        self.shell.second_active.get() == self.second
            && self.doc.displayed_rich()
            && self
                .shell
                .current()
                .is_some_and(|shown| Rc::ptr_eq(&shown, &self.doc))
    }

    fn api(&self) -> Option<Rc<RichEditorApi>> {
        self.doc.rich_reference.api::<RichEditorApi>()
    }

    fn anchor_state(&self, anchor: TableAnchor) -> AnchorState {
        if !self.current() {
            return AnchorState::Stale;
        }
        if self.doc.gate.busy() {
            return AnchorState::Composing;
        }
        match self.api().map(|api| api.table_anchor_geometry(anchor)) {
            Some(TableAnchorGeometry::Stale) => AnchorState::Stale,
            Some(TableAnchorGeometry::Composing) => AnchorState::Composing,
            Some(TableAnchorGeometry::Visible { cell, viewport }) => {
                place(cell, viewport, layout_rtl())
                    .map_or(AnchorState::Missing, AnchorState::Placed)
            }
            _ => AnchorState::Missing,
        }
    }

    fn set_surface(&self, surface: Surface) {
        if self.surface.get_untracked() != surface {
            self.surface_epoch
                .set(self.surface_epoch.get().wrapping_add(1));
            self.closing.set(None);
            self.surface.set(surface);
        }
    }

    fn close(&self) {
        self.set_surface(Surface::Closed);
        self.doc.controls.table.cancel();
    }

    fn settle_scroll_out(self: &Rc<Self>, anchor: TableAnchor) {
        if self.closing.replace(Some(anchor)) == Some(anchor) {
            return;
        }
        let hud = self.clone();
        let epoch = self.surface_epoch.get();
        self.owner.run(|| {
            spawn(async move {
                // Let the platform's current focus report settle before deciding
                // whether the disappearing tools owned the keyboard.
                next_frame().await;
                if hud.surface_epoch.get() != epoch || hud.closing.get() != Some(anchor) {
                    return;
                }
                hud.closing.set(None);
                if hud.doc.controls.table.anchor() != Some(anchor)
                    || hud.surface.get_untracked() != Surface::Anchored
                    || hud.anchor_state(anchor) != AnchorState::Missing
                {
                    return;
                }
                if hud.tools_focused.get() {
                    hud.park_viewport();
                }
                hud.close();
            })
        });
    }

    fn park_viewport(&self) {
        self.parked.set(true);
        self.viewport.focus_back(self.tools);
        let parked = self.parked;
        let focused = self.viewport_focused;
        self.owner.run(|| {
            spawn(async move {
                // A competing focus request can win in the closing commit.
                next_frame().await;
                next_frame().await;
                if !focused.get_untracked() {
                    parked.set(false);
                }
            })
        });
    }

    fn restore_visible_input(&self) {
        if self.tools_focused.get()
            && self.current()
            && let Some(api) = self.api()
        {
            api.restore_table_input(self.tools);
        }
    }

    fn cancel_from_tools(&self) {
        let Some(anchor) = self.doc.controls.table.anchor() else {
            return;
        };
        match self.anchor_state(anchor) {
            AnchorState::Placed(_) => self.restore_visible_input(),
            AnchorState::Missing if self.tools_focused.get() => self.park_viewport(),
            _ => {}
        }
        self.close();
    }

    pub(super) fn escape_from_body(&self) -> bool {
        if self.surface.get_untracked() != Surface::Anchored || !self.current() {
            return false;
        }
        self.close();
        true
    }

    fn press(&self, action: TableAction) {
        let controls = &self.doc.controls.table;
        if !controls
            .anchor()
            .is_some_and(|anchor| matches!(self.anchor_state(anchor), AnchorState::Placed(_)))
        {
            return;
        }
        let result = controls.apply(action);
        if result.is_ok() {
            self.restore_visible_input();
        }
        controls.report(result);
    }

    fn fallback(self: &Rc<Self>, focus: Rc<dyn Fn()>) -> impl IntoElement + use<> {
        let surface = self.surface;
        let hud = self.clone();
        table_controls::dialog(
            self.doc.controls.table.clone(),
            move || surface.get() == Surface::Dialog,
            Rc::new(move || {
                if hud.current() {
                    focus();
                }
            }),
        )
    }

    fn navigation(self: &Rc<Self>) -> View {
        let parked = self.parked;
        let focused = self.viewport_focused;
        let hud = self.clone();
        view()
            .absolute()
            .inset(px(0.0))
            .element_ref(&self.viewport)
            .test_id("texzi.table-tools.viewport-focus")
            .focusable(parked)
            .optional_label(move || {
                parked
                    .get()
                    .then(|| SharedString::from("当前编辑组正文视口"))
            })
            .pointer_events(PointerEvents::BoxNone)
            .border(move || px(if focused.get() { 2.0 } else { 0.0 }))
            .border_color(move || color("ring"))
            .capture_keys(capture(&[
                "up", "down", "pageup", "pagedown", "tab", "escape",
            ]))
            .on_focus(move |_: &()| focused.set(true))
            .on_blur(move |_: &()| {
                focused.set(false);
                parked.set(false);
            })
            .on_key_down(move |event: &KeyEvent| {
                if !event.default_prevented || !focused.get_untracked() || !hud.current() {
                    return;
                }
                let Some(api) = hud.api() else {
                    return;
                };
                match event.key.as_ref() {
                    "ArrowUp" => api.scroll_table_viewport(-36.0, false),
                    "ArrowDown" => api.scroll_table_viewport(36.0, false),
                    "PageUp" => api.scroll_table_viewport(-1.0, true),
                    "PageDown" => api.scroll_table_viewport(1.0, true),
                    "Tab" => api.enter_visible_table_body(),
                    _ => {}
                }
            })
    }

    pub(super) fn layer(self: &Rc<Self>, focus: Rc<dyn Fn()>) -> View {
        let hud = self.clone();
        let surface = self.surface;
        view()
            .absolute()
            .inset(px(0.0))
            .pointer_events(PointerEvents::BoxNone)
            .child(self.navigation())
            .child(when(
                move || surface.get() == Surface::Anchored,
                move || hud.panel(),
            ))
            .child(self.fallback(focus))
    }

    fn panel(self: &Rc<Self>) -> View {
        let position = self.placement;
        let tracks = computed(move || {
            let columns = position.get().map_or(3, |placement| placement.columns);
            SharedString::from(format!("repeat({columns}, minmax(0, 1fr))"))
        });
        let mut buttons = view()
            .grid()
            .gap(px(GRID_GAP))
            .grid_template_columns(tracks);
        for (index, (label, short, action)) in ACTIONS.into_iter().enumerate() {
            let hud = self.clone();
            let escape = self.clone();
            let first = self.clone();
            buttons = buttons.child(
                tool_button(label, short)
                    .when(index == 0, move |button| {
                        button
                            .test_id("texzi.table-tools.first")
                            .element_ref(&first.first)
                            .on_layout(move |_: &Bounds<Points>| {
                                if first.open_focus.replace(false)
                                    && first.surface.get_untracked() == Surface::Anchored
                                    && first.current()
                                {
                                    first.first.focus();
                                }
                            })
                    })
                    .on_key_down(move |event: &KeyEvent| {
                        if event.default_prevented && event.key.as_ref() == "Escape" {
                            escape.cancel_from_tools();
                        }
                    })
                    .on_press(move |_: &()| hud.press(action)),
            );
        }
        let status = self.doc.controls.table.status();
        let cancel = self.clone();
        let escape = self.clone();
        view()
            .absolute()
            .left(move || px(position.get().map_or(0.0, |p| p.x)))
            .top(move || px(position.get().map_or(0.0, |p| p.y)))
            .w(move || px(position.get().map_or(0.0, |p| p.width)))
            .element_ref(&self.tools)
            .role(AccessibilityRole::Toolbar)
            .label("表格操作")
            .p(px(PADDING))
            .gap(px(GRID_GAP))
            .rounded(px(8.0))
            .border(border_width)
            .border_color(move || color("chrome-edge"))
            .bg(move || color("paper"))
            .text_color(move || color("ink-1"))
            .child(buttons)
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        text(move || {
                            let message = status.get();
                            if message.is_empty() {
                                "当前单元格".into()
                            } else {
                                message
                            }
                        })
                        .app_class("truncate")
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(px(font_size("meta"))),
                    )
                    .child(
                        tool_button("关闭表格操作", "关闭")
                            .test_id("texzi.table-tools.close")
                            .w(px(60.0))
                            .on_key_down(move |event: &KeyEvent| {
                                if event.default_prevented && event.key.as_ref() == "Escape" {
                                    escape.cancel_from_tools();
                                }
                            })
                            .on_press(move |_: &()| cancel.cancel_from_tools()),
                    ),
            )
    }
}

fn capture(keys: &[&str]) -> KeyCapture {
    KeyCapture::keys(
        keys.iter()
            .map(|key| Keystroke::parse_with(key, false).expect("valid table tool shortcut")),
    )
}

fn tool_button(label: &'static str, short: &'static str) -> View {
    view()
        .role(AccessibilityRole::Button)
        .focusable(true)
        .label(label)
        .capture_keys(capture(&["escape"]))
        .h(px(ROW))
        .min_w(px(0.0))
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .app_class(CONTROL_STATES)
        .child(text(short).text_size(px(font_size("label"))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_core::geometry::point;

    fn cell(x: f32, y: f32) -> Bounds<Points> {
        Bounds::new(point(px(x), px(y)), Size::new(px(84.0), px(32.0)))
    }

    #[test]
    fn anchored_tools_flip_without_covering_the_cell_and_stay_in_their_group() {
        let wide = Size::new(px(820.0), px(430.0));
        let above = place(cell(112.0, 280.0), wide, false).unwrap();
        assert_eq!(above.x, 112.0);
        assert_eq!(above.columns, 4);
        assert!(above.y < 280.0);
        let below = place(cell(112.0, 5.0), wide, false).unwrap();
        assert_eq!(below.y, 45.0);
        let right = place(cell(780.0, 5.0), wide, false).unwrap();
        assert_eq!(right.x + right.width, 812.0);
        let rtl = place(cell(596.0, 5.0), wide, true).unwrap();
        assert_eq!(rtl.x + rtl.width, 680.0);
        let narrow = Size::new(px(335.0), px(430.0));
        let compact = place(cell(275.0, 5.0), narrow, false).unwrap();
        assert_eq!(compact.columns, 3);
        assert_eq!(compact.x + compact.width, 327.0);
        assert!(place(cell(80.0, 50.0), Size::new(px(335.0), px(120.0)), false).is_none());
    }
}

#[cfg(test)]
mod mounted_tests {
    use super::*;
    use crate::app_shell::{editor, settings};
    use nsh_base::rich_editor::state::TextSelection;
    use nsh_core::{
        ApproxTextEngine, Command, Core, EventKind, EventPayload, MountItem, NodeId, PayloadValue,
        ViewKind,
        element::{Environment, JsEvent, Root},
        geometry::point,
        platform::Os,
        tailwind::state::FOCUSED,
    };
    use std::{
        cell::RefCell,
        collections::{HashMap, HashSet},
        path::PathBuf,
        time::{Duration, Instant},
    };

    struct FixturePaths {
        markdown: PathBuf,
        neighbor: PathBuf,
        observed: Rc<RefCell<(Option<u64>, bool, String)>>,
    }

    thread_local! {
        static PATHS: RefCell<Option<FixturePaths>> = const { RefCell::new(None) };
    }

    #[component]
    fn Fixture() -> impl IntoElement {
        let paths = PATHS.with(|paths| paths.borrow_mut().take().unwrap());
        let shell = Shell::new();
        shell.open_canonical_file(paths.markdown).unwrap();
        assert!(
            shell
                .settings
                .apply(settings::AUTO_SAVE, None, Some("onFocusChange".into()))
        );
        let doc = shell.current().unwrap();
        assert!(doc.displayed_rich());
        let probe = shell.clone();
        effect(move || {
            *paths.observed.borrow_mut() = (
                probe.current().map(|document| document.id),
                probe.settings_active(),
                probe.notice.get(),
            );
        });
        let session = doc.gate.session();
        let state = session.state();
        let at = table_controls::cell_position(state.doc(), 0, 1, 1).unwrap();
        let mut select = state.tr();
        select.set_selection(TextSelection::create(state.doc(), at, None));
        session.dispatch(select).unwrap();

        let mut insert = session.state().tr();
        insert.insert_text("Z", Some(at), Some(at));
        session.dispatch(insert).unwrap();
        assert!(doc.source.is_dirty().unwrap());
        let begin = doc.controls.table.clone();
        let settings = shell.clone();
        let next = shell.clone();
        view()
            .size_full()
            .child(editor(shell, doc, false))
            .child(
                view()
                    .test_id("table-hud-test-open")
                    .focusable(true)
                    .h(px(28.0))
                    .on_press(move |_: &()| {
                        begin.begin().unwrap();
                    }),
            )
            .child(
                view()
                    .test_id("table-hud-test-settings")
                    .focusable(true)
                    .h(px(28.0))
                    .on_press(move |_: &()| {
                        settings.show_settings();
                    }),
            )
            .child(
                view()
                    .test_id("table-hud-test-neighbor")
                    .focusable(true)
                    .h(px(28.0))
                    .on_press(move |_: &()| {
                        next.open_canonical_file(paths.neighbor.clone()).unwrap();
                    }),
            )
    }

    #[derive(Default)]
    struct Mounted {
        inputs: HashSet<NodeId>,
        focus_scope: Option<NodeId>,
        open: Option<NodeId>,
        settings: Option<NodeId>,
        neighbor: Option<NodeId>,
        first_tool: Option<NodeId>,
        close_tool: Option<NodeId>,
        viewport_focus: Option<NodeId>,
        table_scroll: Option<NodeId>,
        scrolls: HashSet<NodeId>,
        parents: HashMap<NodeId, NodeId>,
        focus_commands: Vec<NodeId>,
    }

    impl Mounted {
        fn sink(&self) -> NodeId {
            let scope = self.focus_scope.expect("rich native focus scope");
            let mut in_scope = self.inputs.iter().copied().filter(|&input| {
                let mut node = input;
                while let Some(&parent) = self.parents.get(&node) {
                    if parent == scope {
                        return true;
                    }
                    node = parent;
                }
                false
            });
            let sink = in_scope
                .next()
                .expect("input inside rich native focus scope");
            assert!(
                in_scope.next().is_none(),
                "the rich body must own one native input"
            );
            sink
        }

        fn pump(&mut self, core: &mut Core<ApproxTextEngine>) {
            for _ in 0..8 {
                core.run_component_tasks();
                core.run_frame(Instant::now());
                if let Some(batch) = core.commit() {
                    for item in batch.items {
                        match item {
                            MountItem::Create {
                                id,
                                kind: ViewKind::Scroll,
                            } => {
                                self.scrolls.insert(id);
                            }
                            MountItem::Create {
                                id,
                                kind: ViewKind::Input,
                            } => {
                                self.inputs.insert(id);
                            }
                            MountItem::Insert { parent, child, .. } => {
                                self.parents.insert(child, parent);
                            }
                            MountItem::Delete { id } if Some(id) == self.first_tool => {
                                self.first_tool = None;
                            }
                            MountItem::Command {
                                id,
                                command: Command::Focus,
                            } => {
                                self.focus_commands.push(id);
                            }
                            MountItem::ViewProps { id, props } => match props.test_id.as_deref() {
                                Some("table-hud-test-open") => self.open = Some(id),
                                Some("table-hud-test-settings") => self.settings = Some(id),
                                Some("table-hud-test-neighbor") => self.neighbor = Some(id),
                                Some("texzi.table-tools.first") => self.first_tool = Some(id),
                                Some("texzi.table-tools.close") => self.close_tool = Some(id),
                                Some("texzi.table-tools.viewport-focus") => {
                                    self.viewport_focus = Some(id)
                                }
                                Some("rich-editor-table-scroll") => self.table_scroll = Some(id),
                                Some("texzi.rich.focus-scope") => self.focus_scope = Some(id),
                                _ => {}
                            },
                            _ => {}
                        }
                    }
                }
                let mut events = Vec::new();
                core.take_events(&mut events);
                for event in events {
                    match event {
                        JsEvent::Event(id, kind, payload) => {
                            core.component_event(id, kind, &payload);
                        }
                        JsEvent::LayoutListener {
                            host,
                            id,
                            listener,
                            frame,
                        } => {
                            core.component_layout(host, id, &listener, frame);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn scroll_fixture(mounted: &mut Mounted, core: &mut Core<ApproxTextEngine>, y: f32) {
        let mut parent = mounted.table_scroll.unwrap();
        loop {
            parent = mounted.parents[&parent];
            if mounted.scrolls.contains(&parent) {
                break;
            }
        }
        let (_, content, viewport, _) = core.scroll_listener(parent).unwrap();
        assert!(content.height.0 >= viewport.height.0 + y && viewport.height.0 > 0.0);
        let payload = EventPayload::Map(vec![
            ("offset", PayloadValue::Point(point(px(0.0), px(y)))),
            ("contentSize", PayloadValue::Size(content)),
            ("size", PayloadValue::Size(viewport)),
        ]);
        assert!(core.component_event(parent, EventKind::Scroll, &payload));
        mounted.pump(core);
    }

    #[test]
    fn mounted_table_hud_focus_is_internal_and_leaving_the_editor_still_saves() {
        let original = "| H | V |\n| --- | --- |\n| A | B |\n\n".to_owned()
            + &"Verified visible body after the table.\n\n".repeat(40);
        for (use_settings, scroll) in [(false, false), (true, false), (true, true)] {
            let dir = std::env::temp_dir().join(format!(
                "texzi-table-hud-focus-{}-{use_settings}-{scroll}",
                std::process::id()
            ));
            std::fs::create_dir(&dir).unwrap();
            let markdown = dir.join("original.md");
            let neighbor = dir.join("neighbor.txt");
            std::fs::write(&markdown, &original).unwrap();
            std::fs::write(&neighbor, "sentinel").unwrap();
            let observed = Rc::new(RefCell::new((None, false, String::new())));
            PATHS.with(|paths| {
                *paths.borrow_mut() = Some(FixturePaths {
                    markdown: markdown.clone(),
                    neighbor: neighbor.clone(),
                    observed: observed.clone(),
                });
            });

            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let mut core = Core::new(ApproxTextEngine, Size::new(px(800.0), px(600.0)));
            core.set_task_runtime(runtime.handle().clone());
            let mut environment = Environment::default();
            environment.os = Os::Windows;
            core.set_environment(environment);
            core.mount_root(&Root::new::<Fixture>(|| ()));
            let mut mounted = Mounted::default();
            mounted.pump(&mut core);
            let sink = mounted.sink();
            core.set_states(sink, FOCUSED);
            assert!(core.component_event(sink, EventKind::Focus, &EventPayload::None));
            mounted.pump(&mut core);

            let open = mounted.open.unwrap();
            assert!(core.component_event(open, EventKind::Press, &EventPayload::None));
            mounted.pump(&mut core);
            let first = mounted.first_tool.expect("mounted, cell-anchored tool");
            assert!(mounted.focus_commands.contains(&first));
            core.set_states(sink, 0);
            core.set_states(first, FOCUSED);
            assert!(core.component_event(sink, EventKind::Blur, &EventPayload::None));
            mounted.pump(&mut core);
            let close = mounted.close_tool.unwrap();
            core.set_states(first, 0);
            core.set_states(close, FOCUSED);
            let until = Instant::now() + Duration::from_millis(200);
            while Instant::now() < until {
                mounted.pump(&mut core);
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(std::fs::read_to_string(&markdown).unwrap(), original);

            let last_internal = if scroll {
                mounted.focus_commands.clear();
                scroll_fixture(&mut mounted, &mut core, 365.0);
                let proxy = mounted.viewport_focus.unwrap();
                assert!(mounted.first_tool.is_none());
                assert!(mounted.focus_commands.contains(&proxy));
                core.set_states(proxy, FOCUSED);
                scroll_fixture(&mut mounted, &mut core, 0.0);
                assert!(
                    mounted.first_tool.is_none(),
                    "returning to the cell must not reopen the tools"
                );
                assert_eq!(std::fs::read_to_string(&markdown).unwrap(), original);
                proxy
            } else if use_settings {
                let mut escape = KeyEvent::default();
                escape.key = "Escape".into();
                escape.default_prevented = true;
                mounted.focus_commands.clear();
                assert!(core.component_event(
                    close,
                    EventKind::KeyDown,
                    &EventPayload::Key(escape)
                ));
                mounted.pump(&mut core);
                assert!(mounted.focus_commands.contains(&sink));
                core.set_states(sink, FOCUSED);
                assert!(core.component_event(sink, EventKind::Focus, &EventPayload::None));
                mounted.pump(&mut core);
                sink
            } else {
                close
            };
            let external = if use_settings {
                mounted.settings
            } else {
                mounted.neighbor
            }
            .unwrap();
            mounted.focus_commands.clear();
            core.set_states(last_internal, 0);
            core.set_states(external, FOCUSED);
            assert!(core.component_event(external, EventKind::Press, &EventPayload::None));
            mounted.pump(&mut core);
            {
                let (active, settings, _) = &*observed.borrow();
                assert_eq!(*settings, use_settings);
                if use_settings {
                    assert_eq!(*active, None);
                } else {
                    assert!(active.is_some_and(|active| active > 1));
                }
            }
            let until = Instant::now() + Duration::from_secs(5);
            let expected = original.replace("| B |", "| ZB |");
            loop {
                mounted.pump(&mut core);
                let disk = std::fs::read_to_string(&markdown).unwrap();
                if disk == expected {
                    break;
                }
                assert!(
                    Instant::now() < until,
                    "editor exit did not save: {:?}, file starts {:?}",
                    observed.borrow(),
                    disk.get(..36)
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(std::fs::read_to_string(&neighbor).unwrap(), "sentinel");
            assert!(!mounted.focus_commands.contains(&sink));
            assert!(
                !mounted
                    .focus_commands
                    .contains(&mounted.viewport_focus.unwrap())
            );
            eprintln!(
                "table HUD: settings={use_settings}, scroll_out={scroll}, internal unchanged, exit saved exact bytes, neighbor unchanged"
            );
            drop(core);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
