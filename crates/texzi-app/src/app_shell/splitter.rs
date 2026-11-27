use super::{Document, Shell, color, metric};
use nsh_core::{
    AccessibilityActionDef, AccessibilityRole, AccessibilityValue, Cursor, PointerEvents,
    element::layout_rtl,
    geometry::{Along, Axis, Edges},
    keymap::When,
    prelude::*,
};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

pub(super) const EQUAL_SHARE: f32 = 0.5;
const MIN_PANE: f32 = 128.0;
const KEY_STEP: f32 = 16.0;
const HIT_WIDTH: f32 = 9.0;

#[derive(Clone, Copy, PartialEq)]
struct Layout {
    available: f32,
    minimum: f32,
    first: f32,
}

impl Layout {
    fn new(extent: f32, gap: f32, share: f32) -> Self {
        let available = (extent - gap).max(0.0);
        let minimum = MIN_PANE.min(available * EQUAL_SHARE);
        let first = (available * share).clamp(minimum, available - minimum);
        Self {
            available,
            minimum,
            first,
        }
    }

    fn position(self, by: f32) -> f32 {
        (self.first + by).clamp(self.minimum, self.available - self.minimum)
    }

    fn share(self, position: f32) -> f32 {
        if self.available > 0.0 {
            position / self.available
        } else {
            EQUAL_SHARE
        }
    }
}

#[derive(Clone, Copy)]
struct Drag {
    layout: Layout,
    origin: f32,
    scale: Option<f32>,
    stale: bool,
}

struct PaneFocus {
    doc: Weak<Document>,
    second: bool,
    rich: bool,
}

impl PaneFocus {
    fn capture(shell: &Shell) -> Option<Self> {
        let doc = shell.current()?;
        Some(Self {
            doc: Rc::downgrade(&doc),
            second: shell.second_active.get_untracked(),
            rich: doc.displayed_rich(),
        })
    }

    fn matches(&self, shell: &Shell) -> bool {
        self.second == shell.second_active.get_untracked()
            && self.doc.upgrade().is_some_and(|target| {
                shell.current().is_some_and(|shown| {
                    Rc::ptr_eq(&target, &shown) && self.rich == shown.displayed_rich()
                })
            })
    }
}

struct Separator {
    shell: Rc<Shell>,
    axis: Axis,
    direction: f32,
    gap: f32,
    extent: Signal<f32>,
    focus: Signal<bool>,
    origin: RefCell<Option<PaneFocus>>,
    drag: Cell<Option<Drag>>,
    preview: Signal<Option<f32>>,
}

impl Separator {
    fn layout(&self) -> Layout {
        Layout::new(self.extent.get(), self.gap, self.shell.split_share.get())
    }

    fn allowed(&self) -> bool {
        !self.shell.modal.get()
            && !self.shell.settings_active()
            && self.shell.split.with(Option::is_some)
            && !self
                .shell
                .current()
                .is_some_and(|doc| doc.gate.busy() || doc.controls.open())
    }

    fn focused(&self) {
        *self.origin.borrow_mut() = PaneFocus::capture(&self.shell);
        self.focus.set(true);
    }

    fn blurred(&self) {
        self.focus.set(false);
        self.origin.borrow_mut().take();
        self.clear_drag();
    }

    fn restore_focus(&self) {
        if !self.focus.get_untracked() || !self.allowed() {
            return;
        }
        let group = self.origin.borrow().as_ref().and_then(|origin| {
            origin
                .matches(&self.shell)
                .then_some(if origin.second { 2 } else { 1 })
        });
        if let Some(group) = group
            && self.shell.window.state().is_some_and(|state| state.focused)
        {
            self.shell.focus_group(group);
        }
    }

    fn set_extent(&self, bounds: &Bounds<Points>) {
        let extent = bounds.size.along(self.axis).0;
        if self.extent.get_untracked() != extent {
            self.extent.set(extent);
            self.invalidate_drag();
        }
    }

    fn pointer(&self, event: &ResponderEvent) -> f32 {
        // The lifted point supplies the final coordinate even if its move was coalesced.
        event.page.along(self.axis).0
    }

    fn scale(&self) -> Option<f32> {
        self.shell.window.state().map(|state| state.scale_factor)
    }

