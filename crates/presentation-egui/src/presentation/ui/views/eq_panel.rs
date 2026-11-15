//! EQ popover contents: graphic (10-band: enable switch, presets, preamp, vertical band
//! sliders) and parametric (L+R / L・R 別, filter list, AutoEq import, automatic creation from
//! a target and a measurement — `eq_auto.rs`) tabs over a shared frequency-response graph
//! (`eq_graph.rs`).

use eframe::egui::{
    self, pos2, vec2, Align, Color32, CursorIcon, FontId, Id, Layout, Rect, Sense, Stroke, Ui,
};

use crate::presentation::ui::{
    actions::{EqSettings, ParametricEq, PeqChannelMode, PeqSide, UiAction},
    i18n::Tr,
    icons::{icon_button, Icon},
    theme,
    views::{
        eq_auto::{auto_eq_section, AutoEqView},
        eq_graph::{hz_text, response_graph, EqCurve},
        settings::{fit_segmented, with_max_width},
    },
    widgets::{button, slider::wheel_notches, toggle_switch, ButtonKind},
};
use lrc_player_application::infrastructure::audio::engine::{
    FilterKind, PeqFilter, EQ_FREQUENCIES_HZ, EQ_MAX_GAIN_DB, PEQ_MAX_FILTERS, PEQ_MAX_GAIN_DB,
};

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

/// Tabs of the EQ popover.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EqTab {
    /// The 10-band graphic EQ.
    #[default]
    Graphic,
    /// The parametric EQ (with AutoEq import).
    Parametric,
}

pub struct EqPanelProps<'a> {
    pub settings: EqSettings,
    pub peq: &'a ParametricEq,
    pub tab: EqTab,
    /// Cached response curve (`None` draws the empty grid).
    pub curve: Option<&'a EqCurve>,
    /// Height the panel may use (the filter list scrolls within it).
    pub max_height: f32,
    /// The AutoEq file dialog is open.
    pub importing: bool,
    /// PEQ channel shown / edited (`Left` when linked).
    pub side: PeqSide,
    /// The 自動作成 section.
    pub auto: AutoEqView<'a>,
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

/// Draw the EQ popover contents: a title with the グラフィック / パラメトリック tabs, the
/// combined frequency-response graph and the selected tab. Emits `SetEq` /
/// `SetParametricEq` with the full new settings on every change, `SetEqTab` on a tab switch
/// and the AutoEq import actions.
///
/// Layout (fits any width): header (title … tabs), graph, then for the graphic tab the
/// enable / preset / reset row and the slider block (columns compress, below ~340 px it
/// scrolls horizontally); for the parametric tab the L+R / L・R 別 row (with the L / R
/// selector and the copy menu; two rows when narrow), the enable / preamp row, the import
/// buttons (as many rows as needed), the collapsible 自動作成 section, the filter list (rows
/// stack below ~446 px) and "add filter" … "remove all". With the 自動作成 section collapsed
/// only the filter list scrolls (inside `max_height`); expanded, the whole tab body scrolls.
pub fn eq_panel(ui: &mut Ui, props: &EqPanelProps<'_>) -> Vec<UiAction> {
    egui::Frame::new().inner_margin(PANEL_PADDING).show(ui, |ui| eq_panel_contents(ui, props)).inner
}

fn eq_panel_contents(ui: &mut Ui, props: &EqPanelProps<'_>) -> Vec<UiAction> {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let mut actions = Vec::new();
    let top = ui.cursor().top();

    // header: title … tabs
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        ui.label(theme::title(ui, tr.t("eq.title")).color(pal.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let labels = [tr.t("eq.tab_graphic"), tr.t("eq.tab_parametric")];
            let current = usize::from(props.tab == EqTab::Parametric);
            let w = ui.available_width();
            if let Some(i) = with_max_width(ui, w, |ui| fit_segmented(ui, Id::new("eq_tabs"), &labels, current)) {
                actions.push(UiAction::SetEqTab(if i == 0 { EqTab::Graphic } else { EqTab::Parametric }));
            }
        });
    });
    ui.add_space(8.0);
    let overlay = props.curve.is_some_and(|c| c.overlay.is_some());
    let graph_h = match (props.max_height < 470.0, overlay) {
        (true, false) => 76.0,
        (true, true) => 104.0,
        (false, false) => 112.0,
        (false, true) => 150.0,
    };
    response_graph(ui, props.curve, graph_h, tr);
    ui.add_space(10.0);

    match props.tab {
        EqTab::Graphic => {
            if let Some(s) = graphic_tab(ui, props.settings, tr) {
                actions.push(UiAction::SetEq(s));
            }
        }
        EqTab::Parametric => {
            let used = ui.cursor().top() - top;
            actions.extend(parametric_tab(ui, props, (props.max_height - used).max(0.0)));
        }
    }
    actions
}

