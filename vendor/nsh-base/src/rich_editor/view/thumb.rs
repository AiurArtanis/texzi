//! A Windows scrollbar can cross thousands of unmounted blocks in one
//! pointer move. Keep both pages until Windows applies the mounted target.
//! A grab belongs to one receiver, which is replaced when the document or
//! its layout changes. Late commands and receipts carry that identity.

use std::{cell::Cell, rc::Rc, sync::OnceLock, time::Instant};

use nsh_core::{EventKind, EventPayload, NodeId, prelude::*};
use serde_json::json;

use super::{controller::Rv, render::has_block_window};

pub(super) fn tracing() -> bool {
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| {
        std::env::var_os("TEXZI_TRACE_RICH_THUMB").as_deref() == Some(std::ffi::OsStr::new("1"))
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Grab {
    receiver: NodeId,
    gesture: u32,
}

pub(super) struct ThumbScroll {
    pub destination_fraction: Signal<Option<f32>>,
    grab: Cell<Option<Grab>>,
    latest: Cell<Option<f32>>,
    ticket: Cell<u64>,
    awaiting_offset: Cell<bool>,
}

impl ThumbScroll {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            destination_fraction: signal(None),
            grab: Cell::default(),
            latest: Cell::default(),
            ticket: Cell::default(),
            awaiting_offset: Cell::default(),
        })
    }

    fn reset(&self) {
        self.grab.set(None);
        self.latest.set(None);
        self.awaiting_offset.set(false);
        self.destination_fraction.try_set(None);
    }
}

/// A direct child identifies a windowed rich scroller and the geometry
/// its absolute targets belong to. Ordinary height measurements keep the
/// receiver. Document edits, reflow and viewport resizes replace it.
pub(super) fn receiver(rv: Rv) -> AnyElement {
    let doc = rv.clone();
    let geometry = computed(move || {
        if has_block_window(&doc) {
            let size = doc.viewport.get();
            vec![(
                doc.doc_rev.get(),
                doc.block_layout.get(),
                size.width.0.to_bits(),
                size.height.0.to_bits(),
            )]
        } else {
            Vec::new()
        }
    });
    each(
        geometry,
        |key| *key,
        move |_, _| {
            let current = element_ref();
            let bound = Rc::new(Cell::new(None));
            let bound_at_cleanup = bound.clone();
            let editor = rv.clone();
            cleanup(move || {
                // The keyed receiver's old owner can be disposed after replacement.
                if let (Some(node), Some(grab)) = (bound_at_cleanup.get(), editor.thumb.grab.get())
                    && node == grab.receiver
                {
                    editor.cancel_thumb();
                }
            });
            let rv = rv.clone();
            let handle = current.clone();
            view()
                .element_ref(&current)
                .absolute()
                .w(px(0.0))
                .h(px(0.0))
                .pointer_events(PointerEvents::None)
                .accessibility_elements_hidden(true)
                .test_id("texzi.rich.thumb-target")
                .on(EventKind::NativeEvent, move |event: &EventPayload| {
                    let EventPayload::Native { name, data } = event else {
                        return;
                    };
                    let (Some(receiver), Some(gesture)) = (
                        handle.node(),
                        data[0].as_u64().and_then(|n| u32::try_from(n).ok()),
                    ) else {
                        return;
                    };
                    let grab = Grab { receiver, gesture };
                    match name.as_ref() {
                        "thumbBegin" => {
                            bound.set(Some(receiver));
                            rv.thumb.reset();
                            rv.thumb.grab.set(Some(grab));
                        }
                        "thumbTarget" if rv.thumb.grab.get() == Some(grab) => {
                            if let Some(fraction) = data[1].as_f64() {
                                rv.request_thumb(grab, fraction as f32);
                            }
                        }
                        "thumbApplied" if rv.thumb.grab.get() == Some(grab) => {
                            if let (Some(ticket), Some(actual)) =
                                (data[1].as_u64(), data[2].as_f64())
                            {
                                rv.thumb_applied(grab, ticket, px(actual as f32));
                            }
                        }
                        "thumbCancel" if rv.thumb.grab.get() == Some(grab) => rv.thumb.reset(),
                        _ => {}
                    }
                })
        },
    )
}

impl Rv {
    pub(super) fn cancel_thumb(&self) {
        if let Some(grab) = self.thumb.grab.get() {
            self.thumb.reset();
            self.scroll
                .command("texziCancelRichThumb", json!([grab.receiver, grab.gesture]));
        }
    }

    fn request_thumb(&self, grab: Grab, fraction: f32) {
        let fraction = fraction.clamp(0.0, 1.0);
        // Coalesce samples but let the page already mounting finish.
        if self.thumb.destination_fraction.get_untracked().is_some() {
            self.thumb.latest.set(Some(fraction));
            return;
        }
        let max = (self.content_height.get_untracked() - self.viewport.get_untracked().height)
            .0
            .max(0.0);
        let target = px(max * fraction);
        if tracing() {
            eprintln!(
                "TEXZI_THUMB {:?} request node={} gesture={} shown={} target={} max={max}",
                Instant::now(),
                grab.receiver,
                grab.gesture,
                self.scroll_y.get_untracked().0,
                target.0
            );
        }
        if (target - self.scroll_y.get_untracked()).0.abs() <= 0.5 {
            return;
        }
        let ticket = self.thumb.ticket.get().wrapping_add(1);
        self.thumb.ticket.set(ticket);
        self.thumb.destination_fraction.set(Some(fraction));
        let mounted = run_on_main_thread(|| ());
        let rv = self.clone();
        self.owner.run(|| {
            spawn(async move {
                let ready = mounted.await.is_ok();
                if rv.thumb.grab.get() != Some(grab) || rv.thumb.ticket.get() != ticket {
                    return;
                }
                if !ready || rv.scroll.node().is_none() {
                    rv.cancel_thumb();
                    return;
                }
                rv.thumb.awaiting_offset.set(true);
                if tracing() {
                    eprintln!("TEXZI_THUMB {:?} mount-applied node={} gesture={} ticket={ticket} fraction={fraction}",
                        Instant::now(), grab.receiver, grab.gesture);
                }
                rv.scroll.command("texziApplyRichThumb", json!([grab.receiver, grab.gesture, ticket, fraction]));
            })
        });
    }

    fn thumb_applied(&self, grab: Grab, ticket: u64, actual: Points) {
        if !self.thumb.awaiting_offset.get() || self.thumb.ticket.get() != ticket {
            return;
        }
        batch(|| {
            if tracing() {
                eprintln!(
                    "TEXZI_THUMB {:?} offset-applied node={} gesture={} ticket={ticket} actual={}",
                    Instant::now(),
                    grab.receiver,
                    grab.gesture,
                    actual.0
                );
            }
            self.scroll_y.set(actual);
            self.scrolled.set(actual);
            self.thumb.awaiting_offset.set(false);
            self.thumb.destination_fraction.set(None);
            if let Some(fraction) = self.thumb.latest.take() {
                self.request_thumb(grab, fraction);
            }
        });
    }
}