    fn begin(&self, event: &ResponderEvent) {
        self.drag.set(Some(Drag {
            layout: self.layout(),
            origin: self.pointer(event),
            scale: self.scale(),
            stale: false,
        }));
    }

    fn drag_position(&self, event: &ResponderEvent) -> Option<(Layout, f32)> {
        let drag = self.drag.get()?;
        if drag.stale
            || !self.allowed()
            || self.scale() != drag.scale
            || self.layout() != drag.layout
        {
            return None;
        }
        let by = (self.pointer(event) - drag.origin) * self.direction;
        Some((drag.layout, drag.layout.position(by)))
    }

    fn move_to(&self, event: &ResponderEvent) {
        if let Some((layout, position)) = self.drag_position(event) {
            let shift = (position - layout.first) * self.direction;
            if self.preview.get_untracked() != Some(shift) {
                self.preview.set(Some(shift));
            }
        } else {
            self.invalidate_drag();
        }
    }

    fn hide_preview(&self) {
        if self.preview.get_untracked().is_some() {
            self.preview.set(None);
        }
    }

    fn invalidate_drag(&self) {
        if let Some(mut drag) = self.drag.get() {
            drag.stale = true;
            self.drag.set(Some(drag));
        }
        self.hide_preview();
    }

    fn clear_drag(&self) -> bool {
        let had_drag = self.drag.take().is_some();
        self.hide_preview();
        had_drag
    }

    fn commit(&self, layout: Layout, position: f32) {
        if position != layout.first {
            self.shell.split_share.set(layout.share(position));
        }
    }

    fn release(&self, event: &ResponderEvent) {
        let final_position = self.drag_position(event);
        if self.clear_drag() {
            if let Some((layout, position)) = final_position {
                self.commit(layout, position);
            }
            self.restore_focus();
        }
    }

    fn cancel(&self) {
        self.clear_drag();
        self.restore_focus();
    }

    fn step(&self, by: f32) {
        if self.allowed() && self.drag.get().is_none() {
            let layout = self.layout();
            self.commit(layout, layout.position(by));
        }
    }
}

