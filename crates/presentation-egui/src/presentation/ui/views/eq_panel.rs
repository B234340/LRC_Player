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

/// Extra padding inside whatever frame hosts the panel, so the enable switch, the reset
/// button and the last band never sit on the frame's inner edge.
const PANEL_PADDING: egui::Margin = egui::Margin { left: 8, right: 8, top: 4, bottom: 2 };
/// Height of the slider block (value label, track, caption).
const SLIDER_HEIGHT: f32 = 190.0;
/// Space above / below the track for the value label and the caption.
const SLIDER_LABEL_H: f32 = 18.0;
/// Width of the dB scale column between the preamp and the bands.
const AXIS_W: f32 = 24.0;
/// Column width range of one slider (preamp and bands).
const COL_MIN: f32 = 26.0;
/// Narrowest columns (no dB scale labels, smaller value labels) before the block scrolls.
const COL_TIGHT: f32 = 22.0;
const COL_MAX: f32 = 40.0;
/// Gap range between columns.
const GAP_MIN: f32 = 2.0;
const GAP_MAX: f32 = 14.0;
/// dB values with a grid line (and a scale label).
const GRID_DB: [f32; 5] = [12.0, 6.0, 0.0, -6.0, -12.0];

/// Draw the EQ panel. Emits `SetEq` with the full new settings on every change.
///
/// Layout (fits any width; below ~340 px the slider block scrolls horizontally):
/// header row (title … "enabled" + switch), preset row (label, combo … reset), then the
/// preamp slider, a separator, a dB scale and the 10 band sliders over 0 / ±6 / ±12 dB grid
/// lines. Columns and gaps compress with the width.
pub fn eq_panel(ui: &mut Ui, props: &EqPanelProps) -> Vec<UiAction> {
    egui::Frame::new().inner_margin(PANEL_PADDING).show(ui, |ui| eq_panel_contents(ui, props)).inner
}

fn eq_panel_contents(ui: &mut Ui, props: &EqPanelProps) -> Vec<UiAction> {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let mut s = props.settings;
    let mut changed = false;

    // header: title … "enabled" + switch
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        ui.label(theme::title(ui, tr.t("eq.title")).color(pal.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut on = s.enabled;
            if toggle_switch(ui, &mut on).on_hover_text(tr.t("eq.enabled")).changed() {
                s.enabled = on;
                changed = true;
            }
            ui.add_space(2.0);
            ui.label(theme::muted(ui, tr.t("eq.enabled")));
        });
    });
    ui.add_space(8.0);

    // preset combo … reset. The combo shrinks so the reset button always fits.
    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        ui.label(theme::muted(ui, tr.t("eq.preset")));
        let reset_label = tr.t("eq.reset");
        // Width of the ghost button with icon (see `widgets::button`): text + icon + padding.
        let reset_w = ui.painter().layout_no_wrap(reset_label.to_owned(), FontId::proportional(theme::size::BODY), Color32::PLACEHOLDER).size().x + 22.0 + 28.0;
        let combo_w = (ui.available_width() - reset_w - ui.spacing().item_spacing.x * 2.0).clamp(72.0, 170.0);
        let current = matching_preset(&s).map(|p| tr.t(p.key)).unwrap_or(tr.t("eq.preset.custom"));
        egui::ComboBox::from_id_salt("eq_preset")
            .selected_text(current)
            .width(combo_w)
            .truncate()
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
            if button(ui, ButtonKind::Ghost, Some(Icon::Undo), reset_label).clicked() {
                s.preamp_db = 0.0;
                s.bands_db = [0.0; 10];
                changed = true;
            }
        });
    });
    ui.add_space(12.0);

    // sliders: [preamp] | [dB scale] [10 bands]. When narrow the scale labels go first
    // (grid lines stay), then the block scrolls horizontally.
    let avail = ui.available_width();
    if avail >= slider_block_width(COL_MIN, GAP_MIN, true) {
        changed |= slider_block(ui, &mut s, avail, true, tr);
    } else if avail >= slider_block_width(COL_TIGHT, GAP_MIN, false) {
        changed |= slider_block(ui, &mut s, avail, false, tr);
    } else {
        egui::ScrollArea::horizontal()
            .id_salt("eq_sliders")
            .auto_shrink([false, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                changed |= slider_block(ui, &mut s, slider_block_width(COL_TIGHT, GAP_MIN, false), false, tr);
            });
    }

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

/// Total width of the slider block: 11 columns, separator, optional scale and the gaps.
fn slider_block_width(col: f32, gap: f32, axis: bool) -> f32 {
    if axis {
        col * 11.0 + 1.0 + AXIS_W + gap * 12.0
    } else {
        col * 11.0 + 1.0 + gap * 11.0
    }
}

