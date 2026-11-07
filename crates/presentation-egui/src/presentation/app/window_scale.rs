//! Keeps the window's minimum size and logical size stable when it moves between monitors
//! with different scale factors (e.g. a 150 % laptop panel and a 100 % external monitor).
//!
//! egui-winit (0.35, still in 0.37) hands the viewport's `min_inner_size` to winit as a
//! *physical* size, computed with the scale factor of the monitor the window was created on.
//! winit then enforces that stale physical minimum on every other monitor. Moving from a
//! 150 % to a 100 % monitor, a 708×616 pt minimum stays 1062×924 px, so Windows grows the
//! window while winit is trying to keep its logical size. Every round trip it grows a bit more,
//! and the growth can push the window's centre back over the monitor edge, so it flips between
//! monitors while being dragged.
//!
//! [`ScaleGuard`] runs once per frame. When the native scale factor changes, it re-sends the
//! minimum size (egui-winit converts it with the new scale) and restores the logical size the
//! window had just before the change.

use eframe::egui::{self, Vec2, ViewportCommand};

/// Sizes closer than this (in points) are treated as unchanged.
const SIZE_EPSILON: f32 = 1.0;

#[derive(Debug, Default)]
pub(crate) struct ScaleGuard {
    /// Native pixels per point seen on the previous frame.
    last_scale: Option<f32>,
    /// Logical inner size on the previous frame (not updated while maximized / fullscreen).
    last_size: Option<Vec2>,
}

impl ScaleGuard {
    /// Returns the viewport commands to send for this frame.
    ///
    /// `scale` is the native pixels per point, `size` the inner size in points, and
    /// `fixed_size` is true while the window is maximized or fullscreen (the OS owns the size).
    pub(crate) fn on_frame(
        &mut self,
        scale: Option<f32>,
        size: Option<Vec2>,
        fixed_size: bool,
        min_size: Vec2,
    ) -> Vec<ViewportCommand> {
        let (Some(scale), Some(size)) = (scale, size) else { return Vec::new() };
        let Some(prev_scale) = self.last_scale.replace(scale) else {
            // First frame: the minimum was applied with this scale when the window was created.
            self.last_size = (!fixed_size).then_some(size);
            return Vec::new();
        };

        if (prev_scale - scale).abs() < 1e-3 {
            if !fixed_size {
                self.last_size = Some(size);
            }
            return Vec::new();
        }

        // The scale factor changed: re-apply the minimum for the new monitor.
        let mut commands = vec![ViewportCommand::MinInnerSize(min_size)];
        if fixed_size {
            return commands;
        }
        // Undo any growth caused by the stale minimum, keeping the logical size from before.
        let target = self.last_size.unwrap_or(size).max(min_size);
        if (size - target).abs().max_elem() > SIZE_EPSILON {
            commands.push(ViewportCommand::InnerSize(target));
        }
        self.last_size = Some(target);
        commands
    }

