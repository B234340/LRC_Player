//! Settings page helpers: titled section cards with labelled rows, plus a few width-aware
//! controls (segmented control, combo box, slider width, middle-ellipsis text) that shrink to
//! the space they are given. Everything here sizes itself from `ui.available_width()`, so the
//! Settings / Folders pages stay inside the central panel when the right panel is open.

use eframe::egui::{
    self, pos2, vec2, Align, CursorIcon, FontId, Id, Layout, Rect, RichText, Sense, Stroke,
    TextStyle, TextWrapMode, Ui, WidgetText,
};

use crate::presentation::ui::{
    theme::{self, space},
    widgets::{card_frame, segmented},
};

/// Maximum width of a section card (and of the page column holding them).
pub const SECTION_MAX_WIDTH: f32 = 760.0;
/// Rows narrower than this stack: label and hint on top, the control below.
pub const ROW_STACK_BELOW: f32 = 520.0;
/// Controls at most this wide (switches, badges, short values) stay inline even in a
/// stacked-width row: a lone switch under its label reads worse than one beside it.
const INLINE_CONTROL_MAX: f32 = 96.0;
/// Gap between the label column and the control in an inline row.
const ROW_GAP: f32 = 16.0;
/// Narrowest label column an inline row accepts before it stacks.
const MIN_LABEL_WIDTH: f32 = 160.0;

/// A titled card. Put [`settings_row`]s inside. The card fills the available width (callers
/// cap it with [`page_column`]); it never asks for more than it is given.
pub fn settings_section<R>(ui: &mut Ui, title: &str, description: Option<&str>, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let pal = theme::palette(ui);
    ui.label(theme::title(ui, title).color(pal.text));
    if let Some(d) = description {
        ui.label(theme::caption(ui, d));
    }
    ui.add_space(space::SM);
    let r = card_frame(&pal)
        .inner_margin(egui::Margin::symmetric(space::LG as i8, space::XS as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width().max(0.0));
            ui.spacing_mut().item_spacing.y = 0.0;
            add_contents(ui)
        })
        .inner;
    ui.add_space(space::XL);
    r
}

/// Page column for section cards: `min(available, max_width)` wide, never wider than the
/// parent (unlike a bare `set_max_width`, which can grow the ui past its parent).
pub fn page_column<R>(ui: &mut Ui, max_width: f32, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let w = ui.available_width().min(max_width).max(0.0);
    ui.scope(|ui| {
        ui.set_max_width(w);
        add_contents(ui)
    })
    .inner
}

#[derive(Clone, Copy, Default)]
struct RowMemo {
    /// Width the control used last pass.
    control_w: f32,
    /// Height of the label column last pass (inline rows), used to center the control.
    label_h: f32,
}

