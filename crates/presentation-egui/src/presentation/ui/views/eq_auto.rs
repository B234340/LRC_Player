//! The 「自動作成（ターゲットに合わせる）」 section of the parametric-EQ tab: a collapsible
//! header, the target / measurement files (a flat target needs no file), the fitting options,
//! and 「フィルターを作成」 with the RMS errors of the last run and undo.
//!
//! Layout (fits ~280 px and up): file rows `label [ファイル…] name ×` (labels in a fixed
//! column), check boxes, an options grid with two label/value pairs per row when there is room
//! (one otherwise), then the create row (button, spinner / errors, undo on the right).

use eframe::egui::{self, vec2, Align, Color32, FontId, Layout, Sense, Ui};

use crate::presentation::ui::{
    actions::{AutoEqOptions, PeqSide, UiAction},
    i18n::Tr,
    icons::{icon_button, paint_icon, Icon},
    theme,
    views::eq_graph::hz_text,
    widgets::{button, toggle_switch, ButtonKind},
};
use lrc_player_application::infrastructure::audio::engine::PEQ_MAX_FILTERS;

/// Which imported curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CurveSlot {
    Target,
    /// The shared measurement, or the left one when L and R use different files.
    Source,
    /// The right earpiece's measurement.
    SourceRight,
}

/// RMS errors (dB) of one fit of the last auto-create.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FitSummary {
    /// `Some` when the fit applied to one channel of L・R 別.
    pub side: Option<PeqSide>,
    pub before_db: f32,
    pub after_db: f32,
    pub filters: usize,
}

/// What the section shows (built by the app every frame).
#[derive(Default)]
pub struct AutoEqView<'a> {
    pub expanded: bool,
    /// File names of the loaded curves.
    pub target: Option<&'a str>,
    pub target_flat: bool,
    pub source: Option<&'a str>,
    pub source_r: Option<&'a str>,
    pub same_source: bool,
    /// The PEQ is in L・R 別 mode.
    pub separate: bool,
    pub options: AutoEqOptions,
    /// A file dialog is open (for this slot).
    pub picking: Option<CurveSlot>,
    /// A fit is running.
    pub running: bool,
    pub last: &'a [FitSummary],
    pub can_undo: bool,
}

impl AutoEqView<'_> {
    /// A fit can run (target and the needed measurements are loaded).
    pub fn ready(&self) -> bool {
        let target = self.target_flat || self.target.is_some();
        let source = if self.same_source || !self.separate {
            self.source.is_some() || (!self.same_source && self.source_r.is_some())
        } else {
            self.source.is_some() && self.source_r.is_some()
        };
        target && source
    }
}

/// Parses "1200", "1.2k", "1.2 kHz", "850 Hz".
fn parse_hz(s: &str) -> Option<f64> {
    let t = s.trim().to_ascii_lowercase().replace(' ', "");
    let t = t.strip_suffix("hz").unwrap_or(&t);
    match t.strip_suffix('k') {
        Some(k) => k.parse::<f64>().ok().map(|v| v * 1000.0),
        None => t.parse::<f64>().ok(),
    }
}

/// "4.2 → 0.8 dB" style error text of the last run.
pub fn summary_text(last: &[FitSummary], tr: Tr) -> Option<String> {
    let f = |v: f32| format!("{v:.1}");
    match last {
        [] => None,
        [l, r] => Some(tr.f("autoeq.error_lr", &[&f(l.before_db), &f(l.after_db), &f(r.before_db), &f(r.after_db)])),
        [s, ..] => Some(tr.f("autoeq.error", &[&f(s.before_db), &f(s.after_db)])),
    }
}

