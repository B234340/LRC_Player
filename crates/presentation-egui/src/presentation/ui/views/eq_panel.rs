//! Graphic equalizer panel: enable switch, presets, preamp, 10 vertical band sliders.

use eframe::egui::{
    self, pos2, vec2, Align, Color32, CursorIcon, FontId, Id, Layout, Rect, Sense, Stroke, Ui,
};

use crate::presentation::ui::{
    actions::{EqSettings, UiAction},
    i18n::Tr,
    icons::Icon,
    theme,
    widgets::{button, slider::wheel_notches, toggle_switch, ButtonKind},
};
use lrc_player_application::infrastructure::audio::engine::{EQ_FREQUENCIES_HZ, EQ_MAX_GAIN_DB};

/// A named preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqPreset {
    /// Localization key (`eq.preset.*`).
    pub key: &'static str,
    pub preamp_db: f32,
    pub bands_db: [f32; 10],
}

pub const EQ_PRESETS: &[EqPreset] = &[
    EqPreset { key: "eq.preset.flat", preamp_db: 0.0, bands_db: [0.0; 10] },
    EqPreset { key: "eq.preset.rock", preamp_db: -3.0, bands_db: [4.5, 3.5, 2.0, -0.5, -1.5, -0.5, 1.5, 3.0, 4.0, 4.5] },
    EqPreset { key: "eq.preset.pop", preamp_db: -2.0, bands_db: [-1.0, 1.0, 2.5, 3.5, 2.5, 0.0, -1.0, -1.0, 0.5, 1.0] },
    EqPreset { key: "eq.preset.jazz", preamp_db: -2.0, bands_db: [3.0, 2.0, 1.0, 2.0, -1.5, -1.5, 0.0, 1.5, 3.0, 3.5] },
    EqPreset { key: "eq.preset.classical", preamp_db: -2.0, bands_db: [4.0, 3.0, 2.0, 1.0, -1.0, -1.0, 0.0, 2.0, 3.0, 4.0] },
    EqPreset { key: "eq.preset.vocal", preamp_db: -2.5, bands_db: [-2.0, -1.5, -1.0, 1.0, 3.5, 4.0, 3.0, 1.5, 0.0, -1.0] },
    EqPreset { key: "eq.preset.bass_boost", preamp_db: -5.0, bands_db: [6.0, 5.0, 4.0, 2.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0] },
    EqPreset { key: "eq.preset.treble_boost", preamp_db: -4.0, bands_db: [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.5, 4.0, 5.0, 6.0] },
];

/// Preset whose values equal `s` (within 0.05 dB), if any.
pub fn matching_preset(s: &EqSettings) -> Option<&'static EqPreset> {
    EQ_PRESETS.iter().find(|p| {
        (p.preamp_db - s.preamp_db).abs() < 0.05
            && p.bands_db.iter().zip(s.bands_db.iter()).all(|(a, b)| (a - b).abs() < 0.05)
    })
}

pub struct EqPanelProps {
    pub settings: EqSettings,
    pub tr: Tr,
}

/// Short frequency label: `31`, `250`, `1k`, `16k`.
pub fn freq_label(hz: f32) -> String {
    if hz >= 1000.0 {
        let k = hz / 1000.0;
        if (k - k.round()).abs() < 0.05 {
            format!("{}k", k.round() as u32)
        } else {
            format!("{k:.1}k")
        }
    } else {
        format!("{}", hz.round() as u32)
    }
}

/// Draw the EQ panel. Emits `SetEq` with the full new settings on every change.
pub fn eq_panel(ui: &mut Ui, props: &EqPanelProps) -> Vec<UiAction> {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let mut s = props.settings;
    let mut changed = false;

    // header: title + enable switch
    ui.horizontal(|ui| {
        ui.label(theme::title(ui, tr.t("eq.title")).color(pal.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut on = s.enabled;
            if toggle_switch(ui, &mut on).on_hover_text(tr.t("eq.enabled")).changed() {
                s.enabled = on;
                changed = true;
            }
            ui.label(theme::muted(ui, tr.t("eq.enabled")));
        });
    });
    ui.add_space(8.0);

    // preset + reset
    ui.horizontal(|ui| {
        ui.label(theme::muted(ui, tr.t("eq.preset")));
        let current = matching_preset(&s).map(|p| tr.t(p.key)).unwrap_or(tr.t("eq.preset.custom"));
        egui::ComboBox::from_id_salt("eq_preset")
            .selected_text(current)
            .width(150.0)
            .show_ui(ui, |ui| {
                for p in EQ_PRESETS {
                    let sel = matching_preset(&s).is_some_and(|m| m.key == p.key);
                    if ui.selectable_label(sel, tr.t(p.key)).clicked() {
                        s.preamp_db = p.preamp_db;
                        s.bands_db = p.bands_db;
                        if !s.enabled && p.key != "eq.preset.flat" {
                            s.enabled = true;
                        }
                        changed = true;
                    }
                }
            });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if button(ui, ButtonKind::Ghost, Some(Icon::Undo), tr.t("eq.reset")).clicked() {
                s.preamp_db = 0.0;
                s.bands_db = [0.0; 10];
                changed = true;
            }
        });
    });
    ui.add_space(14.0);

    // sliders
    let height = 190.0;
    let avail = ui.available_width();
    let col_w = ((avail - 1.0) / 11.0 - 2.0).clamp(28.0, 40.0);
    let gap = ((avail - col_w * 11.0 - 1.0) / 11.0).clamp(0.0, 12.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let dim = !s.enabled;
        if let Some(v) = vertical_db_slider(ui, Id::new("eq_preamp"), s.preamp_db, height, col_w, tr.t("eq.preamp"), dim, true) {
            s.preamp_db = v;
            changed = true;
        }
        let (sep, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
        ui.painter().vline(sep.center().x, sep.y_range().shrink(18.0), Stroke::new(1.0, pal.border));
        for (i, hz) in EQ_FREQUENCIES_HZ.iter().enumerate() {
            if let Some(v) = vertical_db_slider(ui, Id::new(("eq_band", i)), s.bands_db[i], height, col_w, &freq_label(*hz), dim, false) {
                s.bands_db[i] = v;
                changed = true;
            }
        }
    });

    if changed {
        s.preamp_db = s.preamp_db.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);
        for b in &mut s.bands_db {
            *b = b.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);
        }
        vec![UiAction::SetEq(s)]
    } else {
        Vec::new()
    }
}

