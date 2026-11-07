//! Keeps the window size stable when it moves between monitors with different scale factors
//! (e.g. a 100 % monitor next to a 150 % one).
//!
//! # The problem
//! egui-winit (0.35, still in 0.37) hands the viewport's `min_inner_size` to winit as a
//! *physical* size, computed with the scale factor of the monitor the window was created on,
//! and winit enforces that physical minimum unchanged on every monitor. A 708×616 pt minimum
//! set on a 150 % monitor stays 1062×924 px, which is 1.5× too large on a 100 % monitor:
//! when the window crosses to that monitor, Windows grows it while winit is trying to keep its
//! logical size. While the window straddles the monitor edge the growth moves its centre back
//! over the edge, the scale flips again, and the window keeps growing as it is dragged back and
//! forth. Correcting the size afterwards cannot catch this: several flips happen between frames.
//!
//! # The fix
//! The OS-enforced minimum must never be larger than the intended minimum on *any* monitor.
//! On Windows the physical minimum is `MIN_WINDOW_SIZE × lowest scale factor of all connected
//! monitors`, so crossing monitors never forces the window to grow. On the higher-DPI monitors
//! that minimum is smaller than intended, so once the window has kept the same size for a
//! moment (not being dragged or resized), a window below the minimum is grown back up to it.
//! That only ever grows *to* the minimum, never past it.
//!
//! Elsewhere winit keeps the minimum in logical units (macOS, Wayland), so nothing is needed.

use std::time::{Duration, Instant};

use eframe::egui::{self, Vec2, ViewportCommand};

/// How long the size must stay unchanged before a too-small window is grown to the minimum.
const SETTLE: Duration = Duration::from_millis(600);
/// How often the monitor scale factors are re-read (monitors can be plugged in / changed).
const MONITOR_POLL: Duration = Duration::from_secs(3);

/// What the guard knows about the window on one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    /// Native pixels per point of the window's current monitor.
    pub scale: f32,
    /// Lowest native scale factor among all connected monitors (None if unknown).
    pub lowest_scale: Option<f32>,
    /// egui zoom factor (points → physical also multiplies by this).
    pub zoom: f32,
    /// Inner size in points.
    pub size: Vec2,
    /// Maximized or fullscreen: the OS owns the size.
    pub fixed_size: bool,
    pub now: Instant,
}

#[derive(Debug, Default)]
pub(crate) struct ScaleGuard {
    /// Physical minimum last sent to the OS.
    sent_min_px: Option<Vec2>,
    /// Size on the previous frame and since when it has been unchanged.
    stable: Option<(Vec2, Instant)>,
    /// Lowest monitor scale, refreshed every `MONITOR_POLL`.
    lowest_scale: Option<(f32, Instant)>,
}

impl ScaleGuard {
    /// Returns the viewport commands to send for this frame.
    pub(crate) fn on_frame(&mut self, f: Frame, min_size: Vec2) -> Vec<ViewportCommand> {
        let mut commands = Vec::new();
        let Some(lowest) = f.lowest_scale else { return commands };
        let lowest = lowest.min(f.scale);

        // 1. OS minimum: never larger than the intended minimum on any monitor.
        let min_px = (min_size * lowest * f.zoom).round();
        if self.sent_min_px != Some(min_px) {
            self.sent_min_px = Some(min_px);
            // egui-winit multiplies by the current pixels per point (zoom × scale).
            commands.push(ViewportCommand::MinInnerSize(min_px / (f.scale * f.zoom)));
        }

        // 2. Grow a too-small window to the minimum once it has settled.
        let unchanged = self.stable.is_some_and(|(s, _)| (s - f.size).abs().max_elem() < 0.5);
        if !unchanged {
            self.stable = Some((f.size, f.now));
            return commands;
        }
        let since = self.stable.map_or(f.now, |(_, t)| t);
        let too_small = f.size.x < min_size.x - 0.5 || f.size.y < min_size.y - 0.5;
        if too_small && !f.fixed_size && f.now.duration_since(since) >= SETTLE {
            let target = f.size.max(min_size);
            commands.push(ViewportCommand::InnerSize(target));
            self.stable = Some((target, f.now));
        }
        commands
    }

