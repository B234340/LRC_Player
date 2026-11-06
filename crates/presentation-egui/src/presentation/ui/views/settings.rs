//! Settings page helpers: titled section cards with labelled rows.

use eframe::egui::{self, Align, Layout, RichText, Stroke, Ui};

use crate::presentation::ui::{
    theme::{self, space},
    widgets::card::card_frame,
};

/// A titled card. Put [`settings_row`]s inside.
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
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add_contents(ui)
        })
        .inner;
    ui.add_space(space::XL);
    r
}

/// Row with a label (and optional hint) on the left and a right-aligned control. Rows are
/// separated by hairlines; pass `first = true` for the first row of a section.
pub fn settings_row<R>(ui: &mut Ui, first: bool, label: &str, hint: Option<&str>, control: impl FnOnce(&mut Ui) -> R) -> R {
    let pal = theme::palette(ui);
    if !first {
        let r = ui.available_rect_before_wrap();
        ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, pal.border));
    }
    let min_h = if hint.is_some() { 58.0 } else { 46.0 };
    ui.horizontal(|ui| {
        ui.set_min_height(min_h);
        ui.vertical(|ui| {
            ui.add_space(if hint.is_some() { 10.0 } else { 14.0 });
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(label).color(pal.text));
            if let Some(h) = hint {
                ui.label(theme::caption(ui, h));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control).inner
    })
    .inner
}