/// Preamp, separator, dB scale (if `axis`) and band sliders laid out in `width`. Returns
/// whether a value changed (written to `s`).
fn slider_block(ui: &mut Ui, s: &mut EqSettings, width: f32, axis: bool, tr: Tr) -> bool {
    let pal = theme::palette(ui);
    let (fixed, gaps) = if axis { (1.0 + AXIS_W, 12.0) } else { (1.0, 11.0) };
    let col_w = ((width - fixed - gaps * 6.0) / 11.0).clamp(if axis { COL_MIN } else { COL_TIGHT }, COL_MAX);
    let gap = ((width - fixed - col_w * 11.0) / gaps).clamp(GAP_MIN, GAP_MAX);
    let mut changed = false;
    // Grid lines go under the sliders: reserve their slot before the sliders paint.
    let grid_slot = ui.painter().add(egui::Shape::Noop);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let dim = !s.enabled;
        if let (Some(v), _) = vertical_db_slider(ui, Id::new("eq_preamp"), s.preamp_db, SLIDER_HEIGHT, col_w, tr.t("eq.preamp"), dim, true) {
            s.preamp_db = v;
            changed = true;
        }
        let (sep, _) = ui.allocate_exact_size(vec2(1.0, SLIDER_HEIGHT), Sense::hover());
        ui.painter().vline(sep.center().x, sep.y_range().shrink(18.0), Stroke::new(1.0, pal.border));
        let scale = if axis { ui.allocate_exact_size(vec2(AXIS_W, SLIDER_HEIGHT), Sense::hover()).0 } else { sep };
        let mut bands_x = scale.right()..=scale.right();
        for (i, hz) in EQ_FREQUENCIES_HZ.iter().enumerate() {
            let (v, rect) = vertical_db_slider(ui, Id::new(("eq_band", i)), s.bands_db[i], SLIDER_HEIGHT, col_w, &freq_label(*hz), dim, false);
            if let Some(v) = v {
                s.bands_db[i] = v;
                changed = true;
            }
            bands_x = *bands_x.start()..=rect.right();
        }
        // dB scale + grid (0 dB a little stronger), behind the band sliders.
        let track = track_y_range(scale);
        let to_y = |db: f32| egui::remap_clamp(db, -EQ_MAX_GAIN_DB..=EQ_MAX_GAIN_DB, *track.end()..=*track.start());
        let mut shapes = Vec::new();
        let font = FontId::proportional(9.5);
        for db in GRID_DB {
            let y = to_y(db).round() + 0.5;
            let color = if db == 0.0 { theme::with_alpha(pal.border_strong, 0.9) } else { theme::with_alpha(pal.border, 0.75) };
            shapes.push(egui::Shape::hline(*bands_x.start() + 2.0..=*bands_x.end(), y, Stroke::new(1.0, color)));
            if axis {
                let text = if db == 0.0 { "0".to_owned() } else { format!("{db:+.0}") };
                let g = ui.painter().layout_no_wrap(text, font.clone(), pal.text_faint);
                shapes.push(egui::Shape::galley(pos2(scale.right() - 3.0 - g.size().x, y - g.size().y / 2.0), g, pal.text_faint));
            }
        }
        ui.painter().set(grid_slot, egui::Shape::Vec(shapes));
    });
    changed
}

/// Vertical extent of a slider's track inside its column rect.
fn track_y_range(rect: Rect) -> std::ops::RangeInclusive<f32> {
    (rect.top() + SLIDER_LABEL_H + 8.0)..=(rect.bottom() - SLIDER_LABEL_H - 8.0)
}

/// Vertical ±12 dB slider with value label on top and caption below. Double-click resets
/// to 0, wheel adjusts by 0.5 dB, drag snaps to 0.5 dB (hold Shift for 0.1 dB). Returns the
/// new value (if changed) and the column rect.
#[allow(clippy::too_many_arguments)]
fn vertical_db_slider(ui: &mut Ui, id: Id, value: f32, height: f32, width: f32, caption: &str, dim: bool, preamp: bool) -> (Option<f32>, Rect) {
    let pal = theme::palette(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let ty = track_y_range(rect);
    let track = Rect::from_min_max(pos2(rect.center().x - 2.0, *ty.start()), pos2(rect.center().x + 2.0, *ty.end()));
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
    // track + zero tick (the bands share the panel's 0 dB grid line instead)
    painter.rect_filled(track, 2.0, theme::with_alpha(pal.text, if pal.dark { 0.12 } else { 0.10 }));
    let zy = to_y(0.0);
    if preamp {
        painter.hline(track.x_range().expand(6.0), zy, Stroke::new(1.0, pal.border_strong));
    }
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
    // Smaller labels when the columns are tight, so "+4.0" values don't touch.
    let small = width < 28.0;
    let g = painter.layout_no_wrap(val, FontId::proportional(if small { 9.5 } else { 11.0 }), vc);
    painter.galley(pos2(rect.center().x - g.size().x / 2.0, rect.top()), g, vc);
    let cc = if preamp { pal.text } else { pal.text_muted };
    let font = FontId::proportional(match (preamp, small) {
        (true, true) => 9.5,
        (true, false) => 10.5,
        (false, true) => 10.0,
        (false, false) => 11.0,
    });
    let g = painter.layout_no_wrap(caption.to_owned(), font, cc);
    painter.galley(pos2(rect.center().x - g.size().x / 2.0, rect.bottom() - g.size().y), g, cc);
    (out.map(|v| v.clamp(-max, max)), rect)
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
