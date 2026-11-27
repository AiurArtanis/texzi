//! Device-local backdrop for a single recovery confirmation.

use std::cell::RefCell;

use nsh_core::{Bounds, Points};
use windows::{
    Win32::Graphics::{
        Direct2D::{
            Common::{
                D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COMPOSITE_MODE_SOURCE_COPY,
                D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D1_PIXEL_FORMAT,
            },
            D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_INTERPOLATION_MODE_LINEAR,
            ID2D1Bitmap1, ID2D1DeviceContext, ID2D1Image,
        },
        Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
    },
    core::Interface,
};
use windows_numerics::{Matrix3x2, Vector2};

use super::layer;
use crate::{geom::rect, render::DrawCx};

// This covers the transient source and the two prepared BGRA targets. It
// excludes driver/effect scratch. One ordinary 4K window fits this budget.
const MAX_BITMAP_BYTES: u64 = 96 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq)]
struct CaptureKey {
    window: (f32, f32),
    scale: f32,
    dark: bool,
    generation: u64,
    card: Bounds<Points>,
}

struct Images {
    backdrop: ID2D1Image,
    card: ID2D1Image,
}

enum Capture<T> {
    Pending,
    Ready(CaptureKey, T),
    Solid,
}

impl<T> Capture<T> {
    fn get(
        &mut self,
        key: Option<CaptureKey>,
        take: impl FnOnce(CaptureKey) -> Option<T>,
    ) -> Option<&T> {
        match (&*self, key) {
            (Self::Pending, Some(key)) => {
                *self = take(key).map_or(Self::Solid, |images| Self::Ready(key, images));
            }
            (Self::Ready(old, _), Some(key)) if *old == key => {}
            _ => *self = Self::Solid,
        }
        match self {
            Self::Ready(_, images) => Some(images),
            _ => None,
        }
    }
}

pub(crate) struct FrozenModal {
    serial: u64,
    capture: RefCell<Capture<Images>>,
}

impl FrozenModal {
    pub(crate) fn new(serial: u64, available: bool) -> Self {
        Self {
            serial,
            capture: RefCell::new(if available {
                Capture::Pending
            } else {
                Capture::Solid
            }),
        }
    }

    /// Must run in the complete visible pass, before any dialog pixels.
    pub(crate) fn backdrop(
        &self,
        cx: &mut DrawCx<'_>,
        size: (f32, f32),
        card: Bounds<Points>,
    ) -> bool {
        // An offscreen inspection may draw the ordinary solid presentation,
        // independently of the visible window's one opportunity to capture.
        if cx.offscreen {
            return false;
        }
        let key =
            (cx.backdrop && cx.damage.is_none() && cx.composed.is_empty()).then_some(CaptureKey {
                window: size,
                scale: cx.scale,
                dark: cx.dark,
                generation: crate::gfx::generation(),
                card,
            });
        let invalidated = matches!(
            &*self.capture.borrow(),
            Capture::Ready(old, _) if Some(*old) != key
        );
        if invalidated {
            self.release();
        }
        let mut capture = self.capture.borrow_mut();
        let Some(images) = capture.get(key, |key| copy(cx, key, self.serial)) else {
            return false;
        };
        // SAFETY: the prepared image belongs to the current graphics device.
        unsafe {
            cx.dc.DrawImage(
                &images.backdrop,
                None,
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_COPY,
            );
        }
        true
    }

    pub(crate) fn card(&self, cx: &mut DrawCx<'_>, card: Bounds<Points>, radius: f32) {
        let capture = self.capture.borrow();
        let Capture::Ready(_, images) = &*capture else {
            return;
        };
        cx.push_clip_rounded_shapes(card, [radius; 4]);
        // SAFETY: the same draw already validated the cache's key.
        unsafe {
            cx.dc.DrawImage(
                &images.card,
                Some(&Vector2 {
                    X: card.origin.x.0,
                    Y: card.origin.y.0,
                }),
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );
        }
        cx.pop();
    }

    pub(crate) fn release(&self) {
        let old = self.capture.replace(Capture::Solid);
        if matches!(old, Capture::Ready(..)) {
            log::debug!("recovery confirmation {} backdrop released", self.serial);
        }
    }

    pub(crate) fn resize(&self, size: (f32, f32)) {
        let changed = matches!(
            &*self.capture.borrow(),
            Capture::Ready(key, _) if key.window != size
        );
        if changed {
            self.release();
        }
    }
}

fn pixel_size(size: (f32, f32), scale: f32) -> Option<D2D_SIZE_U> {
    let pixels = |points: f32| {
        let extent = (points * scale).ceil();
        (extent.is_finite() && extent > 0.0 && extent < u32::MAX as f32).then_some(extent as u32)
    };
    Some(D2D_SIZE_U {
        width: pixels(size.0)?,
        height: pixels(size.1)?,
    })
}

fn bitmap_budget(window: D2D_SIZE_U, card: D2D_SIZE_U) -> bool {
    let area = |size: D2D_SIZE_U| u64::from(size.width) * u64::from(size.height);
    area(window)
        .checked_mul(2)
        .and_then(|total| total.checked_add(area(card)))
        .and_then(|total| total.checked_mul(4))
        .is_some_and(|bytes| bytes <= MAX_BITMAP_BYTES)
}