/// Vertical ±12 dB slider with value label on top and caption below. Double-click resets
/// to 0, wheel adjusts by 0.5 dB, drag snaps to 0.5 dB (hold Shift for 0.1 dB).
#[allow(clippy::too_many_arguments)]
fn vertical_db_slider(ui: &mut Ui, id: Id, value: f32, height: f32, width: f32, caption: &str, dim: bool, preamp: bool) -> Option<f32> {
    let pal = theme::palette(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let label_h = 18.0;
    let track = Rect::from_min_max(pos2(rect.center().x - 2.0, rect.top() + label_h + 8.0), pos2(rect.center().x + 2.0, rect.bottom() - label_h - 8.0));
    let hit = Rect::from_min_max(pos2(rect.left(), track.top() - 8.0), pos2(rect.right(), track.bottom() + 8.0));
    let resp = ui.interact(hit, id, Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeVertical);
    let max = EQ_MAX_GAIN_DB;
    let to_y = |db: f32| egui::remap_clamp(db, -max..=max, track.bottom()..=track.top());
    let from_y = |y: f32| egui::remap_clamp(y, track.bottom()..=track.top(), -max..=max);
    let mut out = None;

    if resp.dragged() || resp.drag_started() {
        if let Some(p) = resp.interact_pointer_pos() {
            let fine = ui.input(|i| i.modifiers.shift);
            let step = if fine { 0.1 } else { 0.5 };
            let v = (from_y(p.y) / step).round() * step;
            if (v - value).abs() > 1e-4 {
                out = Some(v);
            }
        }
    }
    if resp.double_clicked() {
        out = Some(0.0);
    }
    if resp.hovered() {
        let n = wheel_notches(ui);
        if n != 0.0 {
            out = Some(((value + n.signum() * 0.5) * 2.0).round() / 2.0);
        }
    }

    let shown = out.unwrap_or(value);
    let active = resp.hovered() || resp.dragged();
    let accent = if dim { pal.text_faint } else if preamp { pal.text_muted } else { pal.accent };
    let painter = ui.painter();
    // track + zero line
    painter.rect_filled(track, 2.0, theme::with_alpha(pal.text, if pal.dark { 0.12 } else { 0.10 }));
    let zy = to_y(0.0);
    painter.hline(track.x_range().expand(6.0), zy, Stroke::new(1.0, pal.border_strong));
    let vy = to_y(shown);
    let fill = Rect::from_min_max(pos2(track.left(), vy.min(zy)), pos2(track.right(), vy.max(zy)));
    if fill.height() > 0.5 {
        painter.rect_filled(fill, 2.0, accent);
    }
    // knob
    let knob_r = if active { 8.0 } else { 7.0 };
    painter.circle_filled(pos2(track.center().x, vy + 1.0), knob_r + 0.5, Color32::from_black_alpha(50));
    painter.circle_filled(pos2(track.center().x, vy), knob_r, if pal.dark { Color32::WHITE } else { pal.elevated });
    painter.circle_stroke(pos2(track.center().x, vy), knob_r, Stroke::new(if active { 2.0 } else { 1.0 }, if active { accent } else { pal.border_strong }));
    // labels
    let val = if shown.abs() < 0.05 { "0".to_owned() } else { format!("{shown:+.1}") };
    let vc = if active { pal.text } else { pal.text_muted };
    let g = painter.layout_no_wrap(val, FontId::proportional(11.0), vc);
    painter.galley(pos2(rect.center().x - g.size().x / 2.0, rect.top()), g, vc);
    let cc = if preamp { pal.text } else { pal.text_muted };
    let font = if preamp { FontId::proportional(10.5) } else { FontId::proportional(11.0) };
    let g = painter.layout_no_wrap(caption.to_owned(), font, cc);
    painter.galley(pos2(rect.center().x - g.size().x / 2.0, rect.bottom() - g.size().y), g, cc);
    out.map(|v| v.clamp(-max, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_labels() {
        assert_eq!(freq_label(31.0), "31");
        assert_eq!(freq_label(1000.0), "1k");
        assert_eq!(freq_label(16000.0), "16k");
        let flat = EqSettings::default();
        assert_eq!(matching_preset(&flat).map(|p| p.key), Some("eq.preset.flat"));
        for p in EQ_PRESETS {
            assert!(p.bands_db.iter().all(|b| b.abs() <= EQ_MAX_GAIN_DB));
            let s = EqSettings { enabled: true, preamp_db: p.preamp_db, bands_db: p.bands_db };
            assert_eq!(matching_preset(&s).map(|m| m.key), Some(p.key));
        }
    }
}
