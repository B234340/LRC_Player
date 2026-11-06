//! Centered empty state: large muted icon, title, hint and an optional primary action.

use eframe::egui::{self, vec2, Align, Layout, Rect, RichText, Sense, Ui, Vec2};

use super::card::{button, ButtonKind};
use crate::presentation::ui::{
    icons::{paint_icon, Icon},
    theme,
};

/// Draws the empty state in the available space. Returns `true` when the action button
/// was clicked.
pub fn empty_state(ui: &mut Ui, icon: Icon, title: &str, hint: &str, action: Option<(Icon, &str)>) -> bool {
    let pal = theme::palette(ui);
    let avail = ui.available_size();
    let content_h = 64.0 + 16.0 + 26.0 + 40.0 + if action.is_some() { 48.0 } else { 0.0 };
    let top = ((avail.y - content_h) * 0.42).max(8.0);
    let mut clicked = false;
    ui.allocate_ui_with_layout(vec2(avail.x, avail.y.max(content_h)), Layout::top_down(Align::Center), |ui| {
        ui.add_space(top);
        let (r, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 32.0, theme::with_alpha(pal.text, 0.06));
        paint_icon(ui.painter(), Rect::from_center_size(r.center(), Vec2::splat(30.0)), icon, pal.text_faint);
        ui.add_space(12.0);
        ui.label(theme::title(ui, title).color(pal.text));
        ui.add_space(2.0);
        ui.add(egui::Label::new(RichText::new(hint).color(pal.text_muted)).wrap());
        if let Some((icon, label)) = action {
            ui.add_space(14.0);
            clicked = button(ui, ButtonKind::Primary, Some(icon), label).clicked();
        }
    });
    clicked
}