fn handle(separator: Rc<Separator>) -> View {
    let axis = separator.axis;
    let gap = px(separator.gap);
    let slop = (px(HIT_WIDTH) - gap) * 0.5;
    let preview = separator.preview;
    let focus = separator.focus;
    let key_target = separator.clone();
    effect(move || {
        if !focus.get() {
            return;
        }
        let keys = match axis {
            Axis::Horizontal => ["left", "right"],
            Axis::Vertical => ["up", "down"],
        };
        for (key, by) in keys.into_iter().zip([-KEY_STEP, KEY_STEP]) {
            let target = key_target.clone();
            on_key_with(
                key,
                KeyOptions {
                    when: When::NotEditing,
                    for_focus: true,
                    repeat: true,
                    ..Default::default()
                },
                move |_| target.step(by * target.direction),
            );
        }
        let target = key_target.clone();
        on_key_with(
            "escape",
            KeyOptions {
                when: When::NotEditing,
                for_focus: true,
                ..Default::default()
            },
            move |_| target.cancel(),
        );
    });

    let grip = view()
        .pointer_events(PointerEvents::None)
        .rounded(px(2.0))
        .bg(|| color("ink-2"))
        .map(|v| match axis {
            Axis::Horizontal => v.w(px(2.0)).h(px(24.0)).max_h(relative(0.5)),
            Axis::Vertical => v.h(px(2.0)).w(px(24.0)).max_w(relative(0.5)),
        });
    let marker = view()
        // Keep preview updates out of the editor panes' flex layout.
        .pointer_events(PointerEvents::None)
        .absolute()
        .bg(|| color("ring"))
        .opacity(move || {
            if preview.get().is_some() {
                1.0_f32
            } else {
                0.0
            }
        })
        .map(|v| match axis {
            Axis::Horizontal => v
                .top(px(0.0))
                .bottom(px(0.0))
                .left((gap - px(2.0)) * 0.5)
                .w(px(2.0))
                .translate_x(move || DefiniteLength::from(px(preview.get().unwrap_or(0.0)))),
            Axis::Vertical => v
                .left(px(0.0))
                .right(px(0.0))
                .top((gap - px(2.0)) * 0.5)
                .h(px(2.0))
                .translate_y(move || DefiniteLength::from(px(preview.get().unwrap_or(0.0)))),
        });

    let label = match axis {
        Axis::Horizontal => "左右编辑组分隔线",
        Axis::Vertical => "上下编辑组分隔线",
    };
    let focused = separator.clone();
    let blurred = separator.clone();
    let claimed = separator.clone();
    let began = separator.clone();
    let moved = separator.clone();
    let released = separator.clone();
    let terminated = separator.clone();
    let adjusted = separator.clone();
    view()
        .relative()
        .z_index(1)
        .flex_none()
        .items_center()
        .justify_center()
        .map(|v| match axis {
            Axis::Horizontal => v.w(gap).cursor(Cursor::ColResize).hit_slop(Edges {
                left: slop,
                right: slop,
                ..Edges::default()
            }),
            Axis::Vertical => v.h(gap).cursor(Cursor::RowResize).hit_slop(Edges {
                top: slop,
                bottom: slop,
                ..Edges::default()
            }),
        })
        .app_class("hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring")
        .accessible(true)
        .role(AccessibilityRole::Adjustable)
        .label(label)
        .accessibility_value(move || {
            let layout = separator.layout();
            Some(AccessibilityValue {
                min: Some(layout.share(layout.minimum) * 100.0),
                max: Some(layout.share(layout.available - layout.minimum) * 100.0),
                now: Some(layout.share(layout.first) * 100.0),
                ..Default::default()
            })
        })
        .accessibility_actions(
            ["increment", "decrement"]
                .map(|name| AccessibilityActionDef {
                    name: name.into(),
                    label: None,
                })
                .to_vec(),
        )
        .on_accessibility_action(move |name: &str| match name {
            "increment" => adjusted.step(KEY_STEP),
            "decrement" => adjusted.step(-KEY_STEP),
            _ => {}
        })
        .focusable(true)
        .on_focus(move || focused.focused())
        .on_blur(move || blurred.blurred())
        .claim_touches(TouchClaim::on_start().ask(move |request: &ClaimRequest| {
            if claimed.allowed() {
                request.claim();
            }
        }))
        .keep_touches(true)
        .on_responder_grant(move |event: &ResponderEvent| began.begin(event))
        .on_responder_move(move |event: &ResponderEvent| moved.move_to(event))
        .on_responder_release(move |event: &ResponderEvent| released.release(event))
        .on_responder_terminate(move |_: &ResponderEvent| {
            if terminated.drag.get().is_some() {
                terminated.cancel();
            }
        })
        .child(grip)
        .child(marker)
}

