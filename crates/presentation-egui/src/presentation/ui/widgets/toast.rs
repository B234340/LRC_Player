//! Toast notifications: queue, auto-dismiss, hover-to-pause, four severities.

use eframe::egui::{
    self, vec2, Align2, Context, CursorIcon, Frame, Id, Margin, Order, RichText, Sense, Stroke, Vec2,
};

use crate::presentation::ui::{
    actions::UiAction,
    i18n::Tr,
    icons::{self, icon_button_ex, Icon, IconButtonStyle},
    theme::{self, radius},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastKind {
    fn icon(self) -> Icon {
        match self {
            ToastKind::Info => Icon::Info,
            ToastKind::Success => Icon::Success,
            ToastKind::Warning => Icon::Warning,
            ToastKind::Error => Icon::Error,
        }
    }

    /// Default lifetime in seconds.
    pub fn default_secs(self) -> f32 {
        match self {
            ToastKind::Info | ToastKind::Success => 3.5,
            ToastKind::Warning => 6.0,
            ToastKind::Error => 8.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub text: String,
    /// Lifetime in seconds (`f32::INFINITY` = sticky).
    pub secs: f32,
    /// Remaining seconds (paused while hovered).
    remaining: f32,
    /// Time since shown, for the fade-in.
    age: f32,
    /// Optional action button: (label, action emitted when clicked; the toast closes).
    pub action: Option<(String, UiAction)>,
}

/// Toast queue. Push from anywhere; call [`Toasts::show`] once per frame.
#[derive(Clone, Debug)]
pub struct Toasts {
    items: Vec<Toast>,
    next_id: u64,
    /// Maximum toasts on screen; the rest wait in the queue.
    pub max_visible: usize,
    /// Distance from the bottom-right corner of the screen.
    pub margin: Vec2,
    last_time: Option<f64>,
}

impl Default for Toasts {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            next_id: 1,
            max_visible: 4,
            margin: vec2(16.0, 16.0),
            last_time: None,
        }
    }
}

impl Toasts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, kind: ToastKind, text: impl Into<String>, secs: f32) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Toast { id, kind, text: text.into(), secs, remaining: secs, age: 0.0, action: None });
        id
    }

    /// Toast with an action button (e.g. "Show queue"); [`Toasts::show`] returns the action
    /// when it is clicked.
    pub fn push_with_action(&mut self, kind: ToastKind, text: impl Into<String>, secs: f32, label: impl Into<String>, action: UiAction) -> u64 {
        let id = self.push(kind, text, secs);
        if let Some(t) = self.items.last_mut() {
            t.action = Some((label.into(), action));
        }
        id
    }

    pub fn info(&mut self, text: impl Into<String>) -> u64 {
        self.push(ToastKind::Info, text, ToastKind::Info.default_secs())
    }
    pub fn success(&mut self, text: impl Into<String>) -> u64 {
        self.push(ToastKind::Success, text, ToastKind::Success.default_secs())
    }
    pub fn warning(&mut self, text: impl Into<String>) -> u64 {
        self.push(ToastKind::Warning, text, ToastKind::Warning.default_secs())
    }
    pub fn error(&mut self, text: impl Into<String>) -> u64 {
        self.push(ToastKind::Error, text, ToastKind::Error.default_secs())
    }

    pub fn dismiss(&mut self, id: u64) {
        self.items.retain(|t| t.id != id);
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Draw the toasts in the bottom-right corner. `bottom_inset` keeps them above the
    /// transport bar. Returns the actions of clicked toast buttons.
    pub fn show(&mut self, ctx: &Context, bottom_inset: f32, tr: &Tr) -> Vec<UiAction> {
        let mut actions = Vec::new();
        let now = ctx.input(|i| i.time);
        let dt = self.last_time.map(|t| (now - t) as f32).unwrap_or(0.0).clamp(0.0, 0.25);
        self.last_time = Some(now);
        if self.items.is_empty() {
            return actions;
        }
        let pal = theme::palette_for(ctx, ctx.global_style().visuals.dark_mode);
        let mut dismissed: Vec<u64> = Vec::new();
        let visible = self.items.len().min(self.max_visible);

        egui::Area::new(Id::new("lrc_ui_toasts"))
            .order(Order::Foreground)
            .anchor(Align2::RIGHT_BOTTOM, vec2(-self.margin.x, -(self.margin.y + bottom_inset)))
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_max_width(410.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                // newest at the bottom
                for toast in self.items[..visible].iter_mut() {
                    let fade_in = (toast.age / 0.15).clamp(0.0, 1.0);
                    let fade_out = if toast.secs.is_finite() { (toast.remaining / 0.3).clamp(0.0, 1.0) } else { 1.0 };
                    let alpha = fade_in.min(fade_out);
                    let accent = match toast.kind {
                        ToastKind::Info => pal.accent,
                        ToastKind::Success => pal.success,
                        ToastKind::Warning => pal.warning,
                        ToastKind::Error => pal.danger,
                    };
                    ui.set_opacity(alpha);
                    let frame = Frame::new()
                        .fill(pal.elevated)
                        .stroke(Stroke::new(1.0, pal.border))
                        .corner_radius(radius::LG)
                        .shadow(egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: pal.shadow })
                        .inner_margin(Margin { left: 12, right: 6, top: 8, bottom: 8 });
                    let resp = frame
                        .show(ui, |ui| {
                            ui.set_width(if toast.action.is_some() { 390.0 } else { 330.0 });
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                                icons::paint_icon(ui.painter(), r, toast.kind.icon(), accent);
                                ui.add_space(4.0);
                                let action_w = toast.action.as_ref().map_or(0.0, |(label, _)| {
                                    ui.painter().layout_no_wrap(label.clone(), egui::FontId::proportional(12.5), pal.text).size().x + 26.0
                                });
                                let w = ui.available_width() - 34.0 - action_w;
                                ui.allocate_ui(vec2(w, 20.0), |ui| {
                                    ui.add(egui::Label::new(RichText::new(&toast.text).color(pal.text)).wrap());
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let close = icon_button_ex(
                                        ui,
                                        Icon::Close,
                                        12.0,
                                        IconButtonStyle { padding: Some(7.0), ..Default::default() },
                                        tr.t("toast.close"),
                                    );
                                    if close.clicked() {
                                        dismissed.push(toast.id);
                                    }
                                    if let Some((label, action)) = &toast.action {
                                        if action_button(ui, label, accent).clicked() {
                                            actions.push(action.clone());
                                            dismissed.push(toast.id);
                                        }
                                    }
                                });
                            });
                        })
                        .response;
                    // left severity bar
                    let bar = egui::Rect::from_min_size(resp.rect.min + vec2(0.0, 10.0), vec2(3.0, resp.rect.height() - 20.0));
                    ui.painter().rect_filled(bar, 1.5, accent);
                    let hovered = ui.rect_contains_pointer(resp.rect);
                    if hovered {
                        ui.ctx().set_cursor_icon(CursorIcon::Default);
                    } else {
                        toast.remaining -= dt;
                    }
                    toast.age += dt;
                    if toast.remaining <= 0.0 {
                        dismissed.push(toast.id);
                    }
                }
            });
        self.items.retain(|t| !dismissed.contains(&t.id));
        if !self.items.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
        actions
    }
}

/// Compact text button in the toast's accent color.
fn action_button(ui: &mut egui::Ui, label: &str, accent: egui::Color32) -> egui::Response {
    let g = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.5), accent);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 18.0, 26.0), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let fill = if resp.hovered() { theme::with_alpha(accent, 0.18) } else { theme::with_alpha(accent, 0.10) };
    ui.painter().rect_filled(rect, radius::MD, fill);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, accent);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_and_dismiss() {
        let mut t = Toasts::new();
        let a = t.info("a");
        let _b = t.error("b");
        assert_eq!(t.len(), 2);
        t.dismiss(a);
        assert_eq!(t.len(), 1);
        assert_eq!(t.items[0].kind, ToastKind::Error);
        t.push_with_action(ToastKind::Success, "c", 3.0, "Show", UiAction::QueueClear);
        assert_eq!(t.items[1].action.as_ref().map(|a| a.0.as_str()), Some("Show"));
    }
}
