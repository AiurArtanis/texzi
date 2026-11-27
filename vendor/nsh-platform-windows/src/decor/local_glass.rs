use std::cell::Cell;

use nsh_core::{Gradient, NodeId, Points};
use windows::Win32::Graphics::Dxgi::{
    DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3,
};

/// The tint and painted border for a live, app-opted-in translucent card.
/// Its ordinary view background and border supply the opaque alternative.
#[derive(Clone, Copy, PartialEq)]
pub struct LocalGlassStyle {
    /// Theme order is light, then dark.
    pub tint: &'static [Gradient; 2],
    pub glass_border: Points,
}

/// Opts in a live view in the calling engine. Call from an engine main-thread task.
pub fn set_local_glass(id: NodeId, style: LocalGlassStyle) -> bool {
    let Some(view) = crate::surface::task_surface().and_then(|surface| surface.view(id)) else {
        return false;
    };
    let changed = view.state().local_glass != Some(style);
    if changed {
        view.state_mut().local_glass = Some(style);
        view.changed();
    }
    true
}

/// Refreshes the active device's recent glass requests. Call with the app's
/// periodic visual-preference query from an engine main-thread task.
pub fn refresh_local_glass_pressure() {
    if crate::gfx::refresh_local_glass_pressure()
        && let Some(surface) = crate::surface::task_surface()
    {
        surface.invalidate();
    }
}

// Count realized copy rectangles including the blur margins. Two cards fit at
// ordinary scales, and a single reference card still fits at 200% DPI.
const FRAME_BYTES: u64 = 3 * 1024 * 1024;
const FRAME_COPIES: u8 = 2;
const DEVICE_HEADROOM: u64 = 16 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct FrameBudget {
    bytes: u64,
    copies: u8,
}

impl FrameBudget {
    pub(crate) fn reserve(&mut self, width: u32, height: u32, target: [u32; 2]) -> bool {
        let pixels = u64::from(width) * u64::from(height);
        let remaining = FRAME_BYTES - self.bytes;
        if pixels == 0
            || [width, height] == target
            || self.copies >= FRAME_COPIES
            || pixels > remaining / 4
        {
            return false;
        }
        self.bytes += pixels * 4;
        self.copies += 1;
        true
    }
}

/// A sample from the native adapter, shared by windows on this graphics device.
pub(crate) struct DevicePressure {
    adapter: IDXGIAdapter3,
    allowed: Cell<Option<bool>>,
    requested: Cell<bool>,
}

impl DevicePressure {
    pub(crate) fn new(adapter: IDXGIAdapter3) -> Self {
        Self {
            adapter,
            allowed: Cell::new(None),
            requested: Cell::new(false),
        }
    }

    pub(crate) fn allows_glass(&self) -> bool {
        self.requested.set(true);
        self.allowed.get().unwrap_or_else(|| self.query())
    }

    pub(crate) fn refresh(&self) -> bool {
        if !self.requested.replace(false) {
            return false;
        }
        let previous = self.allowed.get();
        previous != Some(self.query())
    }

    fn query(&self) -> bool {
        let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        // SAFETY: the adapter belongs to this device and the output is live.
        let enough = unsafe {
            self.adapter
                .QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info)
        }
        .is_ok()
            && info.Budget.saturating_sub(info.CurrentUsage) >= DEVICE_HEADROOM;
        self.allowed.set(Some(enough));
        enough
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_the_actual_padded_copy_per_frame() {
        let mut frame = FrameBudget::default();
        assert!(!frame.reserve(800, 600, [800, 600]));
        assert!(!frame.reserve(u32::MAX, u32::MAX, [1, 1]));
        assert!(frame.reserve(1024, 584, [1600, 1000]));
        assert!(!frame.reserve(600, 400, [1600, 1000]));
        assert!(frame.reserve(512, 292, [1600, 1000]));
        assert!(!frame.reserve(1, 1, [1600, 1000]));
        assert!(FrameBudget::default().reserve(512, 292, [820, 480]));
    }
}