fn copy(cx: &mut DrawCx<'_>, key: CaptureKey, serial: u64) -> Option<Images> {
    let card_size = (key.card.size.width.0, key.card.size.height.0);
    let window = pixel_size(key.window, key.scale)?;
    let card = pixel_size(card_size, key.scale)?;
    if !bitmap_budget(window, card) {
        return None;
    }
    let (w, h) = key.window;
    let (source, _, _) = layer::backdrop(cx, 0.0, 0.0, w, h)?;
    let backdrop = bake(cx, &source, rect(0.0, 0.0, w, h), window, 6.0)?;
    let card = bake(cx, &source, key.card, card, 32.0)?;
    log::debug!(
        "recovery confirmation {} backdrop captured {}x{}",
        serial,
        window.width,
        window.height
    );
    Some(Images { backdrop, card })
}

fn bake(
    cx: &DrawCx<'_>,
    source: &ID2D1Image,
    region: Bounds<Points>,
    pixels: D2D_SIZE_U,
    sigma: f32,
) -> Option<ID2D1Image> {
    layer::with_context(cx.gfx, |dc| {
        let filtered = layer::blur(dc, source, sigma, true).ok()?;
        let dpi = 96.0 * cx.scale;
        // SAFETY: a live context and a separate target of the same device.
        let bitmap = unsafe {
            dc.CreateBitmap(
                pixels,
                None,
                0,
                &D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: dpi,
                    dpiY: dpi,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                    colorContext: std::mem::ManuallyDrop::new(None),
                },
            )
        }
        .ok()?;
        let target = BitmapDraw::begin(dc, &bitmap, dpi);
        // SAFETY: the source cannot alias the new target.
        unsafe {
            dc.SetTransform(&Matrix3x2::translation(
                -region.origin.x.0,
                -region.origin.y.0,
            ));
            dc.DrawImage(
                &filtered,
                None,
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_COPY,
            );
        }
        target.finish().ok()?;
        bitmap.cast().ok()
    })?
}

/// The pooled context must have no active draw or target when returned.
struct BitmapDraw<'a> {
    dc: &'a ID2D1DeviceContext,
    old_transform: Matrix3x2,
    old_dpi: (f32, f32),
    drawing: bool,
}

impl<'a> BitmapDraw<'a> {
    fn begin(dc: &'a ID2D1DeviceContext, bitmap: &ID2D1Bitmap1, dpi: f32) -> Self {
        let mut old_transform = Matrix3x2::identity();
        let mut old_dpi = (0.0, 0.0);
        // SAFETY: the pooled context is idle, with its target unset.
        unsafe {
            dc.GetTransform(&mut old_transform);
            dc.GetDpi(&mut old_dpi.0, &mut old_dpi.1);
            dc.SetTarget(bitmap);
            dc.SetDpi(dpi, dpi);
            dc.BeginDraw();
        }
        Self {
            dc,
            old_transform,
            old_dpi,
            drawing: true,
        }
    }

    fn finish(mut self) -> windows::core::Result<()> {
        self.drawing = false;
        // SAFETY: closes the BeginDraw from construction exactly once.
        unsafe { self.dc.EndDraw(None, None) }
    }
}

impl Drop for BitmapDraw<'_> {
    fn drop(&mut self) {
        // SAFETY: restore the idle context for its next borrower, including
        // when an operation failed before finish.
        unsafe {
            if self.drawing {
                let _ = self.dc.EndDraw(None, None);
            }
            self.dc.SetTarget(None);
            self.dc.SetDpi(self.old_dpi.0, self.old_dpi.1);
            self.dc.SetTransform(&self.old_transform);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn recovery_capture_runs_once_and_drops_the_previous_generation() {
        struct Retained<'a>(&'a Cell<usize>);
        impl Drop for Retained<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let mut key = CaptureKey {
            window: (640.0, 480.0),
            scale: 1.0,
            dark: false,
            generation: 7,
            card: rect(110.0, 40.0, 420.0, 400.0),
        };
        let released = Cell::new(0);
        let mut cache = Capture::Pending;
        assert!(
            cache
                .get(Some(key), |_| Some(Retained(&released)))
                .is_some()
        );
        assert!(
            cache
                .get(Some(key), |_| panic!("duplicate capture"))
                .is_some()
        );
        assert_eq!(released.get(), 0);
        key.generation += 1;
        assert!(
            cache
                .get(Some(key), |_| panic!("recapture after invalidation"))
                .is_none()
        );
        assert_eq!(released.get(), 1);
        assert!(
            cache
                .get(Some(key), |_| panic!("failed fallback latching"))
                .is_none()
        );
    }

    #[test]
    fn recovery_capture_checks_dimensions_and_transient_bitmap_budget() {
        let card = pixel_size((420.0, 400.0), 1.5).unwrap();
        let window = pixel_size((2560.0, 1440.0), 1.5).unwrap();
        assert!(bitmap_budget(window, card));
        let large = pixel_size((5120.0, 2880.0), 1.0).unwrap();
        assert!(!bitmap_budget(large, card));
        assert!(pixel_size((0.0, 1.0), 1.0).is_none());
        assert!(pixel_size((300.0, f32::INFINITY), 1.0).is_none());
        assert!(!bitmap_budget(
            D2D_SIZE_U {
                width: u32::MAX,
                height: u32::MAX
            },
            card
        ));
    }
}
