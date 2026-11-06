//! Window-level file drag & drop: overlay while hovering, `DropFiles` on release.

use std::path::PathBuf;

use eframe::egui::{
    self, pos2, vec2, Color32, Context, FontId, Id, LayerId, Order, Rect, Stroke, Vec2,
};

use crate::presentation::ui::{
    actions::UiAction,
    i18n::Tr,
    icons::{paint_icon, Icon},
    theme,
};

/// Paint the drop overlay while files hover the window and return `DropFiles` once they
/// are dropped. Call once per frame (after the panels, so the overlay is on top).
pub fn file_drop(ctx: &Context, tr: &Tr) -> Option<UiAction> {
    let (hovering, dropped): (bool, Vec<PathBuf>) = ctx.input(|i| {
        (
            !i.raw.hovered_files.is_empty(),
            i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect(),
        )
    });
    if hovering {
        paint_overlay(ctx, tr);
    }
    (!dropped.is_empty()).then_some(UiAction::DropFiles(dropped))
}

fn paint_overlay(ctx: &Context, tr: &Tr) {
    let pal = theme::palette_for(ctx, ctx.global_style().visuals.dark_mode);
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("file_drop_overlay")));
    painter.rect_filled(screen, 0.0, Color32::from_black_alpha(150));
    let card = Rect::from_center_size(screen.center(), vec2(420.0f32.min(screen.width() - 32.0), 180.0));
    painter.rect(card, theme::radius::XL, pal.elevated, Stroke::new(2.0, pal.accent), egui::StrokeKind::Inside);
    let icon = Rect::from_center_size(pos2(card.center().x, card.top() + 46.0), Vec2::splat(36.0));
    paint_icon(&painter, icon, Icon::ArrowDownToLine, pal.accent_fg);
    let title = painter.layout_no_wrap(tr.t("drop.title").to_owned(), theme::bold_font(ctx, 18.0), pal.text);
    painter.galley(pos2(card.center().x - title.size().x / 2.0, icon.bottom() + 14.0), title, pal.text);
    let hint = painter.layout(tr.t("drop.hint").to_owned(), FontId::proportional(12.5), pal.text_muted, card.width() - 48.0);
    painter.galley(pos2(card.center().x - hint.size().x / 2.0, icon.bottom() + 44.0), hint, pal.text_muted);
}
