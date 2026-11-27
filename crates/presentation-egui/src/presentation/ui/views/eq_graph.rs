//! Frequency-response graph of the EQ popover: the combined response of the 10-band EQ, the
//! parametric EQ (of the edited channel) and both preamps on a log axis (20 Hz–20 kHz), with
//! the automatic headroom level when it applies. When auto-EQ curves are loaded it also shows
//! the target (dashed), the measurement (level-matched to the target) and the predicted result
//! (measurement + the channel's PEQ filters, preamp excluded), all shifted so the target averages 0 dB in the
//! normalization band, with a small legend. The curves are computed by the app
//! ([`EqCurve::compute`]) and cached; this module only draws them.

use eframe::egui::{self, pos2, vec2, Color32, FontId, Mesh, Rect, Sense, Shape, Stroke, Ui};

use crate::presentation::ui::{i18n::Tr, theme};
use lrc_player_application::infrastructure::audio::engine::{
    auto_headroom_preview_db, combined_response_db, fit_grid, prepare_curves, AutoEqOptions, EqSettings,
    FrequencyResponse, ParametricEq, PeqChannelMode, PeqSide,
};

/// Lowest / highest frequency on the graph.
pub const GRAPH_MIN_HZ: f32 = 20.0;
pub const GRAPH_MAX_HZ: f32 = 20_000.0;
/// Points of the cached curve (log spaced).
pub const CURVE_POINTS: usize = 240;

/// Auto-EQ curves to overlay (see the module docs).
#[derive(Clone, Copy, Debug)]
pub struct OverlayInput<'a> {
    pub source: Option<&'a FrequencyResponse>,
    pub target: Option<&'a FrequencyResponse>,
    /// Changes whenever the curves change (part of the cache key).
    pub rev: u64,
}

/// Overlay curves at [`EqCurve::freqs`] (dB, the target's band average at 0 dB).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CurveOverlay {
    pub target_db: Option<Vec<f32>>,
    pub source_db: Option<Vec<f32>>,
    /// Measurement + the channel's PEQ filters (preamp and graphic EQ excluded); `None` while
    /// the PEQ does nothing.
    pub result_db: Option<Vec<f32>>,
}

/// Cached response curve plus the inputs it was computed from.
#[derive(Clone, Debug, PartialEq)]
pub struct EqCurve {
    pub eq: EqSettings,
    pub peq: ParametricEq,
    /// PEQ channel shown.
    pub side: PeqSide,
    pub auto_headroom: bool,
    pub sample_rate: u32,
    pub freqs: Vec<f32>,
    /// Response in dB at `freqs` (EQ + PEQ + preamps; headroom not included).
    pub db: Vec<f32>,
    /// Automatic headroom gain (dB, < 0) when auto headroom is on and lowers the level.
    pub headroom_db: Option<f32>,
    /// `rev` of the overlay input (`None` without overlay).
    pub overlay_rev: Option<u64>,
    pub overlay: Option<CurveOverlay>,
}

/// `CURVE_POINTS` log-spaced frequencies from [`GRAPH_MIN_HZ`] to [`GRAPH_MAX_HZ`].
pub fn graph_freqs() -> Vec<f32> {
    let (lo, hi) = (GRAPH_MIN_HZ.ln(), GRAPH_MAX_HZ.ln());
    (0..CURVE_POINTS).map(|i| (lo + (hi - lo) * i as f32 / (CURVE_POINTS - 1) as f32).exp()).collect()
}

/// Mean of `db` over the points of `freqs` inside `band`.
fn band_mean(freqs: &[f32], db: &[f32], band: (f32, f32)) -> f32 {
    let (sum, n) = freqs.iter().zip(db).filter(|(f, _)| **f >= band.0 && **f <= band.1).fold((0.0f32, 0usize), |(s, n), (_, d)| (s + d, n + 1));
    if n > 0 { sum / n as f32 } else { 0.0 }
}

