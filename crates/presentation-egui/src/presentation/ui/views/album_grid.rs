//! Responsive, virtualized album card grid with lazy artwork.

use eframe::egui::{
    self, pos2, text::LayoutJob, vec2, CursorIcon, FontId, Id, Rect, ScrollArea, Sense, Ui, Vec2,
};

use crate::presentation::ui::{
    actions::UiAction,
    i18n::Tr,
    icons::{paint_icon, Icon},
    theme,
    widgets::{artwork::paint_shadow, empty_state, paint_artwork, Tex},
};

/// One album card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AlbumItem {
    /// Stable key passed back in actions and to the artwork lookup.
    pub key: String,
    pub title: String,
    pub artist: String,
    pub year: Option<i32>,
    pub track_count: usize,
}

pub struct AlbumGridProps<'a> {
    pub id_salt: &'a str,
    pub albums: &'a [AlbumItem],
    /// Key of the album currently playing (gets an accent badge).
    pub now_playing: Option<&'a str>,
    pub tr: Tr,
}

const GAP: f32 = 20.0;
const MIN_CARD: f32 = 156.0;
const MAX_CARD: f32 = 220.0;
const TEXT_H: f32 = 46.0;

/// Number of columns and card width for an available width.
pub fn grid_metrics(width: f32) -> (usize, f32) {
    let cols = (((width + GAP) / (MIN_CARD + GAP)).floor() as usize).max(1);
    let mut card = (width - GAP * (cols as f32 - 1.0)) / cols as f32;
    let mut cols = cols;
    // Prefer more columns over very wide cards.
    while card > MAX_CARD {
        cols += 1;
        card = (width - GAP * (cols as f32 - 1.0)) / cols as f32;
    }
    (cols, card.max(60.0))
}

/// Draw the grid. `artwork(key)` is called only for visible cards; return `None` while
/// the texture is still loading (a placeholder is drawn).
pub fn album_grid(
    ui: &mut Ui,
    props: &AlbumGridProps<'_>,
    artwork: &mut dyn FnMut(&str) -> Option<Tex>,
) -> Vec<UiAction> {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let mut actions = Vec::new();
    if props.albums.is_empty() {
        empty_state(ui, Icon::Album, tr.t("albums.empty"), tr.t("library.empty_hint"), None);
        return actions;
    }
    let width = ui.available_width() - 12.0;
    let (cols, card) = grid_metrics(width);
    let row_h = card + TEXT_H + GAP;
    let rows = props.albums.len().div_ceil(cols);
    let grid_id = Id::new(("album_grid", props.id_salt));

    ScrollArea::vertical()
        .id_salt(grid_id)
        .auto_shrink([false, false])
        .show_rows(ui, row_h, rows, |ui, range| {
            for r in range {
                let (row_rect, _) = ui.allocate_exact_size(vec2(width, row_h), Sense::hover());
                for c in 0..cols {
                    let i = r * cols + c;
                    let Some(album) = props.albums.get(i) else { break };
                    let rect = Rect::from_min_size(row_rect.min + vec2(c as f32 * (card + GAP), 0.0), vec2(card, card + TEXT_H));
                    let resp = ui
                        .interact(rect, grid_id.with(i), Sense::click())
                        .on_hover_cursor(CursorIcon::PointingHand);
                    let t = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.15);
                    let lift = 3.0 * t;
                    let art = Rect::from_min_size(rect.min - vec2(0.0, lift), Vec2::splat(card));
                    paint_shadow(ui.painter(), art, 10, (10.0 + 14.0 * t) as u8, (if pal.dark { 90.0 } else { 26.0 } + 40.0 * t) as u8);
                    paint_artwork(ui, art, artwork(&album.key), 10, &album.title);

                    // hover play button
                    let mut play_clicked = false;
                    if t > 0.01 {
                        let d = 40.0;
                        let pr = Rect::from_center_size(art.right_bottom() - vec2(d / 2.0 + 10.0, d / 2.0 + 10.0), Vec2::splat(d));
                        let presp = ui
                            .interact(pr, grid_id.with(("play", i)), Sense::click())
                            .on_hover_cursor(CursorIcon::PointingHand)
                            .on_hover_text(tr.t("albums.play"));
                        let scale = if presp.hovered() { 1.06 } else { 1.0 };
                        ui.painter().circle_filled(pr.center() + vec2(0.0, 2.0), d / 2.0 * scale, egui::Color32::from_black_alpha((70.0 * t) as u8));
                        ui.painter().circle_filled(pr.center(), d / 2.0 * scale, theme::with_alpha(pal.accent, t));
                        paint_icon(ui.painter(), Rect::from_center_size(pr.center() + vec2(1.5, 0.0), Vec2::splat(18.0)), Icon::Play, theme::with_alpha(pal.accent_text, t));
                        if presp.clicked() {
                            play_clicked = true;
                            actions.push(UiAction::PlayAlbum(album.key.clone()));
                        }
                    }
                    if props.now_playing == Some(album.key.as_str()) {
                        let br = Rect::from_min_size(art.min + vec2(10.0, 10.0), Vec2::splat(26.0));
                        ui.painter().circle_filled(br.center(), 13.0, pal.accent);
                        paint_icon(ui.painter(), br.shrink(6.0), Icon::Volume2, pal.accent_text);
                    }

                    // text
                    let title_job = {
                        let mut j = LayoutJob::simple_singleline(album.title.clone(), theme::bold_font(ui.ctx(), theme::size::BODY), pal.text);
                        j.wrap = egui::text::TextWrapping::truncate_at_width(card);
                        j
                    };
                    let g = ui.painter().layout_job(title_job);
                    let ty = art.bottom() + lift + 9.0;
                    ui.painter().galley(pos2(rect.left(), ty), g, pal.text);
                    let sub = match album.year {
                        Some(y) => format!("{} · {}", album.artist, y),
                        None => album.artist.clone(),
                    };
                    let mut j = LayoutJob::simple_singleline(sub, FontId::proportional(12.5), pal.text_muted);
                    j.wrap = egui::text::TextWrapping::truncate_at_width(card);
                    let g = ui.painter().layout_job(j);
                    ui.painter().galley(pos2(rect.left(), ty + 19.0), g, pal.text_muted);

                    if resp.clicked() && !play_clicked {
                        actions.push(UiAction::OpenAlbum(album.key.clone()));
                    }
                    resp.context_menu(|ui| {
                        if super::track_table::menu_item(ui, Icon::Play, tr.t("albums.play")).clicked() {
                            actions.push(UiAction::PlayAlbum(album.key.clone()));
                            ui.close();
                        }
                        if super::track_table::menu_item(ui, Icon::Album, tr.t("menu.show_album")).clicked() {
                            actions.push(UiAction::OpenAlbum(album.key.clone()));
                            ui.close();
                        }
                        if super::track_table::menu_item(ui, Icon::Artist, tr.t("menu.show_artist")).clicked() {
                            actions.push(UiAction::OpenArtist(album.artist.clone()));
                            ui.close();
                        }
                    });
                }
            }
        });
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_metrics_fill_width() {
        for w in [100.0, 300.0, 640.0, 1000.0, 1600.0] {
            let (cols, card) = grid_metrics(w);
            assert!(cols >= 1);
            assert!(card <= MAX_CARD + 0.01 || cols == 1);
            let used = cols as f32 * card + (cols as f32 - 1.0) * GAP;
            assert!((used - w).abs() < 0.5 || w < MIN_CARD, "w={w} used={used}");
        }
    }
}