/// Draws the section. Returns the actions (file dialogs, options, run, undo).
pub fn auto_eq_section(ui: &mut Ui, v: &AutoEqView<'_>, tr: Tr) -> Vec<UiAction> {
    let pal = theme::palette(ui);
    let mut actions = Vec::new();

    // Header: chevron + title (whole row clickable), last errors on the right when collapsed.
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.hovered() {
        ui.painter().rect_filled(rect, theme::radius::MD, pal.hover);
    }
    let icon = if v.expanded { Icon::ChevronDown } else { Icon::ChevronRight };
    let ir = egui::Rect::from_center_size(egui::pos2(rect.left() + 12.0, rect.center().y), vec2(14.0, 14.0));
    paint_icon(ui.painter(), ir, icon, pal.text_muted);
    let title = ui.painter().layout_no_wrap(tr.t("autoeq.title").to_owned(), FontId::proportional(theme::size::BODY), pal.text);
    let title_w = title.size().x;
    ui.painter().galley(egui::pos2(rect.left() + 26.0, rect.center().y - title.size().y / 2.0), title, pal.text);
    if !v.expanded {
        let note = if v.running { Some(tr.t("autoeq.running").to_owned()) } else { summary_text(v.last, tr) };
        if let Some(note) = note {
            let g = ui.painter().layout_no_wrap(note, FontId::proportional(theme::size::CAPTION), pal.text_muted);
            if rect.left() + 26.0 + title_w + 12.0 + g.size().x < rect.right() - 6.0 {
                ui.painter().galley(egui::pos2(rect.right() - 6.0 - g.size().x, rect.center().y - g.size().y / 2.0), g, pal.text_muted);
            }
        }
    }
    if resp.clicked() {
        actions.push(UiAction::ToggleAutoEqSection);
    }
    if !v.expanded {
        return actions;
    }

    ui.indent("autoeq_body", |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let picking = v.picking.is_some();
        let label_w = [tr.t("autoeq.target"), tr.t("autoeq.source"), tr.t("autoeq.source_l"), tr.t("autoeq.source_r")]
            .iter()
            .map(|t| ui.painter().layout_no_wrap((*t).to_owned(), FontId::proportional(theme::size::BODY), Color32::PLACEHOLDER).size().x)
            .fold(0.0f32, f32::max)
            .min(110.0)
            + 8.0;
        // Narrow: labels on their own line above the controls (no label column).
        let file_btn_w = ui.painter().layout_no_wrap(tr.t("autoeq.choose_file").to_owned(), FontId::proportional(theme::size::BODY), Color32::PLACEHOLDER).size().x + 22.0 + 28.0;
        let stacked = ui.available_width() < label_w + file_btn_w + 140.0 + 3.0 * ui.spacing().item_spacing.x;
        let col = if stacked { 0.0 } else { label_w + ui.spacing().item_spacing.x };
        // target
        let target_name = if v.target_flat { None } else { v.target };
        file_row(ui, &mut actions, FileRow { label: tr.t("autoeq.target"), label_w, stacked, name: target_name, slot: CurveSlot::Target, picking, dim: v.target_flat }, tr);
        ui.horizontal(|ui| {
            ui.add_space(col);
            let mut flat = v.target_flat;
            if ui.checkbox(&mut flat, tr.t("autoeq.flat")).on_hover_text(tr.t("autoeq.flat_hint")).changed() {
                actions.push(UiAction::SetAutoEqTargetFlat(flat));
            }
        });
        ui.add_space(2.0);
        // measurement(s)
        if v.same_source {
            file_row(ui, &mut actions, FileRow { label: tr.t("autoeq.source"), label_w, stacked, name: v.source, slot: CurveSlot::Source, picking, dim: false }, tr);
        } else {
            file_row(ui, &mut actions, FileRow { label: tr.t("autoeq.source_l"), label_w, stacked, name: v.source, slot: CurveSlot::Source, picking, dim: false }, tr);
            file_row(ui, &mut actions, FileRow { label: tr.t("autoeq.source_r"), label_w, stacked, name: v.source_r, slot: CurveSlot::SourceRight, picking, dim: false }, tr);
        }
        ui.horizontal(|ui| {
            ui.add_space(col);
            let mut same = v.same_source;
            if ui.checkbox(&mut same, tr.t("autoeq.same_source")).on_hover_text(tr.t("autoeq.same_source_hint")).changed() {
                actions.push(UiAction::SetAutoEqSameSource(same));
            }
        });
        if !v.same_source && !v.separate {
            ui.horizontal(|ui| {
                ui.add_space(col);
                ui.add(egui::Label::new(theme::muted(ui, tr.t("autoeq.average_note")).size(theme::size::CAPTION)).wrap());
            });
        }
        ui.add_space(6.0);
        // create … errors … undo. The button lines up with the file buttons when there is
        // room; the errors move to their own line when the row is too narrow for them.
        let ready = v.ready();
        let label = if v.running { tr.t("autoeq.running") } else { tr.t("autoeq.run") };
        let hint = if !ready {
            tr.t("autoeq.need_curves")
        } else if v.separate {
            tr.t("autoeq.run_hint_lr")
        } else {
            tr.t("autoeq.run_hint")
        };
        let summary = summary_text(v.last, tr).filter(|_| !v.running);
        let gap = ui.spacing().item_spacing.x;
        let btn_w = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(theme::size::BODY), Color32::PLACEHOLDER).size().x + 22.0 + 28.0;
        let undo_w = if v.can_undo { 25.0 + gap } else { 0.0 };
        let avail = ui.available_width();
        let inset = if col > 0.0 && col + btn_w + 24.0 + undo_w <= avail { col } else { 0.0 };
        let summary_inline = summary.is_some() && inset + btn_w + gap + 120.0 + undo_w <= avail;
        ui.horizontal(|ui| {
            ui.set_min_height(32.0);
            ui.add_space(inset);
            let r = ui.add_enabled_ui(ready && !v.running, |ui| button(ui, ButtonKind::Primary, Some(Icon::Waveform), label)).inner;
            if r.on_hover_text(hint).on_disabled_hover_text(hint).clicked() {
                actions.push(UiAction::RunAutoEq);
            }
            if v.running {
                ui.add(egui::Spinner::new().size(16.0));
            }
            if let Some(text) = summary.as_ref().filter(|_| summary_inline) {
                let w = (ui.available_width() - undo_w).max(0.0);
                ui.allocate_ui_with_layout(vec2(w, 28.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(theme::muted(ui, text).size(theme::size::CAPTION)).truncate()).on_hover_text(tr.t("autoeq.error_hint"));
                });
            }
            if v.can_undo {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icon_button(ui, Icon::Undo, 15.0, false, tr.t("autoeq.undo_hint")).clicked() {
                        actions.push(UiAction::UndoAutoEq);
                    }
                });
            }
        });
        if let Some(text) = summary.filter(|_| !summary_inline) {
            ui.horizontal(|ui| {
                ui.add_space(inset);
                ui.add(egui::Label::new(theme::muted(ui, text).size(theme::size::CAPTION)).wrap()).on_hover_text(tr.t("autoeq.error_hint"));
            });
        }
        ui.add_space(6.0);
        // options
        if let Some(o) = options_grid(ui, v.options, tr) {
            actions.push(UiAction::SetAutoEqOptions(o));
        }
    });
    actions
}

