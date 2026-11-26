//! The Windows scrollbar can cross thousands of unmounted blocks in one
//! pointer move. Keep the displayed and requested pages until the platform
//! has mounted the destination and acknowledged its actual scroll offset.

use std::{cell::Cell, rc::Rc, sync::OnceLock, time::Instant};

use nsh_core::{EventKind, EventPayload, prelude::*};

use super::{controller::Rv, render::has_block_window};

pub(super) fn tracing() -> bool {
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| {
        std::env::var_os("TEXZI_TRACE_RICH_THUMB").as_deref() == Some(std::ffi::OsStr::new("1"))
    })
}

pub(super) struct ThumbScroll {
    pub destination: Signal<Option<Points>>,
    latest: Cell<Option<f32>>,
    ticket: Cell<u64>,
    awaiting_offset: Cell<bool>,
}

impl ThumbScroll {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            destination: signal(None),
            latest: Cell::default(),
            ticket: Cell::default(),
            awaiting_offset: Cell::default(),
        })
    }

    pub fn cancel(&self) {
        self.ticket.set(self.ticket.get().wrapping_add(1));
        self.latest.set(None);
        self.awaiting_offset.set(false);
        // The receiver also cleans up when its editor's owner is disposed.
        self.destination.try_set(None);
    }
}

/// A direct child identifies only a currently windowed rich scroller to
/// its Windows thumb. It occupies no space and never participates in input.
pub(super) fn receiver(rv: Rv) -> AnyElement {
    let doc = rv.clone();
    when(
        move || has_block_window(&doc),
        move || {
            let thumb = rv.thumb.clone();
            cleanup(move || thumb.cancel());
            let rv = rv.clone();
            view()
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
                    match name.as_ref() {
                        "thumbTarget" => {
                            if let Some(fraction) = data.as_f64() {
                                rv.request_thumb(fraction as f32);
                            }
                        }
                        "thumbApplied" => {
                            if let (Some(request), Some(actual)) =
                                (data[0].as_f64(), data[1].as_f64())
                            {
                                rv.thumb_applied(px(request as f32), px(actual as f32));
                            }
                        }
                        _ => {}
                    }
                })
        },
    )
    .into_element()
}

impl Rv {
    fn request_thumb(&self, fraction: f32) {
        // A fast pointer can request several destinations before one is
        // ready. Finish the in-flight page and then take the latest one.
        if self.thumb.destination.get_untracked().is_some() {
            self.thumb.latest.set(Some(fraction));
            return;
        }
        let max = (self.content_height.get_untracked() - self.viewport.get_untracked().height)
            .0
            .max(0.0);
        let target = px(max * fraction.clamp(0.0, 1.0));
        if tracing() {
            eprintln!(
                "TEXZI_THUMB {:?} request node={:?} fraction={fraction} shown={} target={} max={max}",
                Instant::now(),
                self.scroll.node(),
                self.scroll_y.get_untracked().0,
                target.0
            );
        }
        if (target - self.scroll_y.get_untracked()).0.abs() <= 0.5 {
            return;
        }
        let ticket = self.thumb.ticket.get().wrapping_add(1);
        self.thumb.ticket.set(ticket);
        self.thumb.destination.set(Some(target));
        let mounted = run_on_main_thread(|| ());
        let rv = self.clone();
        self.owner.run(|| {
            spawn(async move {
                let ready = mounted.await.is_ok();
                if rv.thumb.ticket.get() != ticket {
                    return;
                }
                if !ready || rv.scroll.node().is_none() {
                    rv.thumb.cancel();
                    return;
                }
                rv.thumb.awaiting_offset.set(true);
                if tracing() {
                    eprintln!(
                        "TEXZI_THUMB {:?} mount-applied node={:?} target={}",
                        Instant::now(),
                        rv.scroll.node(),
                        target.0
                    );
                }
                rv.scroll.scroll_to(Point::new(px(0.0), target), false);
            })
        });
    }

    fn thumb_applied(&self, requested: Points, actual: Points) {
        if !self.thumb.awaiting_offset.get()
            || self.thumb.destination.get_untracked() != Some(requested)
        {
            return;
        }
        batch(|| {
            if tracing() {
                eprintln!(
                    "TEXZI_THUMB {:?} offset-applied node={:?} requested={} actual={}",
                    Instant::now(),
                    self.scroll.node(),
                    requested.0,
                    actual.0
                );
            }
            self.scroll_y.set(actual);
            self.scrolled.set(actual);
            self.thumb.awaiting_offset.set(false);
            self.thumb.destination.set(None);
            if let Some(fraction) = self.thumb.latest.take() {
                self.request_thumb(fraction);
            }
        });
    }
}
