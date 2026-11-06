//! Cards, section headers and buttons.

use eframe::egui::{
    self, vec2, Color32, CursorIcon, FontId, Frame, InnerResponse, Margin, Rect, Response, Sense,
    Stroke, Ui, Vec2,
};

use crate::presentation::ui::{
    icons::{paint_icon, Icon},
    theme::{self, radius, space},
};

/// Elevated card: rounded, 1px border, 16px padding.
pub fn card<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let pal = theme::palette(ui);
    card_frame(&pal).show(ui, add_contents)
}

pub fn card_frame(pal: &theme::Palette) -> Frame {
    Frame::new()
        .fill(pal.elevated)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(radius::LG)
        .inner_margin(Margin::same(space::LG as i8))
}

/// Page/section header: title + optional muted subtitle, with right-aligned actions.
pub fn section_header<R>(
    ui: &mut Ui,
    title: &str,
    subtitle: Option<&str>,
    actions: impl FnOnce(&mut Ui) -> R,
) -> R {
    let pal = theme::palette(ui);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(theme::heading(ui, title).color(pal.text));
            if let Some(s) = subtitle {
                ui.label(theme::caption(ui, s));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions)
            .inner
    })
    .inner
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// Accent fill.
    Primary,
    /// Elevated fill with border.
    Secondary,
    /// Transparent until hovered.
    Ghost,
    /// Danger text, danger fill on hover.
    Danger,
}

/// Painted button with optional leading icon (height 32).
pub fn button(ui: &mut Ui, kind: ButtonKind, icon: Option<Icon>, text: &str) -> Response {
    let pal = theme::palette(ui);
    let font = if kind == ButtonKind::Primary {
        theme::bold_font(ui.ctx(), theme::size::BODY)
    } else {
        FontId::proportional(theme::size::BODY)
    };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER);
    let icon_w = if icon.is_some() { 16.0 + 6.0 } else { 0.0 };
    let pad = if text.is_empty() { 8.0 } else { 14.0 };
    let size = vec2(galley.size().x + icon_w + pad * 2.0 - if text.is_empty() { 6.0 } else { 0.0 }, 32.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.1);
        let pressed = resp.is_pointer_button_down_on();
        let (fill, fg, stroke) = match kind {
            ButtonKind::Primary => {
                let f = theme::blend(pal.accent, pal.accent_hover, t);
                (f, pal.accent_text, Stroke::NONE)
            }
            ButtonKind::Secondary => (
                theme::blend(pal.elevated, pal.hover, t),
                pal.text,
                Stroke::new(1.0, theme::blend(pal.border, pal.border_strong, t)),
            ),
            ButtonKind::Ghost => (theme::with_alpha(pal.hover, t), pal.text, Stroke::NONE),
            ButtonKind::Danger => (
                theme::with_alpha(pal.danger, 0.14 * t),
                pal.danger,
                Stroke::new(1.0, theme::with_alpha(pal.danger, 0.35)),
            ),
        };
        let fill = if pressed { theme::blend(fill, pal.pressed, 0.4) } else { fill };
        ui.painter().rect(rect, radius::MD, fill, stroke, egui::StrokeKind::Inside);
        let mut x = rect.left() + pad;
        if let Some(icon) = icon {
            let r = Rect::from_center_size(egui::pos2(x + 8.0, rect.center().y), Vec2::splat(16.0));
            paint_icon(ui.painter(), r, icon, fg);
            x += icon_w;
        }
        ui.painter().galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    }
    resp
}

pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    button(ui, ButtonKind::Primary, None, text)
}

pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    button(ui, ButtonKind::Secondary, None, text)
}

/// Small pill/badge (e.g. counts).
pub fn badge(ui: &mut Ui, text: &str, fill: Color32, fg: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(10.5), fg);
    let size = vec2((galley.size().x + 10.0).max(18.0), 18.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 9.0, fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp
}