pub(super) fn groups<E: IntoElement>(
    shell: Rc<Shell>,
    down: bool,
    primary: impl IntoElement,
    second: impl Fn() -> E + 'static,
) -> AnyElement {
    let axis = if down {
        Axis::Vertical
    } else {
        Axis::Horizontal
    };
    let gap = metric("splitGap");
    let split = shell.split;
    let direction = if !down && layout_rtl() { -1.0 } else { 1.0 };
    let separator = Rc::new(Separator {
        shell,
        axis,
        gap,
        direction,
        extent: signal(0.0),
        focus: signal(false),
        origin: RefCell::new(None),
        drag: Cell::new(None),
        preview: signal(None),
    });
    let first_share = {
        let separator = separator.clone();
        computed(move || {
            if split.with(Option::is_some) {
                let layout = separator.layout();
                layout.share(layout.first)
            } else {
                1.0
            }
        })
    };
    let measured = separator.clone();
    let pane = || view().flex_1().min_w(px(0.0)).min_h(px(0.0));
    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .px(px(gap))
        .pb(px(gap))
        .child(
            view()
                .when(!down, |v| v.flex_row())
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .on_layout(move |bounds: &Bounds<Points>| measured.set_extent(bounds))
                .child(pane().flex_grow(move || first_share.get()).child(primary))
                .child(when(
                    move || split.with(Option::is_some),
                    move || handle(separator.clone()),
                ))
                .child(when(
                    move || split.with(Option::is_some),
                    move || {
                        pane()
                            .flex_grow(move || 1.0 - first_share.get())
                            .child(second())
                    },
                )),
        )
        .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_core::{
        ApproxTextEngine, Core, MountItem, NodeId, PayloadValue, PointerType, ROOT_ID, TouchEvent,
        TouchPhase, TouchPoint,
        element::{JsEvent, Root},
        geometry::{point, size},
        keymap::{Keymap, Keystroke},
        platform::Os,
    };
    use std::sync::Arc;

    fn fixture(down: bool) -> impl IntoElement {
        let shell = Shell::new();
        shell.split_editor(down);
        view().size_full().child(groups(
            shell,
            down,
            view().flex_1().label("first fixture pane"),
            || view().flex_1().label("second fixture pane"),
        ))
    }

    #[component]
    fn Horizontal() -> impl IntoElement {
        fixture(false)
    }

    #[component]
    fn Vertical() -> impl IntoElement {
        fixture(true)
    }

    #[derive(Default)]
    struct Mounted {
        divider: NodeId,
        hit_slop: Edges<Points>,
        panes: [NodeId; 2],
        keys: Option<Arc<Keymap>>,
    }

    impl Mounted {
        fn settle(&mut self, core: &mut Core<ApproxTextEngine>) {
            for _ in 0..6 {
                core.run_component_tasks();
                if let Some(batch) = core.commit() {
                    for item in batch.items {
                        match item {
                            MountItem::Keymap(keys) => self.keys = Some(keys),
                            MountItem::ViewProps { id, props } => {
                                match props.accessibility.label.as_deref() {
                                    Some("左右编辑组分隔线" | "上下编辑组分隔线") =>
                                    {
                                        self.divider = id;
                                        self.hit_slop = props.hit_slop;
                                        assert!(props.focus_stop(false));
                                    }
                                    Some("first fixture pane") => self.panes[0] = id,
                                    Some("second fixture pane") => self.panes[1] = id,
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                }
                let mut events = Vec::new();
                core.take_events(&mut events);
                for event in events {
                    match event {
                        JsEvent::LayoutListener {
                            host,
                            id,
                            listener,
                            frame,
                        } => {
                            core.component_layout(host, id, &listener, frame);
                        }
                        JsEvent::Event(id, EventKind::Layout, payload) => {
                            core.component_event(id, EventKind::Layout, &payload);
                        }
                        _ => {}
                    }
                }
            }
        }

        fn pointer(
            &mut self,
            core: &mut Core<ApproxTextEngine>,
            phase: TouchPhase,
            at: Point<Points>,
        ) {
            let touch = TouchEvent {
                phase,
                timestamp: match phase {
                    TouchPhase::Start => 0.0,
                    TouchPhase::Move => 16.0,
                    _ => 32.0,
                },
                points: vec![TouchPoint {
                    id: 1,
                    target: self.divider,
                    path: vec![self.divider],
                    page: at,
                    location: at,
                    changed: true,
                    pointer_type: PointerType::Mouse,
                    pressure: 0.5,
                    tilt_x: 0.0,
                    tilt_y: 0.0,
                }],
                modifiers: Default::default(),
                activated: None,
            };
            core.component_event(
                ROOT_ID,
                EventKind::Touch,
                &EventPayload::Touch(Box::new(touch)),
            );
            self.settle(core);
        }

        fn press(&mut self, core: &mut Core<ApproxTextEngine>, key: &str) {
            let stroke = Keystroke::parse_with(key, false).unwrap();
            let command = self
                .keys
                .as_ref()
                .unwrap()
                .find(stroke, false, false)
                .fired()
                .unwrap();
            assert!(core.component_event(
                ROOT_ID,
                EventKind::KeyCommand,
                &EventPayload::Value(PayloadValue::Number(command as f64)),
            ));
            self.settle(core);
        }
    }

    fn exercise(root: Root, axis: Axis, arrow: &str) {
        let mut core = Core::new(ApproxTextEngine, size(px(800.0), px(600.0)));
        let mut environment = Environment::default();
        environment.os = Os::Windows;
        core.set_environment(environment);
        core.mount_root(&root);
        let mut mounted = Mounted::default();
        mounted.settle(&mut core);
        let panes = mounted.panes;
        let sizes = |core: &mut Core<ApproxTextEngine>| {
            panes.map(|id| core.measure(id).unwrap().frame.size.along(axis).0)
        };
        let seam = core.measure(mounted.divider).unwrap();
        let gap = seam.frame.size.along(axis).0;
        assert_eq!(gap, 5.0);
        let hit = match axis {
            Axis::Horizontal => mounted.hit_slop.left + mounted.hit_slop.right,
            Axis::Vertical => mounted.hit_slop.top + mounted.hit_slop.bottom,
        };
        assert_eq!(gap + hit.0, 9.0);
        let start = seam.page + point(px(2.0), px(2.0));
        let delta = |value| match axis {
            Axis::Horizontal => point(px(value), px(0.0)),
            Axis::Vertical => point(px(0.0), px(value)),
        };
        let before = sizes(&mut core);
        assert!((before[0] - before[1]).abs() <= 1.0);

        mounted.pointer(&mut core, TouchPhase::Start, start);
        mounted.pointer(&mut core, TouchPhase::Move, start + delta(72.0));
        assert_eq!(sizes(&mut core), before);
        mounted.pointer(&mut core, TouchPhase::Cancel, start + delta(72.0));
        assert_eq!(sizes(&mut core), before);

        mounted.pointer(&mut core, TouchPhase::Start, start);
        mounted.pointer(&mut core, TouchPhase::Move, start + delta(24.0));
        assert_eq!(sizes(&mut core), before);
        mounted.pointer(&mut core, TouchPhase::End, start + delta(56.0));
        let after = sizes(&mut core);
        assert!((after[0] - before[0] - 56.0).abs() <= 1.0);
        assert!((before[1] - after[1] - 56.0).abs() <= 1.0);

        assert!(core.component_event(mounted.divider, EventKind::Focus, &EventPayload::None));
        mounted.settle(&mut core);
        mounted.press(&mut core, arrow);
        let keyboard = sizes(&mut core);
        assert!((keyboard[0] - after[0] - KEY_STEP).abs() <= 1.0);
        assert!((after[1] - keyboard[1] - KEY_STEP).abs() <= 1.0);
        assert_eq!(mounted.panes, panes);

        let start = core.measure(mounted.divider).unwrap().page + point(px(2.0), px(2.0));
        mounted.pointer(&mut core, TouchPhase::Start, start);
        mounted.pointer(&mut core, TouchPhase::Move, start + delta(48.0));
        mounted.press(&mut core, "escape");
        mounted.pointer(&mut core, TouchPhase::End, start + delta(48.0));
        assert_eq!(sizes(&mut core), keyboard);

        mounted.pointer(&mut core, TouchPhase::Start, start);
        mounted.pointer(&mut core, TouchPhase::Move, start + delta(48.0));
        let narrow = match axis {
            Axis::Horizontal => size(px(245.0), px(600.0)),
            Axis::Vertical => size(px(800.0), px(245.0)),
        };
        core.set_viewport(narrow);
        mounted.settle(&mut core);
        let resized = sizes(&mut core);
        assert!((resized[0] - resized[1]).abs() <= 1.0);
        core.set_viewport(size(px(800.0), px(600.0)));
        mounted.settle(&mut core);
        assert_eq!(sizes(&mut core), keyboard);
        mounted.pointer(&mut core, TouchPhase::End, start + delta(48.0));
        assert_eq!(sizes(&mut core), keyboard);
    }

    #[test]
    fn drag_commits_only_the_release_on_both_axes() {
        exercise(Root::new::<Horizontal>(|| ()), Axis::Horizontal, "right");
        exercise(Root::new::<Vertical>(|| ()), Axis::Vertical, "down");
    }

    #[test]
    fn pane_minimums_clamp_and_tight_windows_keep_both_panes() {
        let layout = Layout::new(805.0, 5.0, EQUAL_SHARE);
        assert_eq!(layout.position(-1_000.0), MIN_PANE);
        assert_eq!(layout.position(1_000.0), 800.0 - MIN_PANE);
        let tight = Layout::new(245.0, 5.0, 0.75);
        assert_eq!(tight.first, 120.0);
        assert_eq!(tight.position(-16.0), 120.0);
        assert_eq!(tight.position(16.0), 120.0);
    }
}
