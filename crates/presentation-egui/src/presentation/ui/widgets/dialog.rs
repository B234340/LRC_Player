//! Modal dialog in the design-system style: elevated rounded surface with a soft shadow over
//! a dimmed backdrop, a heading, body content and a right-aligned button row.
//!
//! Keyboard: Enter confirms (when the confirm button is enabled; ignored while an IME
//! composition is being committed), Esc or a click on the backdrop cancels. The body closure
//! gets `opened = true` on the first frame so it can focus its text field
//! ([`dialog_text_field`] does that).

use eframe::egui::{
    self, text::CCursor, text_selection::CCursorRange, vec2, Align, Color32, Context, Event, Frame,
    Id, Key, Layout, Margin, Response, RichText, Stroke, Ui,
};

use super::card::{button_sized, ButtonKind};
use crate::presentation::ui::{
    icons::Icon,
    theme::{self, radius, space},
};

/// Minimum width of dialog buttons, so short labels ("OK") still look balanced.
const BUTTON_MIN_W: f32 = 88.0;

/// One button of the dialog's button row.
#[derive(Clone, Copy, Debug)]
pub struct DialogButton<'a> {
    pub kind: ButtonKind,
    pub icon: Option<Icon>,
    pub label: &'a str,
}

impl<'a> DialogButton<'a> {
    pub fn primary(label: &'a str) -> Self {
        Self { kind: ButtonKind::Primary, icon: None, label }
    }

    /// Confirm button of a destructive action.
    pub fn danger(label: &'a str) -> Self {
        Self { kind: ButtonKind::DangerSolid, icon: Some(Icon::Trash), label }
    }

    pub fn secondary(icon: Option<Icon>, label: &'a str) -> Self {
        Self { kind: ButtonKind::Secondary, icon, label }
    }
}

pub struct DialogSpec<'a> {
    pub id: Id,
    pub title: &'a str,
    /// Content width (clamped to the window).
    pub width: f32,
    /// Right-most button; Enter triggers it.
    pub confirm: DialogButton<'a>,
    /// Secondary "Cancel" button left of `confirm` (Esc always cancels).
    pub cancel: Option<&'a str>,
    /// Optional extra button left of `cancel`.
    pub extra: Option<DialogButton<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogResult {
    /// Still open.
    Open,
    Confirm,
    Cancel,
    Extra,
}

/// Show the dialog for this frame. `body(ui, opened)` draws the content below the title and
/// returns whether the confirm button is enabled.
pub fn dialog(ctx: &Context, spec: &DialogSpec<'_>, body: impl FnOnce(&mut Ui, bool) -> bool) -> DialogResult {
    let pal = theme::palette_for(ctx, ctx.global_style().visuals.dark_mode);
    let frame_nr = ctx.cumulative_frame_nr();
    let seen_id = spec.id.with("dialog_last_frame");
    let last: Option<u64> = ctx.data(|d| d.get_temp(seen_id));
    let opened = !matches!(last, Some(f) if f + 1 >= frame_nr);
    ctx.data_mut(|d| d.insert_temp(seen_id, frame_nr));

    let width = spec.width.min(ctx.content_rect().width() - 2.0 * space::XXL - 2.0 * space::XL).max(200.0);
    let frame = Frame::new()
        .fill(pal.elevated)
        .stroke(Stroke::new(1.0, if pal.dark { pal.border_strong } else { pal.border }))
        .corner_radius(radius::XL)
        .shadow(egui::epaint::Shadow { offset: [0, 16], blur: 48, spread: 0, color: theme::with_alpha(Color32::BLACK, if pal.dark { 0.55 } else { 0.22 }) })
        .inner_margin(Margin::same(space::XL as i8));
    let backdrop = if pal.dark { Color32::from_black_alpha(150) } else { Color32::from_rgba_unmultiplied(16, 20, 28, 90) };

    let mut result = DialogResult::Open;
    let resp = egui::Modal::new(spec.id).frame(frame).backdrop_color(backdrop).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = vec2(space::SM, space::SM);
        ui.label(theme::heading(ui, spec.title).color(pal.text));
        ui.add_space(space::XS);
        let ok = body(ui, opened);
        ui.add_space(space::LG);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            let c = spec.confirm;
            let confirm = ui.add_enabled_ui(ok, |ui| button_sized(ui, c.kind, c.icon, c.label, BUTTON_MIN_W)).inner;
            if confirm.clicked() {
                result = DialogResult::Confirm;
            }
            if let Some(cancel) = spec.cancel {
                if button_sized(ui, ButtonKind::Secondary, None, cancel, BUTTON_MIN_W).clicked() {
                    result = DialogResult::Cancel;
                }
            }
            if let Some(e) = spec.extra {
                if button_sized(ui, e.kind, e.icon, e.label, 0.0).clicked() {
                    result = DialogResult::Extra;
                }
            }
        });
        ok
    });
    if result == DialogResult::Open {
        let enter = resp.is_top_modal
            && !resp.any_popup_open
            && ctx.input(|i| i.key_pressed(Key::Enter) && !i.events.iter().any(|e| matches!(e, Event::Ime(_))));
        if enter && resp.inner {
            result = DialogResult::Confirm;
        } else if resp.should_close() {
            result = DialogResult::Cancel;
        }
    }
    result
}

/// Body text (muted, wrapping).
pub fn dialog_text(ui: &mut Ui, text: &str) {
    let pal = theme::palette(ui);
    ui.add(egui::Label::new(RichText::new(text).color(pal.text_muted)).wrap());
}

/// Validation message under a field (danger color).
pub fn dialog_error(ui: &mut Ui, text: &str) {
    let pal = theme::palette(ui);
    ui.label(RichText::new(text).size(12.0).color(pal.danger));
}

/// Full-width single-line field, 36 px tall. Focused with the text selected on the first
/// frame (`opened`), and kept focused after Enter so a rejected name can be corrected.
pub fn dialog_text_field(ui: &mut Ui, text: &mut String, opened: bool, hint: &str) -> Response {
    let pal = theme::palette(ui);
    let id = ui.id().with("dialog_text_field");
    let resp = ui.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .hint_text(RichText::new(hint).color(pal.text_faint))
            .desired_width(f32::INFINITY)
            .min_size(vec2(0.0, 36.0))
            .vertical_align(Align::Center)
            .margin(Margin::symmetric(10, 6)),
    );
    if opened {
        resp.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            let n = text.chars().count();
            state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(n))));
            state.store(ui.ctx(), id);
        }
    } else if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
        resp.request_focus();
    }
    // Focus ring in the accent color.
    if resp.has_focus() {
        ui.painter().rect_stroke(resp.rect, radius::MD, Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
    }
    resp
}