impl CurveOverlay {
    fn compute(input: &OverlayInput<'_>, peq: &ParametricEq, side: PeqSide, sample_rate: u32, freqs: &[f32]) -> Self {
        let opts = AutoEqOptions::default();
        let reference = input.target.cloned().unwrap_or_else(FrequencyResponse::flat);
        let grid = fit_grid();
        // The target as the fit sees it (smoothed); its band mean goes to 0 dB.
        let t = prepare_curves(&reference, &reference, &opts);
        let t_mean = band_mean(&grid, &t.target_db, opts.normalize_band);
        let on_graph = |db: Vec<f32>| FrequencyResponse { freqs: grid.clone(), db: db.into_iter().map(|d| d - t_mean).collect() }.resample(freqs);
        let target_db = input.target.map(|_| on_graph(t.target_db.clone()));
        let (source_db, result_db) = match input.source {
            Some(src) => {
                let source = on_graph(prepare_curves(src, &reference, &opts).source_db);
                // The channel's PEQ filters only (what 自動作成 fits, matching its RMS
                // errors): the preamp is a level change and the graphic EQ is a taste
                // adjustment on top (it is part of the EQ line).
                let mut peq0 = peq.clone();
                peq0.preamp_db = 0.0;
                peq0.right.preamp_db = 0.0;
                let corr = combined_response_db(&EqSettings::default(), &peq0, side, sample_rate, freqs);
                // No PEQ (off or flat): the result is the measurement itself, not drawn twice.
                let result = corr.iter().any(|c| c.abs() > 0.05).then(|| source.iter().zip(&corr).map(|(s, c)| s + c).collect());
                (Some(source), result)
            }
            None => (None, None),
        };
        CurveOverlay { target_db, source_db, result_db }
    }

    fn curves(&self) -> impl Iterator<Item = &Vec<f32>> {
        [&self.target_db, &self.source_db, &self.result_db].into_iter().flatten()
    }
}

impl EqCurve {
    pub fn compute(eq: &EqSettings, peq: &ParametricEq, side: PeqSide, auto_headroom: bool, sample_rate: u32, overlay: Option<OverlayInput<'_>>) -> Self {
        let freqs = graph_freqs();
        let db = combined_response_db(eq, peq, side, sample_rate, &freqs);
        let headroom_db = auto_headroom.then(|| auto_headroom_preview_db(eq, peq, sample_rate)).filter(|h| *h < -0.05);
        let overlay_rev = overlay.map(|o| o.rev);
        let overlay = overlay.filter(|o| o.source.is_some() || o.target.is_some()).map(|o| CurveOverlay::compute(&o, peq, side, sample_rate, &freqs));
        Self { eq: *eq, peq: peq.clone(), side, auto_headroom, sample_rate, freqs, db, headroom_db, overlay_rev, overlay }
    }