/// Row with a label (and optional hint) on the left and a right-aligned control. Rows are
/// separated by hairlines; pass `first = true` for the first row of a section.
///
/// Responsive: the label and hint wrap; the control gets at most ~55 % of the row. Below
/// [`ROW_STACK_BELOW`] the row stacks (label and hint on top, control below at full width,
/// laid out left to right with wrapping), unless the control is small enough to stay inline.
/// The control closure should size itself from `ui.available_width()`.
pub fn settings_row<R>(ui: &mut Ui, first: bool, label: &str, hint: Option<&str>, control: impl FnOnce(&mut Ui) -> R) -> R {
    let pal = theme::palette(ui);
    if !first {
        let r = ui.available_rect_before_wrap();
        ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, pal.border));
    }
    let width = ui.available_width().max(0.0);
    let id = ui.make_persistent_id(("settings_row", label, hint));
    let memo: Option<RowMemo> = ui.data(|d| d.get_temp(id));
    let small_control = memo.is_some_and(|m| m.control_w > 0.0 && m.control_w <= INLINE_CONTROL_MAX);
    let stacked = width < ROW_STACK_BELOW && !(small_control && width - memo.map_or(0.0, |m| m.control_w) - ROW_GAP >= MIN_LABEL_WIDTH);
    let pad_top = if hint.is_some() { 10.0 } else { 14.0 };
    let pad_bottom = if hint.is_some() { 11.0 } else { 13.0 };

    let (r, new_memo) = if stacked {
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.add_space(pad_top);
            label_column(ui, &pal, label, hint);
            ui.add_space(space::SM);
            let inner = ui.allocate_ui_with_layout(vec2(width, 32.0), Layout::left_to_right(Align::Center).with_main_wrap(true), |ui| {
                ui.set_max_width(width);
                control(ui)
            });
            ui.add_space(pad_bottom + 1.0);
            (inner.inner, RowMemo { control_w: inner.response.rect.width(), label_h: 0.0 })
        })
        .inner
    } else {
        let min_h = if hint.is_some() { 58.0 } else { 46.0 };
        let row_h = memo.map_or(min_h, |m| m.label_h.max(min_h));
        let control_max = (width * 0.55).min(width - MIN_LABEL_WIDTH - ROW_GAP).max(0.0);
        ui.allocate_ui_with_layout(vec2(width, row_h), Layout::right_to_left(Align::Center), |ui| {
            ui.set_min_size(vec2(width, row_h));
            let ctrl = ui.allocate_ui_with_layout(vec2(control_max, row_h), Layout::right_to_left(Align::Center), |ui| {
                ui.set_max_width(control_max);
                control(ui)
            });
            ui.add_space(ROW_GAP);
            let label_w = ui.available_width().max(0.0);
            let labels = ui.allocate_ui_with_layout(vec2(label_w, row_h), Layout::top_down(Align::Min), |ui| {
                ui.set_width(label_w);
                ui.add_space(pad_top);
                label_column(ui, &pal, label, hint);
                ui.add_space(pad_bottom);
            });
            (ctrl.inner, RowMemo { control_w: ctrl.response.rect.width(), label_h: labels.response.rect.height() })
        })
        .inner
    };

    // The layout depends on last pass's measurements; redo the pass when they changed.
    let changed = memo.is_none_or(|m| (m.control_w - new_memo.control_w).abs() > 0.5 || (m.label_h - new_memo.label_h).abs() > 0.5);
    if changed {
        ui.data_mut(|d| d.insert_temp(id, new_memo));
        ui.ctx().request_discard("settings_row layout");
    }
    r
}

fn label_column(ui: &mut Ui, pal: &theme::Palette, label: &str, hint: Option<&str>) {
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.add(egui::Label::new(RichText::new(label).color(pal.text)).wrap());
    if let Some(h) = hint {
        ui.add(egui::Label::new(theme::caption(ui, h)).wrap());
    }
}

/// Run `add` in a child ui at most `max_width` wide (same layout as `ui`).
pub fn with_max_width<R>(ui: &mut Ui, max_width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let w = ui.available_width().min(max_width).max(0.0);
    let h = ui.spacing().interact_size.y;
    let layout = *ui.layout();
    ui.allocate_ui_with_layout(vec2(w, h), layout, |ui| {
        ui.set_max_width(w);
        add(ui)
    })
    .inner
}

/// Width for a combo box: `preferred` in an inline row (less when the row is narrower), the
/// full width in a stacked row.
pub fn combo_width(ui: &Ui, preferred: f32) -> f32 {
    let avail = ui.available_width();
    let w = if ui.layout().prefer_right_to_left() { preferred } else { avail };
    w.min(avail).max(64.0)
}

/// Whether `text` would be truncated in a combo box `width` wide.
pub fn combo_truncates(ui: &Ui, text: &str, width: f32) -> bool {
    let s = ui.spacing();
    let room = width - 2.0 * s.button_padding.x - s.icon_spacing - s.icon_width;
    let g = WidgetText::from(text).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Button);
    g.size().x > room
}

/// Width for an `egui::Slider` (excluding its value box), shrinking with the row.
pub fn slider_width(ui: &Ui, preferred: f32) -> f32 {
    // The value box (e.g. "30 pt") plus spacing.
    let reserve = 72.0;
    let avail = ui.available_width() - reserve;
    let w = if ui.layout().prefer_right_to_left() { preferred } else { avail };
    w.min(avail).max(48.0)
}

