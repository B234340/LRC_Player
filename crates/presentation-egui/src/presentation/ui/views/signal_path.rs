//! Signal-path display: the audible track's chain from the file to the device
//! (source → resampling → normalization → EQ → parametric EQ → crossfeed → headroom → volume
//! → output) with a quality badge. Used in Settings → 音質 and in the transport popover.

use eframe::egui::{self, pos2, vec2, Align, Color32, FontId, Layout, RichText, Sense, Stroke, Ui};

use crate::presentation::ui::{i18n::Tr, theme};
use lrc_player_application::infrastructure::audio::engine::{CrossfeedLevel, GainSource, ResamplerKind, SignalPath};

/// Overall verdict for a signal path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QualityBadge {
    /// Bit-perfect *and* exclusive: the device gets the file's samples.
    BitPerfect,
    /// Untouched by the app, but the OS mixer may still resample / mix (shared mode).
    Untouched,
    /// Some stage changes the samples.
    Processed,
}

/// Badge for a signal path. `bit_perfect` ignores exclusive mode, so only an exclusive
/// output counts as bit-perfect end to end.
pub fn quality_badge(s: &SignalPath) -> QualityBadge {
    match (s.bit_perfect, s.output.exclusive) {
        (true, true) => QualityBadge::BitPerfect,
        (true, false) => QualityBadge::Untouched,
        _ => QualityBadge::Processed,
    }
}

impl QualityBadge {
    pub fn label(self, tr: Tr) -> &'static str {
        match self {
            QualityBadge::BitPerfect => tr.t("signal.bit_perfect"),
            QualityBadge::Untouched => {
                if cfg!(windows) {
                    tr.t("signal.untouched_windows")
                } else {
                    tr.t("signal.untouched_os")
                }
            }
            QualityBadge::Processed => tr.t("signal.processed"),
        }
    }

    pub fn hint(self, tr: Tr) -> &'static str {
        match self {
            QualityBadge::BitPerfect => tr.t("signal.bit_perfect_hint"),
            QualityBadge::Untouched => tr.t("signal.untouched_hint"),
            QualityBadge::Processed => tr.t("signal.processed_hint"),
        }
    }

    /// Indicator color (dot in the transport, badge fill).
    pub fn color(self, pal: &theme::Palette) -> Color32 {
        match self {
            QualityBadge::BitPerfect => pal.success,
            QualityBadge::Untouched => pal.accent_fg,
            QualityBadge::Processed => pal.text_muted,
        }
    }
}

/// "44.1 kHz", "48 kHz", "176.4 kHz".
pub fn rate_text(hz: u32) -> String {
    if hz == 0 {
        return "—".into();
    }
    let k = hz as f32 / 1000.0;
    if (k - k.round()).abs() < 0.01 { format!("{} kHz", k.round() as u32) } else { format!("{k:.1} kHz") }
}

fn codec_text(codec: &str) -> String {
    let c = codec.trim();
    if c.is_empty() {
        return "—".into();
    }
    if c.starts_with("pcm") {
        return "PCM".into();
    }
    c.to_ascii_uppercase()
}

fn db_text(db: f32) -> String {
    if db.abs() < 0.05 { "0 dB".into() } else { format!("{db:+.1} dB") }
}

struct Stage {
    label: &'static str,
    detail: String,
    active: bool,
    /// Warning color for the detail (limiter engaged).
    warn: bool,
}

