//! Frequency-response graph of the EQ popover: the combined response of the 10-band EQ, the
//! parametric EQ and both preamps on a log axis (20 Hz–20 kHz), with the automatic headroom
//! level when it applies. The curve is computed by the app ([`EqCurve::compute`]) and cached;
//! this module only draws it.

use eframe::egui::{self, pos2, vec2, Color32, FontId, Mesh, Rect, Sense, Shape, Stroke, Ui};

use crate::presentation::ui::{i18n::Tr, theme};
use lrc_player_application::infrastructure::audio::engine::{
    auto_headroom_preview_db, combined_response_db, EqSettings, ParametricEq,
};

/// Lowest / highest frequency on the graph.
pub const GRAPH_MIN_HZ: f32 = 20.0;
pub const GRAPH_MAX_HZ: f32 = 20_000.0;
/// Points of the cached curve (log spaced).
pub const CURVE_POINTS: usize = 240;

/// Cached response curve plus the inputs it was computed from.
#[derive(Clone, Debug, PartialEq)]
pub struct EqCurve {
    pub eq: EqSettings,
    pub peq: ParametricEq,
    pub auto_headroom: bool,
    pub sample_rate: u32,
    pub freqs: Vec<f32>,
    /// Response in dB at `freqs` (EQ + PEQ + preamps; headroom not included).
    pub db: Vec<f32>,
    /// Automatic headroom gain (dB, < 0) when auto headroom is on and lowers the level.
    pub headroom_db: Option<f32>,
}

/// `CURVE_POINTS` log-spaced frequencies from [`GRAPH_MIN_HZ`] to [`GRAPH_MAX_HZ`].
pub fn graph_freqs() -> Vec<f32> {
    let (lo, hi) = (GRAPH_MIN_HZ.ln(), GRAPH_MAX_HZ.ln());
    (0..CURVE_POINTS).map(|i| (lo + (hi - lo) * i as f32 / (CURVE_POINTS - 1) as f32).exp()).collect()
}

impl EqCurve {
    pub fn compute(eq: &EqSettings, peq: &ParametricEq, auto_headroom: bool, sample_rate: u32) -> Self {
        let freqs = graph_freqs();
        let db = combined_response_db(eq, peq, sample_rate, &freqs);
        let headroom_db = auto_headroom.then(|| auto_headroom_preview_db(eq, peq, sample_rate)).filter(|h| *h < -0.05);
        Self { eq: *eq, peq: peq.clone(), auto_headroom, sample_rate, freqs, db, headroom_db }
    }

    /// Whether this curve was computed from these inputs (no recomputation needed).
    pub fn is_for(&self, eq: &EqSettings, peq: &ParametricEq, auto_headroom: bool, sample_rate: u32) -> bool {
        self.eq == *eq && self.peq == *peq && self.auto_headroom == auto_headroom && self.sample_rate == sample_rate
    }

    /// Flat (both EQs off or neutral).
    pub fn is_flat(&self) -> bool {
        self.db.iter().all(|d| d.abs() < 0.01)
    }
}

/// Short frequency text: "20", "500", "1k", "1.5k".
fn hz_label(hz: f32) -> String {
    if hz >= 1000.0 {
        let k = hz / 1000.0;
        if (k - k.round()).abs() < 0.05 { format!("{}k", k.round() as u32) } else { format!("{k:.1}k") }
    } else {
        format!("{}", hz.round() as u32)
    }
}

/// Readable frequency: "105 Hz", "1.25 kHz", "12.5 kHz".
pub fn hz_text(hz: f32) -> String {
    if hz >= 10_000.0 {
        format!("{:.1} kHz", hz / 1000.0)
    } else if hz >= 1000.0 {
        format!("{:.2} kHz", hz / 1000.0)
    } else if hz >= 100.0 {
        format!("{hz:.0} Hz")
    } else {
        format!("{hz:.1} Hz")
    }
}