/// "Enabled" switch with its label (left to right).
fn enable_switch(ui: &mut Ui, on: &mut bool, tr: Tr) -> bool {
    let changed = toggle_switch(ui, on).on_hover_text(tr.t("eq.enabled")).changed();
    ui.label(theme::muted(ui, tr.t("eq.enabled")));
    changed
}

fn graphic_tab(ui: &mut Ui, settings: EqSettings, tr: Tr) -> Option<EqSettings> {
    let mut s = settings;
    let mut changed = false;

    // switch, preset combo … reset. The combo shrinks so the reset button always fits.
    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        let mut on = s.enabled;
        if enable_switch(ui, &mut on, tr) {
            s.enabled = on;
            changed = true;
        }
        ui.add_space(10.0);
        let reset_label = tr.t("eq.reset");
        // Width of the ghost button with icon (see `widgets::button`): text + icon + padding.
        let reset_w = ui.painter().layout_no_wrap(reset_label.to_owned(), FontId::proportional(theme::size::BODY), Color32::PLACEHOLDER).size().x + 22.0 + 28.0;
        if ui.available_width() > reset_w + 72.0 + 90.0 {
            ui.label(theme::muted(ui, tr.t("eq.preset")));
        }
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
        Some(s)
    } else {
        None
    }
}

// ----- parametric tab -------------------------------------------------------------------------

/// Filter types in combo order, with their label keys.
pub const PEQ_KINDS: [(FilterKind, &str); 5] = [
    (FilterKind::Peak, "peq.kind.peak"),
    (FilterKind::LowShelf, "peq.kind.low_shelf"),
    (FilterKind::HighShelf, "peq.kind.high_shelf"),
    (FilterKind::LowPass, "peq.kind.low_pass"),
    (FilterKind::HighPass, "peq.kind.high_pass"),
];

fn kind_label(kind: FilterKind, tr: Tr) -> &'static str {
    PEQ_KINDS.iter().find(|(k, _)| *k == kind).map(|(_, key)| tr.t(key)).unwrap_or("?")
}

fn is_pass(kind: FilterKind) -> bool {
    matches!(kind, FilterKind::LowPass | FilterKind::HighPass)
}

/// A new filter: a neutral 1 kHz peak (Q 1).
pub fn new_filter() -> PeqFilter {
    PeqFilter { kind: FilterKind::Peak, freq_hz: 1000.0, gain_db: 0.0, q: 1.0 }
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

/// Paste events meant for the PEQ: after the paste button (`awaiting`), or Ctrl+V of an
/// AutoEq-looking text while no text field has focus.
fn pasted_profile(ui: &Ui, awaiting: bool) -> Option<String> {
    let focused = ui.ctx().memory(|m| m.focused().is_some());
    ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Paste(text) if awaiting || (!focused && text.to_ascii_lowercase().contains("filter")) => Some(text.clone()),
            _ => None,
        })
    })
}

fn parametric_tab(ui: &mut Ui, props: &EqPanelProps<'_>, max_height: f32) -> Vec<UiAction> {
    let mut actions = Vec::new();
    // Clipboard paste (button -> RequestPaste -> Event::Paste, or plain Ctrl+V).
    let await_id = Id::new("peq_await_paste");
    let awaiting = ui.data(|d| d.get_temp::<f64>(await_id)).is_some_and(|t| ui.input(|i| i.time) - t < 2.0);
    if let Some(text) = pasted_profile(ui, awaiting) {
        ui.data_mut(|d| d.remove::<f64>(await_id));
        actions.push(UiAction::ImportAutoEqText(text));
    }
    if props.auto.expanded {
        ui.scope(|ui| {
            // A solid (always shown) scroll bar: the theme's floating bars are invisible until
            // hovered, and here it must be obvious that more follows below.
            let mut scroll = egui::style::ScrollStyle::solid();
            scroll.bar_width = 5.0;
            scroll.bar_inner_margin = 6.0;
            ui.spacing_mut().scroll = scroll;
            egui::ScrollArea::vertical()
                .id_salt("peq_tab_scroll")
                .max_height(max_height.max(120.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    actions.extend(parametric_body(ui, props, None, await_id));
                });
        });
    } else {
        actions.extend(parametric_body(ui, props, Some(max_height), await_id));
    }
    actions
}