fn stages(s: &SignalPath, tr: Tr) -> Vec<Stage> {
    let off = tr.t("signal.off").to_string();
    let mut v = Vec::new();
    // source
    let mut src = vec![codec_text(&s.codec), rate_text(s.source_rate)];
    if let Some(b) = s.source_bits {
        src.push(format!("{b} bit"));
    }
    if s.source_channels > 0 {
        src.push(format!("{} ch", s.source_channels));
    }
    v.push(Stage { label: tr.t("signal.source"), detail: src.join(" · "), active: true, warn: false });
    // resampling / channel mapping
    let mut conv = Vec::new();
    if s.resampled {
        let kind = match s.resampler {
            Some(ResamplerKind::Fft) => " (FFT)",
            Some(ResamplerKind::Sinc { .. }) => " (Sinc)",
            None => "",
        };
        conv.push(format!("{} → {}{kind}", rate_text(s.source_rate), rate_text(s.output.sample_rate)));
    }
    if s.channels_mapped {
        conv.push(format!("{} → {} ch", s.source_channels, s.output.channels));
    }
    let resampling = !conv.is_empty();
    v.push(Stage {
        label: tr.t("signal.resample"),
        detail: if resampling { conv.join(" · ") } else { tr.t("signal.none").to_string() },
        active: resampling,
        warn: false,
    });
    // normalization
    let norm_active = s.normalization_db.abs() >= 0.05;
    let source = match s.normalization_source {
        GainSource::None => None,
        GainSource::Tag => Some(tr.t("signal.norm_tag")),
        GainSource::Hint => Some(tr.t("signal.norm_measured")),
    };
    let norm_detail = match (norm_active, source) {
        (true, Some(src)) => format!("{} ({src})", db_text(s.normalization_db)),
        (true, None) => db_text(s.normalization_db),
        (false, Some(src)) => format!("0 dB ({src})"),
        (false, None) => tr.t("signal.norm_none").to_string(),
    };
    v.push(Stage { label: tr.t("signal.normalization"), detail: norm_detail, active: norm_active, warn: false });
    let onoff = |on: bool| if on { tr.t("signal.on").to_string() } else { off.clone() };
    v.push(Stage { label: tr.t("signal.eq"), detail: onoff(s.eq), active: s.eq, warn: false });
    v.push(Stage { label: tr.t("signal.peq"), detail: onoff(s.parametric_eq), active: s.parametric_eq, warn: false });
    let xf = s.crossfeed.map(|l| match l {
        CrossfeedLevel::Weak => tr.t("crossfeed.weak"),
        CrossfeedLevel::Medium => tr.t("crossfeed.medium"),
        CrossfeedLevel::Strong => tr.t("crossfeed.strong"),
    });
    v.push(Stage { label: tr.t("signal.crossfeed"), detail: xf.map(str::to_string).unwrap_or_else(|| off.clone()), active: xf.is_some(), warn: false });
    let hr = s.headroom_db.abs() >= 0.05;
    v.push(Stage { label: tr.t("signal.headroom"), detail: if hr { db_text(s.headroom_db) } else { "0 dB".into() }, active: hr, warn: false });
    let vol_detail = if s.volume <= 0.0 {
        tr.t("signal.muted").to_string()
    } else if s.volume_active {
        db_text(20.0 * s.volume.log10())
    } else {
        "0 dB".into()
    };
    let vol_detail = if s.limiter_engaged { format!("{vol_detail} · {}", tr.t("signal.limiter")) } else { vol_detail };
    v.push(Stage { label: tr.t("signal.volume"), detail: vol_detail, active: s.volume_active || s.limiter_engaged, warn: s.limiter_engaged });
    // output
    let o = &s.output;
    let mut out = vec![rate_text(o.sample_rate), o.sample_format.clone(), format!("{} ch", o.channels)];
    out.push(if o.exclusive { tr.t("signal.exclusive").to_string() } else { tr.t("signal.shared").to_string() });
    // Dither is armed for the format but only runs while the sound is processed.
    if o.dither && !s.bit_perfect {
        out.push(tr.t("signal.dither").to_string());
    }
    v.push(Stage { label: tr.t("signal.output"), detail: out.join(" · "), active: true, warn: false });
    v
}