    /// Whether this curve was computed from these inputs (no recomputation needed).
    pub fn is_for(&self, eq: &EqSettings, peq: &ParametricEq, side: PeqSide, auto_headroom: bool, sample_rate: u32, overlay_rev: Option<u64>) -> bool {
        self.eq == *eq
            && self.peq == *peq
            && self.side == side
            && self.auto_headroom == auto_headroom
            && self.sample_rate == sample_rate
            && self.overlay_rev == overlay_rev
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
    let overlay = curve.and_then(|c| c.overlay.as_ref());
    let max_abs = curve
        .map(|c| c.db.iter().chain(overlay.into_iter().flat_map(|o| o.curves().flatten())).fold(0.0f32, |m, d| m.max(d.abs())))
        .unwrap_or(0.0);
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
    let flat = curve.is_flat() && overlay.is_none();
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
    // Overlay curves under the EQ line: target (dashed), measurement, predicted result.
    let colors = overlay_colors(&pal);
    if let Some(o) = overlay {
        let to_pts = |db: &Vec<f32>| curve.freqs.iter().zip(db).map(|(f, d)| pos2(to_x(*f), to_y(*d))).collect::<Vec<_>>();
        if let Some(t) = &o.target_db {
            clip.extend(Shape::dashed_line(&to_pts(t), Stroke::new(1.4, colors[0]), 4.0, 3.0));
        }
        if let Some(sdb) = &o.source_db {
            clip.add(Shape::line(to_pts(sdb), Stroke::new(1.3, colors[1])));
        }
        if let Some(r) = &o.result_db {
            clip.add(Shape::line(to_pts(r), Stroke::new(1.8, colors[2])));
        }
    }
    clip.add(Shape::line(pts.clone(), Stroke::new(2.0, line)));
    if let Some(o) = overlay {
        legend(ui, plot, o, &colors, line, tr);
    }
    // Which channel is shown (L・R 別).
    if curve.peq.channel_mode == PeqChannelMode::Separate {
        let text = if curve.side == PeqSide::Right { "R" } else { "L" };
        let g = painter.layout_no_wrap(text.to_owned(), FontId::proportional(10.5), pal.accent_fg);
        let r = Rect::from_min_size(pos2(plot.right() - g.size().x - 10.0, plot.top() + 2.0), g.size() + vec2(8.0, 2.0));
        painter.rect_filled(r, 3.0, theme::with_alpha(pal.accent, 0.18));
        painter.galley(pos2(r.left() + 4.0, r.top() + 1.0), g, pal.accent_fg);
    }

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

/// Colors of target (muted, dashed), measurement (orange; red when the accent itself is
/// orange-ish) and result (the text color: neutral and strong, unlike every accent).
fn overlay_colors(pal: &theme::Palette) -> [Color32; 3] {
    let dist = |a: Color32, b: Color32| {
        let d = |x: u8, y: u8| (x as i32 - y as i32).abs();
        d(a.r(), b.r()) + d(a.g(), b.g()) + d(a.b(), b.b())
    };
    let measured = if dist(pal.accent, pal.warning) < 160 { pal.danger } else { pal.warning };
    [pal.text_muted, theme::with_alpha(measured, 0.95), theme::with_alpha(pal.text, 0.9)]
}

/// Legend (top left of the plot): the overlay curves present plus the EQ line.
fn legend(ui: &Ui, plot: Rect, o: &CurveOverlay, colors: &[Color32; 3], eq_color: Color32, tr: Tr) {
    let pal = theme::palette(ui);
    let painter = ui.painter();
    let font = FontId::proportional(9.5);
    let items: Vec<(&str, Color32, bool)> = [
        (o.target_db.is_some(), tr.t("autoeq.legend_target"), colors[0], true),
        (o.source_db.is_some(), tr.t("autoeq.legend_source"), colors[1], false),
        (o.result_db.is_some(), tr.t("autoeq.legend_result"), colors[2], false),
        (true, tr.t("autoeq.legend_eq"), eq_color, false),
    ]
    .into_iter()
    .filter(|i| i.0)
    .map(|i| (i.1, i.2, i.3))
    .collect();
    let galleys: Vec<_> = items.iter().map(|(t, _, _)| painter.layout_no_wrap((*t).to_owned(), font.clone(), pal.text_muted)).collect();
    let swatch = 12.0;
    let gap = 8.0;
    let row_h = galleys.iter().map(|g| g.size().y).fold(0.0f32, f32::max);
    let total: f32 = galleys.iter().map(|g| swatch + 3.0 + g.size().x).sum::<f32>() + gap * (galleys.len().saturating_sub(1)) as f32;
    // One row when it fits; else a 2-column grid (narrow popovers).
    let item_w: Vec<f32> = galleys.iter().map(|g| swatch + 3.0 + g.size().x).collect();
    let one_row = total + 10.0 <= plot.width() - 40.0;
    let col_w = [0, 1].map(|c| item_w.iter().skip(c).step_by(2).copied().fold(0.0f32, f32::max));
    let size = if one_row {
        vec2(total + 10.0, row_h + 4.0)
    } else {
        vec2(col_w[0] + gap + col_w[1] + 10.0, row_h * galleys.len().div_ceil(2) as f32 + 4.0)
    };
    let bg = Rect::from_min_size(pos2(plot.left() + 4.0, plot.top() + 2.0), size);
    painter.rect_filled(bg, 4.0, theme::with_alpha(pal.elevated, 0.82));
    let mut x = bg.left() + 5.0;
    for (i, ((_, color, dashed), g)) in items.iter().zip(galleys).enumerate() {
        let (ix, y) = if one_row {
            (x, bg.top() + 2.0)
        } else {
            let c = i % 2;
            (bg.left() + 5.0 + if c == 1 { col_w[0] + gap } else { 0.0 }, bg.top() + 2.0 + row_h * (i / 2) as f32)
        };
        let cy = y + row_h / 2.0;
        if *dashed {
            painter.extend(Shape::dashed_line(&[pos2(ix, cy), pos2(ix + swatch, cy)], Stroke::new(1.4, *color), 3.0, 2.0));
        } else {
            painter.hline(ix..=ix + swatch, cy, Stroke::new(2.0, *color));
        }
        x += swatch + 3.0 + g.size().x + gap;
        painter.galley(pos2(ix + swatch + 3.0, y), g, pal.text_muted);
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
        let c = EqCurve::compute(&eq, &peq, PeqSide::Left, true, 48_000, None);
        assert_eq!(c.freqs.len(), CURVE_POINTS);
        assert!((c.freqs[0] - 20.0).abs() < 0.01 && (c.freqs[CURVE_POINTS - 1] - 20_000.0).abs() < 1.0);
        let peak = c.db.iter().cloned().fold(f32::MIN, f32::max);
        assert!((peak - 6.0).abs() < 0.2, "{peak}");
        assert!(c.headroom_db.is_some_and(|h| (h + 6.0).abs() < 0.2));
        assert!(c.is_for(&eq, &peq, PeqSide::Left, true, 48_000, None));
        assert!(!c.is_for(&eq, &peq, PeqSide::Left, false, 48_000, None));
        assert!(!c.is_for(&eq, &peq, PeqSide::Left, true, 48_000, Some(1)));
        assert!(!c.is_for(&eq, &peq, PeqSide::Right, true, 48_000, None));
        assert!(EqCurve::compute(&EqSettings::default(), &peq, PeqSide::Left, true, 48_000, None).is_flat());
        assert_eq!(hz_text(105.0), "105 Hz");
        assert_eq!(hz_text(1250.0), "1.25 kHz");
        assert_eq!(hz_label(2000.0), "2k");
    }

    #[test]
    fn overlay_is_level_matched_and_includes_the_eq() {
        use lrc_player_application::infrastructure::audio::engine::{FilterKind, PeqFilter};
        // Target: flat at 80 dB SPL; measurement: 100 dB with a 6 dB dip at 3 kHz.
        let freqs: Vec<f32> = (0..=200).map(|i| 20.0 * 1000f32.powf(i as f32 / 200.0)).collect();
        let target = FrequencyResponse { freqs: freqs.clone(), db: vec![80.0; freqs.len()] };
        let dip = PeqFilter { kind: FilterKind::Peak, freq_hz: 3000.0, gain_db: -6.0, q: 2.0 };
        let dip_db = lrc_player_application::infrastructure::audio::engine::filters_response_db(&[dip], &freqs);
        let source = FrequencyResponse { freqs: freqs.clone(), db: dip_db.iter().map(|d| 100.0 + d).collect() };
        let peq = ParametricEq { enabled: true, preamp_db: -6.0, filters: vec![PeqFilter { gain_db: 6.0, ..dip }], ..Default::default() };
        let input = OverlayInput { source: Some(&source), target: Some(&target), rev: 7 };
        let c = EqCurve::compute(&EqSettings::default(), &peq, PeqSide::Left, false, 48_000, Some(input));
        assert_eq!(c.overlay_rev, Some(7));
        let o = c.overlay.as_ref().unwrap();
        let t = o.target_db.as_ref().unwrap();
        assert!(t.iter().all(|d| d.abs() < 0.01), "target at 0 dB");
        let i3k = c.freqs.partition_point(|f| *f < 3000.0);
        let s = o.source_db.as_ref().unwrap();
        assert!(s[i3k] < -3.0, "dip visible: {}", s[i3k]);
        // The EQ (preamp excluded) fills the dip: the result is ~flat.
        let r = o.result_db.as_ref().unwrap();
        assert!(r.iter().all(|d| d.abs() < 0.8), "{r:?}");
        // Target only.
        let c = EqCurve::compute(&EqSettings::default(), &peq, PeqSide::Left, false, 48_000, Some(OverlayInput { source: None, target: Some(&target), rev: 1 }));
        let o = c.overlay.unwrap();
        assert!(o.target_db.is_some() && o.source_db.is_none() && o.result_db.is_none());
        // PEQ off: no separate result curve.
        let off = ParametricEq { enabled: false, ..peq.clone() };
        let c = EqCurve::compute(&EqSettings::default(), &off, PeqSide::Left, false, 48_000, Some(input));
        let o = c.overlay.unwrap();
        assert!(o.source_db.is_some() && o.result_db.is_none());
    }
}
