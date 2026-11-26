//! Hand-off of the native scrollbar to a virtualized rich document.
//! Normal scrolling commands keep their own Core accounting. A thumb
//! offset can only come from the current grab and its mounted receiver.

use std::rc::Rc;

use nsh_core::{EventKind, EventPayload, NodeId, Points, scroll::ScrollHost};
use serde_json::{Value, json};
use windows::Win32::Foundation::HWND;

use super::Host;
use crate::{app_window, surface::Surface, view::NshView};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Grab {
    view: NodeId,
    receiver: NodeId,
    gesture: u32,
    window: HWND,
    last_ticket: u64,
}

fn receiver(view: &NshView) -> Option<NodeId> {
    view.with_children(|children| {
        children
            .last()
            .filter(|child| {
                child.state().props.test_id.as_deref() == Some("texzi.rich.thumb-target")
            })
            .map(NshView::id)
    })
}

fn event(surface: &Surface, receiver: NodeId, name: &str, data: Value) {
    surface.event(
        receiver,
        EventKind::NativeEvent,
        EventPayload::Native {
            name: name.into(),
            data,
        },
    );
}

pub(super) fn trace() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| {
        std::env::var_os("TEXZI_TRACE_RICH_THUMB").as_deref() == Some(std::ffi::OsStr::new("1"))
    })
}

#[track_caller]
fn cancel_if(surface: &Surface, matches: impl FnOnce(Grab) -> bool) {
    let owner = &surface.input.rich_thumb;
    if let Some(grab) = owner.get()
        && matches(grab)
    {
        if trace() {
            eprintln!(
                "THUMB_DIAG {:?} native-cancel caller={} grab={grab:?}",
                std::time::Instant::now(),
                std::panic::Location::caller()
            );
        }
        owner.set(None);
        event(surface, grab.receiver, "thumbCancel", json!([grab.gesture]));
    }
}

#[track_caller]
pub(crate) fn cancel_window(surface: &Surface, hwnd: HWND) {
    cancel_if(surface, |grab| grab.window == hwnd);
}

#[track_caller]
pub(crate) fn supersede_input(surface: &Surface) {
    if surface.input.rich_thumb.get().is_some() {
        cancel_window(
            surface,
            app_window::hwnd_showing(surface, &surface.input_root()),
        );
    }
}

#[track_caller]
pub(crate) fn supersede_view(surface: &Surface, view: &NshView) {
    cancel_if(surface, |grab| grab.view == view.id());
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VirtualThumb {
    receiver: NodeId,
    gesture: u32,
    origin: f32,
    start: f32,
    travel: f32,
    last: f32,
}

impl VirtualThumb {
    pub(crate) fn capture(
        surface: &Surface,
        view: &NshView,
        origin: f32,
        gesture: u32,
    ) -> Option<Self> {
        let receiver = receiver(view)?;
        let visible = view.visible_bounds();
        let state = view.state();
        let scroll = state.scroll.as_ref()?;
        let max = scroll.max(state.frame.size).y.0;
        let (_, travel) = scroll.thumb_within(state.frame.size, visible)?;
        if scroll.horizontal || max <= 0.0 || travel.0 <= 0.0 {
            return None;
        }
        let start = (scroll.y.0 / max).clamp(0.0, 1.0);
        drop(state);
        cancel_if(surface, |_| true);
        surface.input.rich_thumb.set(Some(Grab {
            view: view.id(),
            receiver,
            gesture,
            window: app_window::hwnd_showing(surface, view),
            last_ticket: 0,
        }));
        event(surface, receiver, "thumbBegin", json!([gesture]));
        Some(Self {
            receiver,
            gesture,
            origin,
            start,
            travel: travel.0,
            last: start,
        })
    }

    pub(crate) fn move_to(&mut self, surface: &Surface, y: f32) {
        let target = (self.start + (y - self.origin) / self.travel).clamp(0.0, 1.0);
        if trace() && target >= 0.79 {
            eprintln!(
                "THUMB_DIAG {:?} native-intent receiver={} gesture={} y={y} origin={} travel={} fraction={target} owner={:?}",
                std::time::Instant::now(),
                self.receiver,
                self.gesture,
                self.origin,
                self.travel,
                surface.input.rich_thumb.get()
            );
        }
        if !surface
            .input
            .rich_thumb
            .get()
            .is_some_and(|grab| grab.receiver == self.receiver && grab.gesture == self.gesture)
        {
            return;
        }
        if target != self.last {
            self.last = target;
            event(
                surface,
                self.receiver,
                "thumbTarget",
                json!([self.gesture, target]),
            );
        }
    }

    /// A normal release can still have a final mount in flight.
    pub(crate) fn finish(&self, surface: &Surface, cancelled: bool) {
        if trace() {
            eprintln!(
                "THUMB_DIAG {:?} native-end receiver={} gesture={} last={} cancelled={cancelled} owner={:?}",
                std::time::Instant::now(),
                self.receiver,
                self.gesture,
                self.last,
                surface.input.rich_thumb.get()
            );
        }
        if cancelled {
            cancel_if(surface, |grab| {
                grab.receiver == self.receiver && grab.gesture == self.gesture
            });
        }
    }
}

/// This is a user movement, not a Core-issued ScrollTo. Preserve the
/// ordinary command counter and report the actual clamped position.
pub(crate) fn command(surface: &Rc<Surface>, view: &NshView, name: &str, args: &Value) {
    let Some(mut grab) = surface.input.rich_thumb.get() else {
        return;
    };
    if grab.view != view.id()
        || args[0].as_u64() != Some(grab.receiver.into())
        || args[1].as_u64() != Some(grab.gesture.into())
    {
        return;
    }
    if name == "texziCancelRichThumb" || receiver(view) != Some(grab.receiver) {
        supersede_view(surface, view);
        return;
    }
    let (Some(ticket), Some(fraction)) = (args[2].as_u64(), args[3].as_f64()) else {
        return;
    };
    if ticket <= grab.last_ticket || !fraction.is_finite() {
        return;
    }
    grab.last_ticket = ticket;
    surface.input.rich_thumb.set(Some(grab));
    let host = Host::new(surface, view);
    if let Some((actual, applied)) = host.scroll_mut(|s, size| {
        let max = s.max(size).y.0.max(0.0);
        s.y = Points(fraction.clamp(0.0, 1.0) as f32 * max);
        (s.reported(size), s.applied)
    }) {
        host.moved(actual);
        host.report(actual, applied);
        event(
            surface,
            grab.receiver,
            "thumbApplied",
            json!([grab.gesture, ticket, actual.y.0]),
        );
    }
}
