//! Search field with a leading icon, clear button and Esc-to-clear.

use eframe::egui::{
    self, vec2, Color32, CursorIcon, Id, Key, Margin, Rect, Response, Sense, Stroke, TextEdit, Ui,
};

use crate::presentation::ui::{
    i18n::Tr,
    icons::{icon_button_ex, paint_icon, Icon, IconButtonStyle},
    theme,
};

/// Output of [`search_box`].
pub struct SearchBoxOutput {
    /// The text-edit response (use `.has_focus()`, `.request_focus()`).
    pub response: Response,
    /// The query changed this frame (typed, cleared).
    pub changed: bool,
}

/// Search box of `width`. Pass `focus = true` to grab keyboard focus (e.g. on Ctrl+F).
pub fn search_box(
    ui: &mut Ui,
    id: Id,
    query: &mut String,
    width: f32,
    focus: bool,
    tr: &Tr,
) -> SearchBoxOutput {
    let pal = theme::palette(ui);
    let height = 32.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let edit_id = id.with("edit");
    let has_focus = ui.memory(|m| m.has_focus(edit_id));
    let hovered = ui.rect_contains_pointer(rect);

    let fill = if has_focus { pal.input_bg } else if hovered { theme::blend(pal.input_bg, pal.hover, 0.5) } else { pal.input_bg };
    let stroke = if has_focus {
        Stroke::new(1.5, pal.accent)
    } else {
        Stroke::new(1.0, if hovered { pal.border_strong } else { pal.border })
    };
    ui.painter().rect(rect, theme::radius::MD, fill, stroke, egui::StrokeKind::Inside);
    let icon_rect = Rect::from_center_size(rect.left_center() + vec2(18.0, 0.0), vec2(16.0, 16.0));
    paint_icon(ui.painter(), icon_rect, Icon::Search, if has_focus { pal.text } else { pal.text_muted });

    let mut changed = false;
    // Esc: clear, or drop focus when already empty.
    if has_focus && ui.input(|i| i.key_pressed(Key::Escape)) {
        if query.is_empty() {
            ui.memory_mut(|m| m.surrender_focus(edit_id));
        } else {
            query.clear();
            changed = true;
        }
    }

    let clear_w = if query.is_empty() { 0.0 } else { 30.0 };
    let edit_rect = Rect::from_min_max(
        rect.min + vec2(34.0, 0.0),
        rect.max - vec2(6.0 + clear_w, 0.0),
    );
    let response = ui
        .scope_builder(egui::UiBuilder::new().max_rect(edit_rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.add(
                TextEdit::singleline(query)
                    .id(edit_id)
                    .frame(egui::Frame::NONE)
                    .margin(Margin::ZERO)
                    .hint_text(egui::RichText::new(tr.t("search.placeholder")).color(pal.text_faint))
                    .desired_width(edit_rect.width())
                    .background_color(Color32::TRANSPARENT),
            )
        })
        .inner;
    changed |= response.changed();
    if focus {
        response.request_focus();
    }

    if !query.is_empty() {
        let clear_rect = Rect::from_center_size(rect.right_center() - vec2(18.0, 0.0), vec2(26.0, 26.0));
        let resp = ui
            .scope_builder(egui::UiBuilder::new().max_rect(clear_rect), |ui| {
                icon_button_ex(ui, Icon::Close, 11.0, IconButtonStyle { padding: Some(7.5), ..Default::default() }, tr.t("search.clear"))
            })
            .inner;
        if resp.clicked() {
            query.clear();
            changed = true;
            response.request_focus();
        }
    }
    let response = response.on_hover_cursor(CursorIcon::Text);
    SearchBoxOutput { response, changed }
}
