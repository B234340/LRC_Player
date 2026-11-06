//! Toggle switch and segmented control.

use eframe::egui::{
    self, pos2, vec2, CursorIcon, FontId, Id, Rect, Response, Sense, Stroke, Ui, WidgetInfo,
    WidgetType,
};

use crate::presentation::ui::theme;

/// iOS-style switch. Returns the response; `*on` is flipped when clicked.
pub fn toggle_switch(ui: &mut Ui, on: &mut bool) -> Response {
    let pal = theme::palette(ui);
    let size = vec2(36.0, 20.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *on, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.15);
        let off_fill = if pal.dark { pal.pressed } else { pal.border_strong };
        let mut fill = theme::blend(off_fill, pal.accent, t);
        if resp.hovered() {
            fill = if pal.dark { theme::lighten(fill, 0.06) } else { theme::darken(fill, 0.05) };
        }
        let painter = ui.painter();
        painter.rect_filled(rect, rect.height() / 2.0, fill);
        let r = rect.height() / 2.0 - 2.5;
        let cx = egui::lerp((rect.left() + r + 2.5)..=(rect.right() - r - 2.5), t);
        painter.circle_filled(pos2(cx, rect.center().y + 0.6), r + 0.4, egui::Color32::from_black_alpha(40));
        painter.circle_filled(pos2(cx, rect.center().y), r, egui::Color32::WHITE);
    }
    resp
}

/// Segmented control. Returns `Some(index)` when the selection changes.
pub fn segmented(ui: &mut Ui, id: Id, options: &[&str], selected: usize) -> Option<usize> {
    let pal = theme::palette(ui);
    let font = FontId::proportional(theme::size::BODY - 0.5);
    let galleys: Vec<_> = options
        .iter()
        .map(|s| ui.painter().layout_no_wrap((*s).to_owned(), font.clone(), egui::Color32::PLACEHOLDER))
        .collect();
    let pad = 14.0;
    let seg_w = galleys.iter().map(|g| g.size().x + pad * 2.0).fold(56.0f32, f32::max);
    let height = 30.0;
    let (rect, _) = ui.allocate_exact_size(vec2(seg_w * options.len() as f32 + 6.0, height), Sense::hover());
    let mut changed = None;
    let painter = ui.painter().clone();
    painter.rect_filled(rect, theme::radius::MD, pal.input_bg);
    painter.rect_stroke(rect, theme::radius::MD, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
    let inner = rect.shrink(3.0);

    // animated pill
    let target = selected.min(options.len().saturating_sub(1)) as f32;
    let x = ui.ctx().animate_value_with_time(id.with("pill"), target, 0.15);
    let pill = Rect::from_min_size(pos2(inner.left() + x * seg_w, inner.top()), vec2(seg_w, inner.height()));
    painter.add(egui::epaint::Shadow { offset: [0, 1], blur: 4, spread: 0, color: pal.shadow }.as_shape(pill, 6));
    painter.rect_filled(pill, 6.0, if pal.dark { pal.hover } else { pal.elevated });

    for (i, galley) in galleys.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(inner.left() + i as f32 * seg_w, inner.top()), vec2(seg_w, inner.height()));
        let resp = ui
            .interact(r, id.with(i), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
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
        painter.galley(r.center() - galley.size() / 2.0, galley, color);
    }
    changed
}
