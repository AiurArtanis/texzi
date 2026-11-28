use std::{
    cell::Cell,
    time::{Duration, Instant},
};

use nsh_core::{Color, Gradient, NodeId, Points};
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
    /// Per-theme edge color, used only when the backdrop was sampled.
    pub glass_edge: [Color; 2],
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

/// Refreshes glass pressure while eligible windows are showing cards. Call with the app's
/// periodic visual-preference query from an engine main-thread task.
pub fn refresh_local_glass_pressure() {
    if let Some(surface) = crate::surface::task_surface() {
        crate::app_window::refresh_local_glass_pressure(&surface);
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
    /// The cached pressure allowance used by an eligible card in this frame.
    pub(crate) pressure_allowed: Option<bool>,
}

impl FrameBudget {
    pub(crate) fn allows_glass(&mut self, sample: impl FnOnce() -> bool) -> bool {
        *self.pressure_allowed.get_or_insert_with(sample)
    }

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
    sample: PressureSample,
}

impl DevicePressure {
    pub(crate) fn new(adapter: IDXGIAdapter3) -> Self {
        Self {
            adapter,
            sample: PressureSample::default(),
        }
    }

    pub(crate) fn allows_glass(&self) -> bool {
        self.refresh(true).unwrap_or(false)
    }

    pub(crate) fn refresh(&self, interested: bool) -> Option<bool> {
        self.sample
            .refresh(interested, Instant::now(), || self.query())
    }

    fn query(&self) -> bool {
        let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        // SAFETY: the adapter belongs to this device and the output is live.
        unsafe {
            self.adapter
                .QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info)
        }
        .is_ok()
            && info.Budget.saturating_sub(info.CurrentUsage) >= DEVICE_HEADROOM
    }
}

#[derive(Default)]
struct PressureSample {
    current: Cell<Option<(Instant, bool)>>,
}

impl PressureSample {
    fn refresh(
        &self,
        interested: bool,
        now: Instant,
        query: impl FnOnce() -> bool,
    ) -> Option<bool> {
        if !interested {
            return None;
        }
        if let Some((last, allowed)) = self.current.get()
            && now.saturating_duration_since(last) < Duration::from_secs(1)
        {
            return Some(allowed);
        }
        let allowed = query();
        self.current.set(Some((now, allowed)));
        Some(allowed)
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

    #[test]
    fn static_card_recovers_and_a_dormant_window_does_not_query() {
        let pressure = PressureSample::default();
        let start = Instant::now();
        let queries = Cell::new(0);
        let query = || {
            queries.set(queries.get() + 1);
            queries.get() >= 3
        };
        assert_eq!(pressure.refresh(false, start, query), None);
        let presented = pressure.refresh(true, start, query);
        assert_eq!(presented, Some(false));

        let next = start + Duration::from_secs(1);
        assert_eq!(pressure.refresh(true, next, query), presented);
        // A second window or timer on the same device reuses that sample.
        assert_eq!(pressure.refresh(true, next, query), presented);
        assert_eq!(queries.get(), 2);

        // The still-presented card has not drawn again since the first denial.
        let recovered = start + Duration::from_secs(2);
        assert_eq!(pressure.refresh(true, recovered, query), Some(true));
        assert_eq!(queries.get(), 3);
        assert_eq!(
            pressure.refresh(false, recovered + Duration::from_secs(1), query),
            None
        );
        assert_eq!(
            pressure.refresh(false, recovered + Duration::from_secs(2), query),
            None
        );
        assert_eq!(queries.get(), 3);
    }

    #[test]
    fn first_eligible_frame_after_idle_rechecks_the_old_allowance() {
        let pressure = PressureSample::default();
        let start = Instant::now();
        let queries = Cell::new(0);
        let query = || {
            queries.set(queries.get() + 1);
            queries.get() == 1
        };
        let mut opened = FrameBudget::default();
        assert!(opened.allows_glass(|| pressure.refresh(true, start, query).unwrap()));
        let reopened = start + Duration::from_secs(30);
        assert_eq!(pressure.refresh(false, reopened, query), None);
        assert_eq!(queries.get(), 1);

        // The first eligible draw after reopening must observe the new denial.
        let mut reopened_frame = FrameBudget::default();
        assert!(!reopened_frame.allows_glass(|| pressure.refresh(true, reopened, query).unwrap()));
        assert!(!reopened_frame.allows_glass(|| panic!("the second card reuses this frame")));
        // The periodic check and draws on the same device share the interval.
        for millis in [0, 16, 999] {
            assert_eq!(
                pressure.refresh(true, reopened + Duration::from_millis(millis), query),
                Some(false)
            );
        }
        assert_eq!(queries.get(), 2);
        assert_eq!(
            pressure.refresh(true, reopened + Duration::from_secs(1), query),
            Some(false)
        );
        assert_eq!(queries.get(), 3);
    }
}