/// The parametric tab below the graph. `list_room`: total height for the tab (the filter list
/// scrolls to fit it); `None` = no inner scrolling (the caller scrolls).
fn parametric_body(ui: &mut Ui, props: &EqPanelProps<'_>, list_room: Option<f32>, await_id: Id) -> Vec<UiAction> {
    let tr = props.tr;
    let pal = theme::palette(ui);
    let mut peq = props.peq.clone();
    let separate = peq.channel_mode == PeqChannelMode::Separate;
    let side = if separate { props.side } else { PeqSide::Left };
    let mut ch = peq.channel(side);
    let mut changed = false;
    let mut actions = Vec::new();
    let start = ui.cursor().top();

    // switch, L+R / L・R 別, (L / R, copy) and the edited channel's preamp
    let mut enabled = peq.enabled;
    let mut pre = ch.preamp_db;
    let head = HeaderState { separate, side, tr };
    let (en_changed, pre_changed) = header_rows(ui, &head, &mut enabled, &mut pre, &mut actions);
    if en_changed {
        peq.enabled = enabled;
        changed = true;
    }
    if pre_changed {
        ch.preamp_db = pre;
        changed = true;
    }
    ui.add_space(6.0);
    // import / paste, packed into as few rows as fit (a wrapping layout does not
    // wrap the enabled/disabled scopes the buttons need).
    let import_label = if props.importing { tr.t("peq.importing") } else { tr.t("peq.import_file") };
    let buttons: [(Option<Icon>, &str); 2] = [(Some(Icon::Folder), import_label), (None, tr.t("peq.paste"))];
    let gap = 6.0;
    let avail = ui.available_width();
    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
    let mut x = 0.0;
    for (i, (icon, text)) in buttons.iter().enumerate() {
        let w = button_width(ui, text, icon.is_some());
        if x > 0.0 && x + gap + w > avail {
            rows.push(Vec::new());
            x = 0.0;
        }
        x += if x > 0.0 { gap + w } else { w };
        rows.last_mut().expect("non-empty").push(i);
    }
    for (r, row) in rows.iter().enumerate() {
        if r > 0 {
            ui.add_space(gap);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for &i in row {
                let (icon, text) = buttons[i];
                match i {
                    0 => {
                        let r = ui.add_enabled_ui(!props.importing, |ui| button(ui, ButtonKind::Secondary, icon, text)).inner;
                        if r.on_hover_text(tr.t("peq.import_hint")).clicked() {
                            actions.push(UiAction::ImportAutoEqFile);
                        }
                    }
                    _ => {
                        if button(ui, ButtonKind::Secondary, icon, text).on_hover_text(tr.t("peq.paste_hint")).clicked() {
                            ui.data_mut(|d| d.insert_temp(await_id, ui.input(|i| i.time)));
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                        }
                    }
                }
            }
        });
    }
    ui.add_space(6.0);
    // 自動作成（ターゲットに合わせる）
    actions.extend(auto_eq_section(ui, &props.auto, tr));

    ui.add_space(6.0);
    // filter list
    let footer_h = 56.0;
    if ch.filters.is_empty() {
        ui.add(egui::Label::new(theme::muted(ui, tr.t("peq.empty"))).wrap());
    } else {
        let mut remove = None;
        let mut rows_ui = |ui: &mut Ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for (i, f) in ch.filters.iter_mut().enumerate() {
                let (c, r) = filter_row(ui, i, f, tr, &pal);
                changed |= c;
                if r {
                    remove = Some(i);
                }
            }
        };
        match list_room {
            Some(room) => {
                let list_max = (room - (ui.cursor().top() - start) - footer_h).max(84.0);
                egui::ScrollArea::vertical()
                    .id_salt("peq_filters")
                    .max_height(list_max)
                    .auto_shrink([false, true])
                    .show(ui, rows_ui);
            }
            None => rows_ui(ui),
        }
        if let Some(i) = remove {
            ch.filters.remove(i);
            changed = true;
        }
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        let full = ch.filters.len() >= PEQ_MAX_FILTERS;
        let add = ui.add_enabled_ui(!full, |ui| button(ui, ButtonKind::Secondary, Some(Icon::Plus), tr.t("peq.add"))).inner;
        if add.clicked() {
            ch.filters.push(new_filter());
            if !peq.enabled {
                peq.enabled = true;
            }
            changed = true;
        }
        ui.label(theme::muted(ui, format!("{} / {}", ch.filters.len(), PEQ_MAX_FILTERS)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let clear = ui.add_enabled_ui(!ch.filters.is_empty(), |ui| button(ui, ButtonKind::Ghost, Some(Icon::Trash), tr.t("peq.clear"))).inner;
            if clear.clicked() {
                ch.filters.clear();
                changed = true;
            }
        });
    });

    if changed {
        peq.set_channel(side, ch);
        actions.push(UiAction::SetParametricEq(peq));
    }
    actions
}