/// Segmented control that fits the available width: the themed widget when it fits,
/// otherwise equal-width segments with truncated labels (full label in a tooltip).
pub fn fit_segmented(ui: &mut Ui, id: Id, options: &[&str], selected: usize) -> Option<usize> {
    let font = FontId::proportional(theme::size::BODY - 0.5);
    let pad = 14.0;
    let natural_seg = options
        .iter()
        .map(|s| ui.painter().layout_no_wrap((*s).to_owned(), font.clone(), egui::Color32::PLACEHOLDER).size().x + pad * 2.0)
        .fold(56.0f32, f32::max);
    let n = options.len().max(1) as f32;
    let avail = ui.available_width();
    if natural_seg * n + 6.0 <= avail {
        return segmented(ui, id, options, selected);
    }
    compact_segmented(ui, id, options, selected, ((avail - 6.0) / n).max(28.0), font)
}

/// Compressed segmented control, painted like `widgets::segmented`.
fn compact_segmented(ui: &mut Ui, id: Id, options: &[&str], selected: usize, seg_w: f32, font: FontId) -> Option<usize> {
    let pal = theme::palette(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(seg_w * options.len() as f32 + 6.0, 30.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, theme::radius::MD, pal.input_bg);
    painter.rect_stroke(rect, theme::radius::MD, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
    let inner = rect.shrink(3.0);
    let target = selected.min(options.len().saturating_sub(1)) as f32;
    let x = ui.ctx().animate_value_with_time(id.with("pill"), target, 0.15);
    let pill = Rect::from_min_size(pos2(inner.left() + x * seg_w, inner.top()), vec2(seg_w, inner.height()));
    painter.add(egui::epaint::Shadow { offset: [0, 1], blur: 4, spread: 0, color: pal.shadow }.as_shape(pill, 6));
    painter.rect_filled(pill, 6.0, if pal.dark { pal.hover } else { pal.elevated });

    let mut changed = None;
    for (i, text) in options.iter().enumerate() {
        let r = Rect::from_min_size(pos2(inner.left() + i as f32 * seg_w, inner.top()), vec2(seg_w, inner.height()));
        let mut resp = ui.interact(r, id.with(i), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let (shown, cut) = truncate_end(ui, text, &font, seg_w - 10.0);
        if cut {
            resp = resp.on_hover_text(*text);
        }
        if resp.clicked() && i != selected {
            changed = Some(i);
        }
        let color = if i == selected {
            pal.text
        } else if resp.hovered() {
            theme::blend(pal.text_muted, pal.text, 0.5)
        } else {
            pal.text_muted
        };
        let g = painter.layout_no_wrap(shown, font.clone(), color);
        painter.galley(r.center() - g.size() / 2.0, g, color);
    }
    changed
}

fn text_width(ui: &Ui, text: &str, font: &FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER).size().x
}

/// `text` cut at the end with "…" to fit `max_w`; `true` when it was cut.
fn truncate_end(ui: &Ui, text: &str, font: &FontId, max_w: f32) -> (String, bool) {
    if text_width(ui, text, font) <= max_w {
        return (text.to_owned(), false);
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let s: String = chars[..mid].iter().chain(std::iter::once(&'…')).collect();
        if text_width(ui, &s, font) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    (chars[..lo].iter().chain(std::iter::once(&'…')).collect(), true)
}

/// `text` shortened in the middle ("C:\Users\…\Music\Albums") to fit `max_w`; `true` when
/// it was shortened. Keeps a little more of the end (the most specific part of a path).
pub fn middle_ellipsis(ui: &Ui, text: &str, font: &FontId, max_w: f32) -> (String, bool) {
    if text_width(ui, text, font) <= max_w {
        return (text.to_owned(), false);
    }
    let chars: Vec<char> = text.chars().collect();
    let build = |keep: usize| -> String {
        let head = keep * 2 / 5;
        let tail = keep - head;
        let mut s: String = chars[..head].iter().collect();
        s.push('…');
        s.extend(&chars[chars.len() - tail..]);
        s
    };
    let (mut lo, mut hi) = (0usize, chars.len().saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if text_width(ui, &build(mid), font) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    (build(lo), true)
}
