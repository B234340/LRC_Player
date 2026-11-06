//! Slim horizontal slider used by the seek bar and the volume control.

use eframe::egui::{
    self, pos2, vec2, Color32, CursorIcon, FontId, Id, LayerId, Order, Rect, Response, Sense,
    Stroke, Ui,
};

use crate::presentation::ui::theme::{self, Palette};

/// Colors for [`slim_slider`].
#[derive(Clone, Copy, Debug)]
pub struct SlimSliderStyle {
    pub rail: Color32,
    pub fill: Color32,
    pub fill_hover: Color32,
    pub knob: Color32,
    /// Track thickness at rest / when hovered.
    pub thickness: f32,
    pub thickness_hover: f32,
    pub height: f32,
}

impl SlimSliderStyle {
    pub fn themed(p: &Palette) -> Self {
        Self {
            rail: theme::with_alpha(p.text, if p.dark { 0.16 } else { 0.14 }),
            fill: p.text_muted,
            fill_hover: p.accent,
            knob: if p.dark { Color32::WHITE } else { p.text },
            thickness: 4.0,
            thickness_hover: 6.0,
            height: 18.0,
        }
    }

    /// Variant for use over dark artwork backdrops.
    pub fn on_dark() -> Self {
        Self {
            rail: Color32::from_white_alpha(46),
            fill: Color32::from_white_alpha(210),
            fill_hover: Color32::WHITE,
            knob: Color32::WHITE,
            thickness: 4.0,
            thickness_hover: 6.0,
            height: 18.0,
        }
    }
}

/// Result of a [`slim_slider`] frame.
pub struct SlimSliderOutput {
    pub response: Response,
    /// Value committed this frame (click or drag release).
    pub committed: Option<f32>,
    /// Live value while dragging (for previews).
    pub dragging: Option<f32>,
    /// Value under the pointer while hovering.
    pub hover_value: Option<f32>,
}

/// Slim slider over `0..=1`. Does not change anything itself; the caller applies
/// `committed` (or `dragging` for live controls such as volume).
pub fn slim_slider(ui: &mut Ui, id: Id, value: f32, width: f32, style: SlimSliderStyle) -> SlimSliderOutput {
    let (rect, _) = ui.allocate_exact_size(vec2(width, style.height), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let track = rect.shrink2(vec2(6.0, 0.0));
    let frac_at = |x: f32| ((x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);

    let pointer = ui.input(|i| i.pointer.latest_pos());
    let hover_value = if resp.hovered() || resp.dragged() {
        pointer.map(|p| frac_at(p.x))
    } else {
        None
    };
    let mut dragging = None;
    let mut committed = None;
    if resp.dragged() {
        dragging = resp.interact_pointer_pos().or(pointer).map(|p| frac_at(p.x));
        if let Some(v) = dragging {
            ui.data_mut(|d| d.insert_temp(id.with("drag"), v));
        }
    }
    if resp.drag_stopped() {
        committed = ui
            .data(|d| d.get_temp::<f32>(id.with("drag")))
            .or_else(|| pointer.map(|p| frac_at(p.x)));
    } else if resp.clicked() {
        committed = resp.interact_pointer_pos().or(pointer).map(|p| frac_at(p.x));
    }

    if ui.is_rect_visible(rect) {
        let active = resp.hovered() || resp.dragged();
        let t = ui.ctx().animate_bool_with_time(id.with("hover"), active, 0.12);
        let th = egui::lerp(style.thickness..=style.thickness_hover, t);
        let shown = dragging.unwrap_or(value).clamp(0.0, 1.0);
        let y = rect.center().y;
        let rail = Rect::from_min_max(pos2(track.left(), y - th / 2.0), pos2(track.right(), y + th / 2.0));
        let painter = ui.painter();
        painter.rect_filled(rail, th / 2.0, style.rail);
        let fill_color = theme::blend(style.fill, style.fill_hover, t);
        let fill_rect = Rect::from_min_max(rail.min, pos2(track.left() + track.width() * shown, rail.max.y));
        if fill_rect.width() > 0.5 {
            painter.rect_filled(fill_rect, th / 2.0, fill_color);
        }
        if t > 0.01 {
            let kx = track.left() + track.width() * shown;
            painter.circle_filled(pos2(kx, y + 1.0), 6.5 * t, Color32::from_black_alpha((60.0 * t) as u8));
            painter.circle_filled(pos2(kx, y), 6.0 * t, style.knob);
        }
    }
    SlimSliderOutput { response: resp, committed, dragging, hover_value }
}

/// Small floating label above `anchor` (drawn on the tooltip layer, no delay).
pub fn floating_label(ui: &Ui, id: Id, anchor: egui::Pos2, text: &str) {
    let pal = theme::palette(ui);
    let painter = ui.ctx().layer_painter(LayerId::new(Order::Tooltip, id));
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(12.0), pal.text);
    let size = galley.size() + vec2(14.0, 8.0);
    let mut rect = Rect::from_center_size(anchor - vec2(0.0, size.y / 2.0 + 8.0), size);
    let screen = ui.ctx().content_rect();
    if rect.left() < screen.left() + 2.0 {
        rect = rect.translate(vec2(screen.left() + 2.0 - rect.left(), 0.0));
    }
    if rect.right() > screen.right() - 2.0 {
        rect = rect.translate(vec2(screen.right() - 2.0 - rect.right(), 0.0));
    }
    painter.add(pal_shadow(&pal).as_shape(rect, 6));
    painter.rect(rect, 6.0, pal.elevated, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        pal.text,
    );
}

fn pal_shadow(p: &Palette) -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 3],
        blur: 10,
        spread: 0,
        color: p.shadow,
    }
}

/// Mouse-wheel movement this frame in "notches" (positive = wheel up / away from user).
pub fn wheel_notches(ui: &Ui) -> f32 {
    ui.input(|i| {
        i.events
            .iter()
            .map(|e| match e {
                egui::Event::MouseWheel { unit, delta, .. } => match unit {
                    egui::MouseWheelUnit::Line => delta.y,
                    egui::MouseWheelUnit::Point => delta.y / 50.0,
                    egui::MouseWheelUnit::Page => delta.y * 3.0,
                },
                _ => 0.0,
            })
            .sum()
    })
}