    /// Reads the viewport state from `ctx` and sends the needed commands.
    pub(crate) fn update(&mut self, ctx: &egui::Context, min_size: Vec2) {
        let (scale, size, fixed, minimized) = ctx.input(|i| {
            let vp = i.viewport();
            let fixed = vp.maximized.unwrap_or(false) || vp.fullscreen.unwrap_or(false);
            (vp.native_pixels_per_point, vp.inner_rect.map(|r| r.size()), fixed, vp.minimized.unwrap_or(false))
        });
        let (Some(scale), Some(size)) = (scale, size) else { return };
        if minimized {
            return;
        }
        let now = Instant::now();
        let refresh = self.lowest_scale.is_none_or(|(_, t)| now.duration_since(t) >= MONITOR_POLL);
        if refresh {
            if let Some(lowest) = lowest_monitor_scale() {
                self.lowest_scale = Some((lowest, now));
            }
        }
        let frame = Frame {
            scale,
            lowest_scale: self.lowest_scale.map(|(s, _)| s),
            zoom: ctx.zoom_factor(),
            size,
            fixed_size: fixed,
            now,
        };
        for command in self.on_frame(frame, min_size) {
            ctx.send_viewport_cmd(command);
        }
        let too_small = size.x < min_size.x - 0.5 || size.y < min_size.y - 0.5;
        if too_small && !fixed && frame.lowest_scale.is_some() {
            // Wake up after the settle time to grow the window (no repaint otherwise).
            ctx.request_repaint_after(SETTLE);
        }
    }
}

/// Lowest effective scale factor among the connected monitors (Windows only).
#[cfg(windows)]
fn lowest_monitor_scale() -> Option<f32> {
    use winapi::shared::minwindef::{BOOL, LPARAM, TRUE};
    use winapi::shared::windef::{HDC, HMONITOR, LPRECT};
    use winapi::um::shellscalingapi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use winapi::um::winuser::EnumDisplayMonitors;

    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: LPRECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the `&mut u32` passed to EnumDisplayMonitors below, which only
        // calls back synchronously during that call.
        let lowest = unsafe { &mut *(data as *mut u32) };
        let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
        // SAFETY: valid monitor handle from the enumeration and valid out pointers.
        if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } == 0 && dpi_x > 0 {
            *lowest = (*lowest).min(dpi_x);
        }
        TRUE
    }

    let mut lowest_dpi = u32::MAX;
    // SAFETY: null DC and clip enumerate every monitor; the callback only writes `lowest_dpi`.
    let ok = unsafe {
        EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(collect), &mut lowest_dpi as *mut u32 as LPARAM)
    };
    (ok != 0 && lowest_dpi != u32::MAX).then(|| lowest_dpi as f32 / 96.0)
}