fn side_label(side: PeqSide) -> &'static str {
    match side {
        PeqSide::Left => "L",
        PeqSide::Right => "R",
    }
}

struct HeaderState {
    separate: bool,
    side: PeqSide,
    tr: Tr,
}

/// The rows above the import buttons, packed by width:
///
/// * wide, L+R: `● 有効 [L+R（共通）| L・R 別] … プリアンプ [x dB]` (one row);
/// * wide, L・R 別: `● 有効 [L+R（共通）| L・R 別]` / `編集 [L|R] [コピー] … プリアンプ（R） [x dB]`;
/// * narrow: the mode selector, the L / R row (L・R 別), then `● 有効 … プリアンプ [x dB]`.
///
/// Returns (enabled changed, preamp changed).
fn header_rows(ui: &mut Ui, h: &HeaderState, enabled: &mut bool, pre: &mut f32, actions: &mut Vec<UiAction>) -> (bool, bool) {
    let tr = h.tr;
    let gap = 12.0;
    let modes = [tr.t("peq.mode_linked"), tr.t("peq.mode_separate")];
    let font = FontId::proportional(theme::size::BODY - 0.5);
    let text_w = |t: &str, f: &FontId| ui.painter().layout_no_wrap(t.to_owned(), f.clone(), Color32::PLACEHOLDER).size().x;
    let seg_w = |labels: &[&str]| labels.iter().map(|t| text_w(t, &font) + 28.0).fold(56.0f32, f32::max) * labels.len() as f32 + 6.0;
    let body = FontId::proportional(theme::size::BODY);
    let switch_w = 40.0 + 8.0 + text_w(tr.t("eq.enabled"), &body);
    let pre_label = if h.separate { tr.f("peq.preamp_side", &[&side_label(h.side)]) } else { tr.t("eq.preamp").to_owned() };
    let pre_w = text_w(&pre_label, &body) + 8.0 + 72.0;
    let side_w = text_w(tr.t("peq.side_hint"), &body) + 8.0 + seg_w(&["L", "R"]) + 8.0 + button_width(ui, tr.t("peq.copy"), false);
    let mode_w = seg_w(&modes);
    let avail = ui.available_width();
    let (mut en_changed, mut pre_changed) = (false, false);

    let mut switch_ui = |ui: &mut Ui| {
        let mut on = *enabled;
        if enable_switch(ui, &mut on, tr) {
            *enabled = on;
            en_changed = true;
        }
    };
    let mut preamp_ui = |ui: &mut Ui| {
        // Right to left: value, then its label.
        let mut v = *pre;
        let dv = egui::DragValue::new(&mut v).range(-PEQ_MAX_GAIN_DB..=PEQ_MAX_GAIN_DB).speed(0.05).fixed_decimals(1).suffix(" dB");
        if ui.add(dv).on_hover_text(tr.t("peq.preamp_hint")).changed() {
            *pre = v;
            pre_changed = true;
        }
        ui.label(theme::muted(ui, pre_label.clone()));
    };
    let mode_ui = |ui: &mut Ui, actions: &mut Vec<UiAction>| {
        let w = ui.available_width().min(mode_w);
        let r = ui.scope(|ui| with_max_width(ui, w, |ui| fit_segmented(ui, Id::new("peq_channel_mode"), &modes, usize::from(h.separate))));
        r.response.on_hover_text(tr.t("peq.mode_hint"));
        if let Some(i) = r.inner {
            actions.push(UiAction::SetPeqChannelMode(if i == 0 { PeqChannelMode::Linked } else { PeqChannelMode::Separate }));
        }
    };
    let side_ui = |ui: &mut Ui, actions: &mut Vec<UiAction>| {
        ui.label(theme::muted(ui, tr.t("peq.side_hint")));
        let cur = usize::from(h.side == PeqSide::Right);
        if let Some(i) = crate::presentation::ui::widgets::segmented(ui, Id::new("peq_edit_side"), &["L", "R"], cur) {
            actions.push(UiAction::SetPeqEditSide(if i == 0 { PeqSide::Left } else { PeqSide::Right }));
        }
        ui.add_space(4.0);
        ui.menu_button(tr.t("peq.copy"), |ui| {
            if ui.button(tr.t("peq.copy_l_to_r")).clicked() {
                actions.push(UiAction::CopyPeqChannel(PeqSide::Left));
                ui.close();
            }
            if ui.button(tr.t("peq.copy_r_to_l")).clicked() {
                actions.push(UiAction::CopyPeqChannel(PeqSide::Right));
                ui.close();
            }
        })
        .response
        .on_hover_text(tr.t("peq.copy_hint"));
    };
    let row = |ui: &mut Ui, add: &mut dyn FnMut(&mut Ui)| {
        ui.horizontal(|ui| {
            ui.set_min_height(32.0);
            add(ui);
        });
    };

    if !h.separate && switch_w + gap + mode_w + gap + pre_w <= avail {
        row(ui, &mut |ui| {
            switch_ui(ui);
            ui.add_space(gap);
            mode_ui(ui, actions);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| preamp_ui(ui));
        });
    } else if h.separate && switch_w + gap + mode_w <= avail && side_w + gap + pre_w <= avail {
        row(ui, &mut |ui| {
            switch_ui(ui);
            ui.add_space(gap);
            mode_ui(ui, actions);
        });
        ui.add_space(2.0);
        row(ui, &mut |ui| {
            side_ui(ui, actions);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| preamp_ui(ui));
        });
    } else {
        row(ui, &mut |ui| mode_ui(ui, actions));
        if h.separate {
            ui.add_space(2.0);
            row(ui, &mut |ui| side_ui(ui, actions));
        }
        ui.add_space(2.0);
        row(ui, &mut |ui| {
            switch_ui(ui);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| preamp_ui(ui));
        });
    }
    (en_changed, pre_changed)
}

