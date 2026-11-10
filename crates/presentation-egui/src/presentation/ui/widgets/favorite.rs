//! Heart toggle for marking a track as a favorite.

use eframe::egui::{self, Color32, CursorIcon, Id, Rect, Response, Sense, Ui};

use crate::presentation::ui::i18n::Tr;
use crate::presentation::ui::icons::{paint_icon, Icon};
use crate::presentation::ui::theme;

/// Colors of the heart button.
#[derive(Clone, Copy, Debug)]
pub struct FavoriteColors {
    /// Outline when not a favorite.
    pub idle: Color32,
    /// Outline when not a favorite and hovered.
    pub hover: Color32,
    /// Filled heart when a favorite.
    pub active: Color32,
}

impl FavoriteColors {
    /// Default colors on the app's surfaces.
    pub fn from_palette(pal: &theme::Palette) -> Self {
        Self { idle: pal.text_muted, hover: pal.text, active: pal.accent }
    }

    /// Colors on the dark artwork backdrop (Now Playing).
    pub fn on_dark(accent: Color32) -> Self {
        Self { idle: Color32::from_white_alpha(170), hover: Color32::WHITE, active: accent }
    }
}

/// Heart toggle at `rect` (registered on top of whatever is below it). Filled when `favorite`,
/// an outline otherwise. Returns the response; `clicked()` means "toggle".
pub fn favorite_button(ui: &Ui, rect: Rect, id: Id, favorite: bool, colors: FavoriteColors, tr: &Tr) -> Response {
    let resp = ui.interact(rect, id, Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    let tip = if favorite { tr.t("favorite.remove") } else { tr.t("favorite.add") };
    let resp = resp.on_hover_text(tip);
    let hot = resp.hovered();
    let pressed = resp.is_pointer_button_down_on();
    let side = rect.width().min(rect.height());
    let icon_side = side * if pressed { 0.56 } else { 0.62 };
    let icon_rect = Rect::from_center_size(rect.center(), egui::Vec2::splat(icon_side));
    let (icon, color) = if favorite {
        (Icon::HeartFilled, if hot { theme::lighten(colors.active, 0.12) } else { colors.active })
    } else {
        (Icon::Heart, if hot { colors.hover } else { colors.idle })
    };
    paint_icon(ui.painter(), icon_rect, icon, color);
    resp
}