    /// Reads the viewport state from `ctx` and sends the needed commands.
    pub(crate) fn update(&mut self, ctx: &egui::Context, min_size: Vec2) {
        let (scale, size, fixed) = ctx.input(|i| {
            let vp = i.viewport();
            let fixed = vp.maximized.unwrap_or(false) || vp.fullscreen.unwrap_or(false);
            (vp.native_pixels_per_point, vp.inner_rect.map(|r| r.size()), fixed)
        });
        if ctx.input(|i| i.viewport().minimized.unwrap_or(false)) {
            return;
        }
        for command in self.on_frame(scale, size, fixed, min_size) {
            ctx.send_viewport_cmd(command);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;

    const MIN: Vec2 = Vec2::new(708.0, 616.0);

    fn sizes(cmds: &[ViewportCommand]) -> (Option<Vec2>, Option<Vec2>) {
        let mut min = None;
        let mut inner = None;
        for c in cmds {
            match c {
                ViewportCommand::MinInnerSize(s) => min = Some(*s),
                ViewportCommand::InnerSize(s) => inner = Some(*s),
                _ => {}
            }
        }
        (min, inner)
    }

    #[test]
    fn nothing_happens_while_the_scale_stays_the_same() {
        let mut g = ScaleGuard::default();
        assert!(g.on_frame(Some(1.5), Some(vec2(1280.0, 820.0)), false, MIN).is_empty());
        assert!(g.on_frame(Some(1.5), Some(vec2(1000.0, 700.0)), false, MIN).is_empty());
        assert!(g.on_frame(Some(1.5), Some(vec2(900.0, 650.0)), false, MIN).is_empty());
    }

    #[test]
    fn moving_to_a_lower_scale_reapplies_the_minimum_and_undoes_growth() {
        let mut g = ScaleGuard::default();
        g.on_frame(Some(1.5), Some(vec2(800.0, 640.0)), false, MIN);
        // On the 100 % monitor the stale 1062×924 px minimum grew the window.
        let cmds = g.on_frame(Some(1.0), Some(vec2(1062.0, 924.0)), false, MIN);
        assert_eq!(sizes(&cmds), (Some(MIN), Some(vec2(800.0, 640.0))));
        // Next frame (size restored): nothing more to do.
        assert!(g.on_frame(Some(1.0), Some(vec2(800.0, 640.0)), false, MIN).is_empty());
    }

    #[test]
    fn moving_to_a_higher_scale_only_refreshes_the_minimum() {
        let mut g = ScaleGuard::default();
        g.on_frame(Some(1.0), Some(vec2(1280.0, 820.0)), false, MIN);
        let cmds = g.on_frame(Some(1.5), Some(vec2(1280.0, 820.0)), false, MIN);
        assert_eq!(sizes(&cmds), (Some(MIN), None));
    }

    #[test]
    fn round_trips_keep_the_size() {
        let mut g = ScaleGuard::default();
        let size = vec2(900.0, 650.0);
        g.on_frame(Some(1.5), Some(size), false, MIN);
        for scale in [1.0, 1.5, 1.0, 1.25, 1.5] {
            let cmds = g.on_frame(Some(scale), Some(size + vec2(40.0, 120.0)), false, MIN);
            assert_eq!(sizes(&cmds).1, Some(size), "scale {scale}");
            g.on_frame(Some(scale), Some(size), false, MIN);
        }
    }

    #[test]
    fn restored_size_is_never_below_the_minimum() {
        let mut g = ScaleGuard::default();
        g.on_frame(Some(1.5), Some(vec2(500.0, 400.0)), false, MIN);
        let cmds = g.on_frame(Some(1.0), Some(vec2(1062.0, 924.0)), false, MIN);
        assert_eq!(sizes(&cmds).1, Some(MIN));
    }

    #[test]
    fn maximized_windows_only_get_the_minimum() {
        let mut g = ScaleGuard::default();
        g.on_frame(Some(1.5), Some(vec2(1280.0, 820.0)), false, MIN);
        let cmds = g.on_frame(Some(1.0), Some(vec2(1920.0, 1040.0)), true, MIN);
        assert_eq!(sizes(&cmds), (Some(MIN), None));
    }

    #[test]
    fn missing_viewport_info_is_ignored() {
        let mut g = ScaleGuard::default();
        assert!(g.on_frame(None, Some(vec2(1.0, 1.0)), false, MIN).is_empty());
        assert!(g.on_frame(Some(1.0), None, false, MIN).is_empty());
    }

    /// Drives a real `egui::Context` and checks the commands leave through the viewport output.
    #[test]
    fn commands_reach_the_viewport_output() {
        use eframe::egui::{pos2, Rect, ViewportId, ViewportInfo};
        let ctx = egui::Context::default();
        let mut guard = ScaleGuard::default();
        let frame = |guard: &mut ScaleGuard, scale: f32, size: Vec2| {
            let mut input = egui::RawInput::default();
            input.viewports.insert(
                ViewportId::ROOT,
                ViewportInfo {
                    native_pixels_per_point: Some(scale),
                    inner_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), size)),
                    ..Default::default()
                },
            );
            let out = ctx.run_ui(input, |ui| guard.update(ui.ctx(), MIN));
            out.viewport_output.get(&ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default()
        };
        assert!(frame(&mut guard, 1.5, vec2(900.0, 650.0)).is_empty());
        let cmds = frame(&mut guard, 1.0, vec2(1062.0, 924.0));
        assert_eq!(sizes(&cmds), (Some(MIN), Some(vec2(900.0, 650.0))));
    }
}