#[cfg(not(windows))]
fn lowest_monitor_scale() -> Option<f32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;

    const MIN: Vec2 = Vec2::new(708.0, 616.0);

    struct Sim {
        guard: ScaleGuard,
        t0: Instant,
        lowest: Option<f32>,
    }

    impl Sim {
        fn new(lowest: Option<f32>) -> Self {
            Self { guard: ScaleGuard::default(), t0: Instant::now(), lowest }
        }
        fn frame(&mut self, ms: u64, scale: f32, size: Vec2) -> Vec<ViewportCommand> {
            let f = Frame {
                scale,
                lowest_scale: self.lowest,
                zoom: 1.0,
                size,
                fixed_size: false,
                now: self.t0 + Duration::from_millis(ms),
            };
            self.guard.on_frame(f, MIN)
        }
    }

    /// Physical minimum the OS would enforce for a MinInnerSize command at `scale`.
    fn min_px(cmds: &[ViewportCommand], scale: f32) -> Option<Vec2> {
        cmds.iter().find_map(|c| match c {
            ViewportCommand::MinInnerSize(s) => Some(*s * scale),
            _ => None,
        })
    }

    fn inner(cmds: &[ViewportCommand]) -> Option<Vec2> {
        cmds.iter().find_map(|c| match c {
            ViewportCommand::InnerSize(s) => Some(*s),
            _ => None,
        })
    }

    #[test]
    fn os_minimum_uses_the_lowest_monitor_scale() {
        // Window on the 150 % monitor, a 100 % monitor is also connected.
        let mut sim = Sim::new(Some(1.0));
        let cmds = sim.frame(0, 1.5, vec2(1280.0, 820.0));
        assert_eq!(min_px(&cmds, 1.5), Some(MIN), "708×616 px, not 1062×924 px");
    }

    #[test]
    fn os_minimum_is_never_too_large_on_any_monitor() {
        for (scale, lowest) in [(1.0, 1.0), (1.5, 1.0), (2.0, 1.25), (1.25, 1.25)] {
            let mut sim = Sim::new(Some(lowest));
            let px = min_px(&sim.frame(0, scale, vec2(1280.0, 820.0)), scale).unwrap();
            for monitor_scale in [lowest, 1.25_f32.max(lowest), 1.5_f32.max(lowest), 2.0] {
                assert!(px.x <= MIN.x * monitor_scale + 0.5 && px.y <= MIN.y * monitor_scale + 0.5);
            }
        }
    }

    #[test]
    fn os_minimum_is_sent_once_and_survives_scale_changes() {
        let mut sim = Sim::new(Some(1.0));
        assert!(min_px(&sim.frame(0, 1.0, vec2(1280.0, 820.0)), 1.0).is_some());
        // Crossing monitors does not change the physical minimum: nothing is re-sent.
        for (i, scale) in [1.5, 1.0, 1.5, 1.0].into_iter().enumerate() {
            let cmds = sim.frame(10 + i as u64, scale, vec2(1280.0, 820.0));
            assert!(cmds.is_empty(), "scale {scale}: {cmds:?}");
        }
    }

    #[test]
    fn single_monitor_gets_the_exact_minimum() {
        let mut sim = Sim::new(Some(1.5));
        assert_eq!(min_px(&sim.frame(0, 1.5, vec2(1280.0, 820.0)), 1.5), Some((MIN * 1.5).round()));
    }

    #[test]
    fn a_too_small_window_grows_to_the_minimum_only_after_it_settles() {
        let mut sim = Sim::new(Some(1.0));
        let small = vec2(600.0, 500.0);
        sim.frame(0, 1.5, small);
        assert_eq!(inner(&sim.frame(100, 1.5, small)), None, "still settling");
        assert_eq!(inner(&sim.frame(700, 1.5, small)), Some(MIN));
        // Only to the minimum, and only once.
        assert!(sim.frame(800, 1.5, MIN).is_empty());
        assert!(sim.frame(2000, 1.5, MIN).is_empty());
    }

    #[test]
    fn partly_small_windows_keep_their_larger_side() {
        let mut sim = Sim::new(Some(1.0));
        sim.frame(0, 1.5, vec2(1000.0, 500.0));
        assert_eq!(inner(&sim.frame(700, 1.5, vec2(1000.0, 500.0))), Some(vec2(1000.0, MIN.y)));
    }

    #[test]
    fn a_window_that_keeps_changing_is_left_alone() {
        let mut sim = Sim::new(Some(1.0));
        for i in 0..20u64 {
            let size = vec2(600.0 + i as f32, 500.0);
            assert_eq!(inner(&sim.frame(i * 100, 1.5, size)), None);
        }
    }

    #[test]
    fn dragging_back_and_forth_never_grows_the_window() {
        // Window at its normal size straddling the monitor edge; the scale flips repeatedly.
        let mut sim = Sim::new(Some(1.0));
        let size = vec2(900.0, 650.0);
        for i in 0..50u64 {
            let scale = if i % 2 == 0 { 1.0 } else { 1.5 };
            let cmds = sim.frame(i * 16, scale, size);
            assert_eq!(inner(&cmds), None);
        }
    }

    #[test]
    fn maximized_windows_are_not_resized() {
        let mut sim = Sim::new(Some(1.0));
        let mut f = Frame {
            scale: 1.5,
            lowest_scale: Some(1.0),
            zoom: 1.0,
            size: vec2(600.0, 500.0),
            fixed_size: true,
            now: sim.t0,
        };
        sim.guard.on_frame(f, MIN);
        f.now += Duration::from_secs(2);
        assert_eq!(inner(&sim.guard.on_frame(f, MIN)), None);
    }

    #[test]
    fn unknown_monitor_scales_change_nothing() {
        let mut sim = Sim::new(None);
        assert!(sim.frame(0, 1.5, vec2(600.0, 500.0)).is_empty());
        assert!(sim.frame(2000, 1.5, vec2(600.0, 500.0)).is_empty());
    }

    #[test]
    fn zoom_is_taken_into_account() {
        let mut guard = ScaleGuard::default();
        let f = Frame {
            scale: 1.5,
            lowest_scale: Some(1.0),
            zoom: 1.25,
            size: vec2(1280.0, 820.0),
            fixed_size: false,
            now: Instant::now(),
        };
        let cmds = guard.on_frame(f, MIN);
        // egui-winit converts with zoom × scale; the OS minimum is MIN × lowest × zoom.
        assert_eq!(min_px(&cmds, 1.5 * 1.25), Some((MIN * 1.25).round()));
    }
}