/// Width of a [`button`] with this label (see `widgets::button`).
fn button_width(ui: &Ui, text: &str, icon: bool) -> f32 {
    let w = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(theme::size::BODY), Color32::PLACEHOLDER).size().x;
    w + if icon { 22.0 } else { 0.0 } + 28.0
}

/// One filter row. Returns (changed, remove clicked).
fn filter_row(ui: &mut Ui, i: usize, f: &mut PeqFilter, tr: Tr, pal: &theme::Palette) -> (bool, bool) {
    let mut changed = false;
    let mut remove = false;
    let avail = ui.available_width();
    let wide = avail >= 446.0;
    let index = |ui: &mut Ui| {
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 24.0), Sense::hover());
        let g = ui.painter().layout_no_wrap(format!("{}", i + 1), FontId::proportional(11.0), pal.text_faint);
        ui.painter().galley(pos2(r.right() - g.size().x, r.center().y - g.size().y / 2.0), g, pal.text_faint);
    };
    let kind_combo = |ui: &mut Ui, f: &mut PeqFilter, w: f32| {
        let mut c = false;
        egui::ComboBox::from_id_salt(("peq_kind", i))
            .selected_text(kind_label(f.kind, tr))
            .width(w)
            .truncate()
            .show_ui(ui, |ui| {
                for (k, key) in PEQ_KINDS {
                    if ui.selectable_label(f.kind == k, tr.t(key)).clicked() && f.kind != k {
                        f.kind = k;
                        c = true;
                    }
                }
            });
        c
    };
    let values = |ui: &mut Ui, f: &mut PeqFilter, w: f32| {
        let mut c = false;
        let speed = (f.freq_hz as f64 * 0.004).max(0.05);
        let freq = egui::DragValue::new(&mut f.freq_hz)
            .range(20.0..=20_000.0)
            .speed(speed)
            .custom_formatter(|v, _| hz_text(v as f32))
            .custom_parser(parse_hz);
        c |= ui.add_sized(vec2(w * 1.25, 24.0), freq).on_hover_text(tr.t("peq.freq")).changed();
        let pass = is_pass(f.kind);
        let gain = egui::DragValue::new(&mut f.gain_db).range(-PEQ_MAX_GAIN_DB..=PEQ_MAX_GAIN_DB).speed(0.05).fixed_decimals(1).suffix(" dB");
        let tip = if pass { tr.t("peq.gain_pass") } else { tr.t("peq.gain") };
        c |= ui.add_enabled_ui(!pass, |ui| ui.add_sized(vec2(w, 24.0), gain)).inner.on_hover_text(tip).on_disabled_hover_text(tip).changed();
        let q = egui::DragValue::new(&mut f.q).range(0.1..=10.0).speed(0.01).fixed_decimals(2).prefix("Q ");
        c |= ui.add_sized(vec2(w * 0.85, 24.0), q).on_hover_text(tr.t("peq.q")).changed();
        c
    };
    let remove_btn = |ui: &mut Ui| icon_button(ui, Icon::Close, 13.0, false, tr.t("peq.remove")).clicked();

    if wide {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            index(ui);
            changed |= kind_combo(ui, f, 128.0);
            let w = ((avail - 18.0 - 134.0 - 30.0 - 5.0 * 6.0) / 3.1).clamp(56.0, 84.0);
            changed |= values(ui, f, w);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| remove = remove_btn(ui));
        });
    } else {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            index(ui);
            let w = (ui.available_width() - 30.0 - 6.0).clamp(80.0, 160.0);
            changed |= kind_combo(ui, f, w);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| remove = remove_btn(ui));
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add_space(24.0);
            let w = ((ui.available_width() - 12.0) / 3.1).max(48.0);
            changed |= values(ui, f, w);
        });
        ui.add_space(2.0);
    }
    (changed, remove)
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
        assert_eq!(parse_hz("1.2k"), Some(1200.0));
        assert_eq!(parse_hz("1.5 kHz"), Some(1500.0));
        assert_eq!(parse_hz("850 Hz"), Some(850.0));
        assert_eq!(parse_hz("abc"), None);
        assert!(is_pass(FilterKind::HighPass) && !is_pass(FilterKind::LowShelf));
        assert_eq!(PEQ_KINDS.len(), 5);
    }

    /// The parametric tab never grows wider than it is given (both channel modes, the
    /// 自動作成 section open and closed, long file names), from the narrowest popover up.
    #[test]
    fn parametric_tab_fits_its_width() {
        use crate::presentation::ui::views::eq_auto::FitSummary;
        let ctx = egui::Context::default();
        let last = [
            FitSummary { side: Some(PeqSide::Left), before_db: 2.7, after_db: 0.3, filters: 9 },
            FitSummary { side: Some(PeqSide::Right), before_db: 2.9, after_db: 0.4, filters: 9 },
        ];
        for mode in [PeqChannelMode::Linked, PeqChannelMode::Separate] {
            let peq = ParametricEq { enabled: true, filters: vec![new_filter(); 3], channel_mode: mode, ..Default::default() };
            for w in [288.0f32, 380.0, 464.0, 508.0] {
                for expanded in [false, true] {
                    let mut width = 0.0;
                    // A few frames: some widgets size themselves from the previous frame.
                    for _ in 0..3 {
                        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 1400.0))), ..Default::default() };
                        let _ = ctx.run_ui(input, |ui| {
                            egui::Area::new(Id::new(("fit", w as i32, expanded, mode == PeqChannelMode::Separate))).show(ui.ctx(), |ui| {
                                ui.set_width(w);
                                let props = EqPanelProps {
                                    settings: EqSettings::default(),
                                    peq: &peq,
                                    tab: EqTab::Parametric,
                                    curve: None,
                                    max_height: 500.0,
                                    importing: false,
                                    side: PeqSide::Right,
                                    auto: AutoEqView {
                                        expanded,
                                        target: Some("IEM_target_harman2019_with_a_long_name.csv"),
                                        source: Some("DIY-IEM_L_2026-11-14_measurement.txt"),
                                        source_r: Some("DIY-IEM_R_2026-11-14_measurement.txt"),
                                        same_source: false,
                                        separate: mode == PeqChannelMode::Separate,
                                        last: &last,
                                        can_undo: true,
                                        ..Default::default()
                                    },
                                    tr: Tr::JA,
                                };
                                width = ui.scope(|ui| eq_panel(ui, &props)).response.rect.width();
                            });
                        });
                    }
                    assert!(width <= w + 1.0, "{mode:?} width {w} expanded {expanded}: {width}");
                }
            }
        }
    }
}