struct FileRow<'a> {
    label: &'a str,
    label_w: f32,
    /// Label on its own line above the controls.
    stacked: bool,
    name: Option<&'a str>,
    slot: CurveSlot,
    picking: bool,
    dim: bool,
}

/// `label [ファイル…] name ×` (label above when stacked).
fn file_row(ui: &mut Ui, actions: &mut Vec<UiAction>, r: FileRow<'_>, tr: Tr) {
    let pal = theme::palette(ui);
    if r.stacked {
        ui.add(egui::Label::new(theme::muted(ui, r.label)).truncate());
    }
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        if !r.stacked {
            ui.allocate_ui_with_layout(vec2(r.label_w, 28.0), Layout::left_to_right(Align::Center), |ui| {
                ui.set_min_width(r.label_w);
                ui.add(egui::Label::new(theme::muted(ui, r.label)).truncate());
            });
        }
        let text = if r.picking { tr.t("autoeq.loading") } else { tr.t("autoeq.choose_file") };
        let b = ui.add_enabled_ui(!r.picking, |ui| button(ui, ButtonKind::Secondary, Some(Icon::Folder), text)).inner;
        if b.on_hover_text(tr.t("autoeq.file_hint")).clicked() {
            actions.push(UiAction::PickAutoEqCurve(r.slot));
        }
        let clear_w = if r.name.is_some() && !r.dim { 26.0 } else { 0.0 };
        let w = (ui.available_width() - clear_w - ui.spacing().item_spacing.x).max(0.0);
        ui.allocate_ui_with_layout(vec2(w, 28.0), Layout::left_to_right(Align::Center), |ui| {
            let (text, color) = match r.name {
                Some(n) if !r.dim => (n, pal.text),
                Some(n) => (n, pal.text_faint),
                None if r.dim => (tr.t("autoeq.flat_name"), pal.text_muted),
                None => (tr.t("autoeq.no_file"), pal.text_faint),
            };
            let resp = ui.add(egui::Label::new(egui::RichText::new(text).color(color)).truncate());
            if let Some(n) = r.name {
                resp.on_hover_text(n);
            }
        });
        if clear_w > 0.0 && icon_button(ui, Icon::Close, 12.0, false, tr.t("autoeq.clear")).clicked() {
            actions.push(UiAction::ClearAutoEqCurve(r.slot));
        }
    });
}

