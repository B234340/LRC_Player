//! Volume control: mute toggle icon + slim slider with perceptual (cubic) mapping and
//! mouse-wheel adjustment.

use eframe::egui::{Id, Ui};

use super::slider::{floating_label, slim_slider, wheel_notches, SlimSliderStyle};
use crate::presentation::ui::{
    i18n::Tr,
    icons::{icon_button, Icon},
    theme,
};

/// Slider position (`0..=1`, perceptual) → linear gain (`0..=1`).
pub fn ui_to_gain(x: f32) -> f32 {
    let x = if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 };
    x * x * x
}

/// Linear gain → slider position. Inverse of [`ui_to_gain`].
pub fn gain_to_ui(gain: f32) -> f32 {
    let g = if gain.is_finite() { gain.clamp(0.0, 1.0) } else { 0.0 };
    g.cbrt()
}

/// One mouse-wheel notch / arrow key step in slider units.
pub const VOLUME_STEP: f32 = 0.05;

/// What the user did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VolumeOutput {
    /// New linear gain.
    pub set_gain: Option<f32>,
    pub toggle_mute: bool,
}

/// Mute button + slider. `gain` is linear; the slider shows `gain_to_ui(gain)`.
pub fn volume_control(
    ui: &mut Ui,
    id: Id,
    gain: f32,
    muted: bool,
    slider_width: f32,
    tr: &Tr,
) -> VolumeOutput {
    let mut out = VolumeOutput::default();
    let pal = theme::palette(ui);
    let pos = gain_to_ui(gain);
    let shown = if muted { 0.0 } else { pos };
    let inner = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let tip = if muted { tr.t("transport.unmute") } else { tr.t("transport.mute") };
        if icon_button(ui, Icon::for_volume(gain, muted), 18.0, false, tip).clicked() {
            out.toggle_mute = true;
        }
        if slider_width > 0.0 {
            let s = slim_slider(ui, id, shown, slider_width, SlimSliderStyle::themed(&pal));
            if let Some(v) = s.dragging.or(s.committed) {
                out.set_gain = Some(ui_to_gain(v));
            }
            if let Some(v) = s.dragging.or(s.hover_value.filter(|_| s.response.dragged())) {
                let r = s.response.rect;
                let x = r.left() + 6.0 + (r.width() - 12.0) * v;
                floating_label(ui, id.with("bubble"), eframe::egui::pos2(x, r.top()), &format!("{}%", (v * 100.0).round()));
            }
            s.response.on_hover_text(tr.t("transport.volume"));
        }
    });
    // Wheel anywhere over the control adjusts the volume.
    if ui.rect_contains_pointer(inner.response.rect) {
        let dy = wheel_notches(ui);
        if dy != 0.0 {
            let base = if muted { 0.0 } else { pos };
            let next = (base + dy * VOLUME_STEP).clamp(0.0, 1.0);
            out.set_gain = Some(ui_to_gain(next));
            if muted && dy > 0.0 {
                out.toggle_mute = true;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cubic_mapping_round_trips_and_is_monotonic() {
        assert_eq!(ui_to_gain(0.0), 0.0);
        assert_eq!(ui_to_gain(1.0), 1.0);
        assert!((ui_to_gain(0.5) - 0.125).abs() < 1e-6);
        let mut prev = -1.0;
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            let g = ui_to_gain(x);
            assert!(g >= prev);
            prev = g;
            assert!((gain_to_ui(g) - x).abs() < 1e-4, "x={x}");
        }
        assert_eq!(ui_to_gain(2.0), 1.0);
        assert_eq!(ui_to_gain(-1.0), 0.0);
        assert_eq!(gain_to_ui(f32::NAN), 0.0);
        assert!((gain_to_ui(0.001) - 0.1).abs() < 1e-4);
    }
}