/// Pill with the badge text.
pub fn badge_pill(ui: &mut Ui, badge: QualityBadge, tr: Tr) -> egui::Response {
    let pal = theme::palette(ui);
    let color = badge.color(&pal);
    let font = FontId::proportional(11.5);
    let max_w = ui.available_width().max(60.0);
    let g = ui.painter().layout(badge.label(tr).to_owned(), font, color, max_w - 30.0);
    let size = vec2(g.size().x + 30.0, (g.size().y + 8.0).max(22.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, rect.height().min(22.0) / 2.0, theme::with_alpha(color, if pal.dark { 0.16 } else { 0.12 }));
    ui.painter().circle_filled(pos2(rect.left() + 12.0, rect.center().y), 3.5, color);
    ui.painter().galley(pos2(rect.left() + 21.0, rect.center().y - g.size().y / 2.0), g, color);
    resp.on_hover_text(badge.hint(tr))
}

/// The chain as a vertical list: a node per stage on a connecting line, the stage name and
/// its detail (wrapping when narrow). Inactive stages are dimmed. `None` shows a note.
pub fn signal_path_view(ui: &mut Ui, signal: Option<&SignalPath>, tr: Tr) {
    let pal = theme::palette(ui);
    let Some(s) = signal else {
        ui.add(egui::Label::new(theme::muted(ui, tr.t("signal.idle"))).wrap());
        return;
    };
    let badge = quality_badge(s);
    badge_pill(ui, badge, tr);
    ui.add_space(4.0);
    ui.add(egui::Label::new(theme::caption(ui, badge.hint(tr))).wrap());
    ui.add_space(8.0);

    let rows = stages(s, tr);
    let width = ui.available_width();
    let rail_x = 7.0;
    let label_w = (width * 0.36).clamp(84.0, 150.0);
    let stacked = width < 300.0;
    let mut nodes: Vec<(egui::Pos2, bool)> = Vec::new();
    let line_slot = ui.painter().add(egui::Shape::Noop);
    for st in &rows {
        let (fg, detail_fg) = if st.active { (pal.text, if st.warn { pal.warning } else { pal.text_muted }) } else { (pal.text_faint, pal.text_faint) };
        let resp = ui.allocate_ui_with_layout(vec2(width, 22.0), Layout::left_to_right(Align::Min), |ui| {
            ui.set_min_height(22.0);
            ui.add_space(22.0);
            let rest = ui.available_width();
            if stacked {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.add(egui::Label::new(RichText::new(st.label).size(12.5).color(fg)).wrap());
                    ui.add(egui::Label::new(RichText::new(&st.detail).size(11.5).color(detail_fg)).wrap());
                });
            } else {
                ui.allocate_ui_with_layout(vec2(label_w, 18.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_width(label_w);
                    ui.add(egui::Label::new(RichText::new(st.label).size(12.5).color(fg)).truncate());
                });
                ui.allocate_ui_with_layout(vec2((rest - label_w).max(40.0), 18.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&st.detail).size(12.0).color(detail_fg)).wrap());
                });
            }
        });
        let r = resp.response.rect;
        nodes.push((pos2(r.left() + rail_x, r.top() + 9.5), st.active));
        ui.add_space(3.0);
    }
    // rail + nodes
    if let (Some(first), Some(last)) = (nodes.first(), nodes.last()) {
        let mut shapes = vec![egui::Shape::line_segment([first.0, last.0], Stroke::new(1.5, theme::with_alpha(pal.border_strong, 0.8)))];
        for (p, active) in &nodes {
            if *active {
                shapes.push(egui::Shape::circle_filled(*p, 4.5, pal.accent));
            } else {
                shapes.push(egui::Shape::circle_filled(*p, 4.0, pal.elevated));
                shapes.push(egui::Shape::circle_stroke(*p, 3.5, Stroke::new(1.2, pal.text_faint)));
            }
        }
        ui.painter().set(line_slot, egui::Shape::Vec(shapes));
    }
    // device name
    if !s.output.name.is_empty() {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            ui.add(egui::Label::new(theme::caption(ui, &s.output.name)).truncate()).on_hover_text(&s.output.name);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lrc_player_application::infrastructure::audio::engine::DeviceInfo;

    fn sample(bit_perfect: bool, exclusive: bool) -> SignalPath {
        SignalPath {
            codec: "flac".into(),
            source_rate: 44_100,
            source_bits: Some(16),
            source_channels: 2,
            resampled: false,
            resampler: None,
            channels_mapped: false,
            output: DeviceInfo { name: "dev".into(), sample_rate: 44_100, channels: 2, sample_format: "I24".into(), exclusive, ..Default::default() },
            eq: false,
            parametric_eq: false,
            crossfeed: None,
            normalization_db: 0.0,
            normalization_source: GainSource::None,
            headroom_db: 0.0,
            volume: 1.0,
            volume_active: false,
            limiter_engaged: false,
            bit_perfect,
        }
    }

    #[test]
    fn badge_needs_exclusive_for_bit_perfect() {
        assert_eq!(quality_badge(&sample(true, true)), QualityBadge::BitPerfect);
        assert_eq!(quality_badge(&sample(true, false)), QualityBadge::Untouched);
        assert_eq!(quality_badge(&sample(false, true)), QualityBadge::Processed);
        assert_eq!(rate_text(44_100), "44.1 kHz");
        assert_eq!(rate_text(48_000), "48 kHz");
        assert_eq!(codec_text("pcm_s16le"), "PCM");
        let tr = Tr::JA;
        let st = stages(&sample(true, true), tr);
        assert_eq!(st.len(), 9);
        assert_eq!(st.iter().filter(|s| s.active).count(), 2, "source and output only");
        let mut s = sample(false, false);
        s.crossfeed = Some(CrossfeedLevel::Strong);
        s.limiter_engaged = true;
        let st = stages(&s, tr);
        assert_eq!(st[5].detail, tr.t("crossfeed.strong"));
        assert!(st[7].warn);
    }
}