/// The fitting options (two label/value pairs per row when there is room). Returns the new
/// options when one changed.
fn options_grid(ui: &mut Ui, o: AutoEqOptions, tr: Tr) -> Option<AutoEqOptions> {
    let mut n = o;
    let mut changed = false;
    // Two pairs per row need ~430 px (measured with the Japanese labels).
    let two = ui.available_width() >= 445.0;
    fn freq(v: &mut f32) -> egui::DragValue<'_> {
        let speed = (*v as f64 * 0.004).max(0.05);
        egui::DragValue::new(v).range(20.0..=20_000.0).speed(speed).custom_formatter(|x, _| hz_text(x as f32)).custom_parser(parse_hz)
    }
    egui::Grid::new("autoeq_options").num_columns(if two { 4 } else { 2 }).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        let mut cell = 0usize;
        let mut end_cell = |ui: &mut Ui| {
            cell += 1;
            if !two || cell.is_multiple_of(2) {
                ui.end_row();
            }
        };
        // Pairs ordered so the wide values (the two ranges) share a column with short labels.
        // filters
        ui.label(theme::muted(ui, tr.t("autoeq.opt_filters")));
        let mut mf = n.max_filters;
        changed |= ui.add(egui::DragValue::new(&mut mf).range(1..=PEQ_MAX_FILTERS).speed(0.1)).changed();
        n.max_filters = mf;
        end_cell(ui);
        // max gain
        ui.label(theme::muted(ui, tr.t("autoeq.opt_max_gain")));
        changed |= ui.add(egui::DragValue::new(&mut n.max_gain_db).range(1.0..=20.0).speed(0.1).fixed_decimals(1).suffix(" dB")).changed();
        end_cell(ui);
        // Q range
        ui.label(theme::muted(ui, tr.t("autoeq.opt_q")));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            changed |= ui.add(egui::DragValue::new(&mut n.min_q).range(0.1..=10.0).speed(0.01).fixed_decimals(2)).changed();
            ui.label(theme::muted(ui, "–"));
            changed |= ui.add(egui::DragValue::new(&mut n.max_q).range(0.1..=10.0).speed(0.01).fixed_decimals(2)).changed();
        });
        end_cell(ui);
        // treble boost limit
        ui.label(theme::muted(ui, tr.t("autoeq.opt_treble"))).on_hover_text(tr.t("autoeq.opt_treble_hint"));
        changed |= ui
            .add(egui::DragValue::new(&mut n.treble_boost_limit_db).range(0.0..=20.0).speed(0.1).fixed_decimals(1).suffix(" dB"))
            .on_hover_text(tr.t("autoeq.opt_treble_hint"))
            .changed();
        end_cell(ui);
        // frequency range
        ui.label(theme::muted(ui, tr.t("autoeq.opt_freq")));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            changed |= ui.add(freq(&mut n.freq_min_hz)).on_hover_text(tr.t("peq.freq")).changed();
            ui.label(theme::muted(ui, "–"));
            changed |= ui.add(freq(&mut n.freq_max_hz)).on_hover_text(tr.t("peq.freq")).changed();
        });
        end_cell(ui);
        // shelves
        ui.label(theme::muted(ui, tr.t("autoeq.opt_shelves"))).on_hover_text(tr.t("autoeq.opt_shelves_hint"));
        let mut sh = n.use_shelves;
        if toggle_switch(ui, &mut sh).on_hover_text(tr.t("autoeq.opt_shelves_hint")).changed() {
            n.use_shelves = sh;
            changed = true;
        }
        end_cell(ui);
    });
    if changed {
        // Keep the ranges ordered while dragging one end.
        if n.min_q > n.max_q {
            if n.min_q != o.min_q {
                n.max_q = n.min_q;
            } else {
                n.min_q = n.max_q;
            }
        }
        if n.freq_min_hz * 1.42 > n.freq_max_hz {
            if n.freq_min_hz != o.freq_min_hz {
                n.freq_max_hz = (n.freq_min_hz * 1.42).min(20_000.0);
            } else {
                n.freq_min_hz = n.freq_max_hz / 1.42;
            }
        }
        Some(n)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view<'a>(last: &'a [FitSummary]) -> AutoEqView<'a> {
        AutoEqView {
            expanded: true,
            target: None,
            target_flat: false,
            source: None,
            source_r: None,
            same_source: true,
            separate: false,
            options: AutoEqOptions::default(),
            picking: None,
            running: false,
            last,
            can_undo: false,
        }
    }

    #[test]
    fn readiness_and_summary() {
        let mut v = view(&[]);
        assert!(!v.ready());
        v.target_flat = true;
        assert!(!v.ready());
        v.source = Some("l.txt");
        assert!(v.ready());
        v.same_source = false;
        v.separate = true;
        assert!(!v.ready(), "L・R 別 with two files needs R");
        v.source_r = Some("r.txt");
        assert!(v.ready());
        v.separate = false;
        v.source = None;
        assert!(v.ready(), "linked: one of the two files is enough");
        let one = [FitSummary { side: None, before_db: 4.25, after_db: 0.81, filters: 8 }];
        assert_eq!(summary_text(&one, Tr::EN).as_deref(), Some("RMS error 4.2 → 0.8 dB"));
        let two = [
            FitSummary { side: Some(PeqSide::Left), before_db: 4.0, after_db: 0.5, filters: 8 },
            FitSummary { side: Some(PeqSide::Right), before_db: 3.0, after_db: 0.7, filters: 7 },
        ];
        assert!(summary_text(&two, Tr::JA).unwrap().contains("R 3.0 → 0.7"));
        assert_eq!(summary_text(&[], Tr::JA), None);
        assert_eq!(parse_hz("16k"), Some(16_000.0));
    }
}