/// Draws the graph `height` tall across the available width.
pub fn response_graph(ui: &mut Ui, curve: Option<&EqCurve>, height: f32, tr: Tr) {
    let pal = theme::palette(ui);
    let width = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, theme::radius::MD, if pal.dark { theme::with_alpha(Color32::BLACK, 0.18) } else { pal.input_bg });
    painter.rect_stroke(rect, theme::radius::MD, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);

    let small = FontId::proportional(9.5);
    let label_w = 26.0;
    let plot = Rect::from_min_max(pos2(rect.left() + label_w, rect.top() + 8.0), pos2(rect.right() - 8.0, rect.bottom() - 16.0));
    let max_abs = curve.map(|c| c.db.iter().fold(0.0f32, |m, d| m.max(d.abs()))).unwrap_or(0.0);
    let headroom = curve.and_then(|c| c.headroom_db);
    let span = if max_abs.max(headroom.map_or(0.0, f32::abs)) > 14.5 { 30.0 } else { 15.0 };
    let step = if span > 15.0 { 10.0 } else { 5.0 };
    let (lo, hi) = (GRAPH_MIN_HZ.ln(), GRAPH_MAX_HZ.ln());
    let to_x = |hz: f32| egui::remap_clamp(hz.max(1.0).ln(), lo..=hi, plot.left()..=plot.right());
    let to_y = |db: f32| egui::remap_clamp(db, -span..=span, plot.bottom()..=plot.top());

    // dB grid + labels
    let mut db = -span;
    while db <= span + 0.01 {
        let y = to_y(db).round() + 0.5;
        let strong = db == 0.0;
        let color = if strong { theme::with_alpha(pal.border_strong, 0.9) } else { theme::with_alpha(pal.border, 0.7) };
        painter.hline(plot.x_range(), y, Stroke::new(1.0, color));
        if (db / step).round() as i32 % 2 == 0 || span <= 15.0 {
            let text = if db == 0.0 { "0".to_owned() } else { format!("{db:+.0}") };
            let g = painter.layout_no_wrap(text, small.clone(), pal.text_faint);
            painter.galley(pos2(plot.left() - 4.0 - g.size().x, y - g.size().y / 2.0), g, pal.text_faint);
        }
        db += step;
    }
    // frequency grid + labels
    let marks: &[f32] = if plot.width() >= 300.0 {
        &[20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0, 20_000.0]
    } else {
        &[20.0, 100.0, 1000.0, 10_000.0]
    };
    for &hz in marks {
        let x = to_x(hz).round() + 0.5;
        painter.vline(x, plot.y_range(), Stroke::new(1.0, theme::with_alpha(pal.border, 0.45)));
        let g = painter.layout_no_wrap(hz_label(hz), small.clone(), pal.text_faint);
        let gx = (x - g.size().x / 2.0).clamp(plot.left() - 2.0, rect.right() - 4.0 - g.size().x);
        painter.galley(pos2(gx, plot.bottom() + 3.0), g, pal.text_faint);
    }

    let Some(curve) = curve else { return };
    let clip = painter.with_clip_rect(plot.expand(1.0));
    let flat = curve.is_flat();
    let line = if flat { pal.text_faint } else { pal.accent };
    let pts: Vec<egui::Pos2> = curve.freqs.iter().zip(&curve.db).map(|(f, d)| pos2(to_x(*f), to_y(*d))).collect();
    if !flat {
        // Area between the curve and 0 dB.
        let zy = to_y(0.0);
        let fill = theme::with_alpha(pal.accent, if pal.dark { 0.20 } else { 0.16 });
        let mut mesh = Mesh::default();
        for w in pts.windows(2) {
            let base = mesh.vertices.len() as u32;
            for p in [w[0], w[1], pos2(w[1].x, zy), pos2(w[0].x, zy)] {
                mesh.colored_vertex(p, fill);
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
        clip.add(Shape::mesh(mesh));
    }
    clip.add(Shape::line(pts.clone(), Stroke::new(2.0, line)));

    if let Some(h) = headroom {
        let y = to_y(h);
        let color = pal.warning;
        clip.extend(Shape::dashed_line(&[pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.2, color), 5.0, 4.0));
        let text = tr.f("eq.graph_headroom", &[&format!("{h:+.1}")]);
        let g = painter.layout_no_wrap(text, small.clone(), color);
        let ty = if y + 3.0 + g.size().y <= plot.bottom() { y + 3.0 } else { y - 3.0 - g.size().y };
        painter.galley(pos2(plot.right() - 4.0 - g.size().x, ty), g, color);
    }

    // Hover readout.
    if let Some(p) = resp.hover_pos().filter(|p| plot.contains(*p)) {
        let hz = egui::remap_clamp(p.x, plot.left()..=plot.right(), lo..=hi).exp();
        let i = curve.freqs.partition_point(|f| *f < hz).min(curve.freqs.len().saturating_sub(1));
        let d = curve.db.get(i).copied().unwrap_or(0.0);
        let x = to_x(hz);
        painter.vline(x, plot.y_range(), Stroke::new(1.0, theme::with_alpha(pal.text, 0.35)));
        painter.circle_filled(pos2(x, to_y(d)), 3.5, line);
        let text = format!("{}  {d:+.1} dB", hz_text(hz));
        let g = painter.layout_no_wrap(text, FontId::proportional(10.5), pal.text);
        let gx = if x + 8.0 + g.size().x < plot.right() { x + 8.0 } else { x - 8.0 - g.size().x };
        let r = Rect::from_min_size(pos2(gx - 4.0, plot.top() + 2.0), g.size() + vec2(8.0, 4.0));
        painter.rect_filled(r, 4.0, pal.elevated);
        painter.rect_stroke(r, 4.0, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
        painter.galley(pos2(gx, plot.top() + 4.0), g, pal.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_cache_key_and_values() {
        let mut eq = EqSettings { enabled: true, ..Default::default() };
        eq.bands_db[5] = 6.0;
        let peq = ParametricEq::default();
        let c = EqCurve::compute(&eq, &peq, true, 48_000);
        assert_eq!(c.freqs.len(), CURVE_POINTS);
        assert!((c.freqs[0] - 20.0).abs() < 0.01 && (c.freqs[CURVE_POINTS - 1] - 20_000.0).abs() < 1.0);
        let peak = c.db.iter().cloned().fold(f32::MIN, f32::max);
        assert!((peak - 6.0).abs() < 0.2, "{peak}");
        assert!(c.headroom_db.is_some_and(|h| (h + 6.0).abs() < 0.2));
        assert!(c.is_for(&eq, &peq, true, 48_000));
        assert!(!c.is_for(&eq, &peq, false, 48_000));
        assert!(EqCurve::compute(&EqSettings::default(), &peq, true, 48_000).is_flat());
        assert_eq!(hz_text(105.0), "105 Hz");
        assert_eq!(hz_text(1250.0), "1.25 kHz");
        assert_eq!(hz_label(2000.0), "2k");
    }
}
