//! Seek bar: thin track that thickens on hover, hover-time bubble, drag preview,
//! click-to-seek.

use eframe::egui::{pos2, Id, Response, Ui};

use super::slider::{floating_label, slim_slider, SlimSliderStyle};
use crate::presentation::ui::{format::format_time, theme};

/// Output of [`seek_bar`].
pub struct SeekBarOutput {
    pub response: Response,
    /// Seek target (ms) committed this frame.
    pub seek_to: Option<u64>,
    /// Target (ms) while dragging; show it instead of the playback position.
    pub preview_ms: Option<u64>,
}

/// Seek bar of `width` points. `on_dark` switches to white-on-artwork colors.
pub fn seek_bar(
    ui: &mut Ui,
    id: Id,
    position_ms: u64,
    duration_ms: u64,
    width: f32,
    on_dark: bool,
) -> SeekBarOutput {
    let pal = theme::palette(ui);
    let style = if on_dark { SlimSliderStyle::on_dark() } else { SlimSliderStyle::themed(&pal) };
    let enabled = duration_ms > 0;
    let value = if enabled { position_ms as f32 / duration_ms as f32 } else { 0.0 };
    let out = slim_slider(ui, id, value, width, style);
    let to_ms = |f: f32| (f.clamp(0.0, 1.0) as f64 * duration_ms as f64).round() as u64;

    if !enabled {
        return SeekBarOutput { response: out.response, seek_to: None, preview_ms: None };
    }
    let preview_ms = out.dragging.map(to_ms);
    if let Some(v) = out.dragging.or(out.hover_value) {
        let r = out.response.rect;
        let track_left = r.left() + 6.0;
        let track_w = r.width() - 12.0;
        let x = track_left + track_w * v;
        floating_label(ui, id.with("bubble"), pos2(x, r.top()), &format_time(to_ms(v)));
    }
    SeekBarOutput {
        response: out.response,
        seek_to: out.committed.map(to_ms),
        preview_ms,
    }
}
