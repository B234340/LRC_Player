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
    /// `artist` comes from an album-artist tag (else it is the track artist); decides the
    /// Artists view mode when the artist link is clicked.
    pub album_artist: bool,
}

pub struct AlbumGridProps<'a> {
    pub id_salt: &'a str,
    pub albums: &'a [AlbumItem],
    /// Key of the album currently playing (gets an accent badge).
    pub now_playing: Option<&'a str>,
    /// Preferred card width (the Albums header slider); cards stretch up to ~1.3x of it to
    /// fill the row.
    pub card_size: f32,
    pub tr: Tr,
}

const GAP: f32 = 20.0;
const TEXT_H: f32 = 46.0;
/// Card width range of the size slider.
pub const ALBUM_CARD_MIN: f32 = 120.0;
pub const ALBUM_CARD_MAX: f32 = 320.0;
pub const ALBUM_CARD_DEFAULT: f32 = 180.0;

/// Number of columns and card width for an available width and preferred card width.
pub fn grid_metrics(width: f32, preferred: f32) -> (usize, f32) {
    let preferred = preferred.clamp(ALBUM_CARD_MIN, ALBUM_CARD_MAX);
    let min_card = preferred * 0.85;
    let max_card = preferred * 1.25;
    let cols = (((width + GAP) / (min_card + GAP)).floor() as usize).max(1);
    let mut card = (width - GAP * (cols as f32 - 1.0)) / cols as f32;
    let mut cols = cols;
    // Prefer more columns over very wide cards.
    while card > max_card {
        cols += 1;
        card = (width - GAP * (cols as f32 - 1.0)) / cols as f32;
    }
    (cols, card.max(60.0))
}

/// Hit test of a text link (2 px vertical slack for the underline).
fn link_contains(text: Rect, pos: egui::Pos2) -> bool {
    text.expand2(vec2(0.0, 2.0)).contains(pos)
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
    let (cols, card) = grid_metrics(width, props.card_size);
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
                    // Geometric hover: `resp.hovered()` turns false while the pointer is on the
                    // play button (a widget on top of the card), which made it flicker.
                    let card_hovered = ui.rect_contains_pointer(rect) && !egui::DragAndDrop::has_any_payload(ui.ctx());
                    let t = ui.ctx().animate_bool_with_time(resp.id.with("h"), card_hovered, 0.15);
                    let lift = 3.0 * t;
                    let art = Rect::from_min_size(rect.min - vec2(0.0, lift), Vec2::splat(card));
                    paint_shadow(ui.painter(), art, 10, (10.0 + 14.0 * t) as u8, (if pal.dark { 90.0 } else { 26.0 } + 40.0 * t) as u8);
                    paint_artwork(ui, art, artwork(&album.key), 10, &album.title);

                    // hover play button
                    let mut play_clicked = false;
                    if card_hovered || t > 0.01 {
                        let d = 40.0;
                        let pr = Rect::from_center_size(art.right_bottom() - vec2(d / 2.0 + 10.0, d / 2.0 + 10.0), Vec2::splat(d));
                        let presp = ui
                            .interact(pr, grid_id.with(("play", i)), Sense::click())
                            .on_hover_cursor(CursorIcon::PointingHand)
                            .on_hover_text(tr.t("albums.play"));
                        // The card's response is computed first; a click on the button is
                        // routed to the button (registered later = on top), never to both.
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
                    // "Artist · 2020": the artist text is a link (only the text itself).
                    let year = album.year.map(|y| format!(" · {y}")).unwrap_or_default();
                    let year_g = ui.painter().layout_no_wrap(year, FontId::proportional(12.5), pal.text_muted);
                    let sub_y = ty + 19.0;
                    let mut artist_rect = None;
                    if !album.artist.is_empty() {
                        let mut j = LayoutJob::simple_singleline(album.artist.clone(), FontId::proportional(12.5), egui::Color32::PLACEHOLDER);
                        j.wrap = egui::text::TextWrapping::truncate_at_width((card - year_g.size().x).max(24.0));
                        let g = ui.painter().layout_job(j);
                        let r = Rect::from_min_size(pos2(rect.left(), sub_y), g.size());
                        let link_hovered = card_hovered && ui.ctx().pointer_hover_pos().is_some_and(|p| link_contains(r, p));
                        let color = if link_hovered { pal.accent_fg } else { pal.text_muted };
                        if link_hovered {
                            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                            ui.painter().hline(r.x_range(), r.bottom() - 1.0, egui::Stroke::new(1.0, color));
                        }
                        ui.painter().galley(r.min, g, color);
                        artist_rect = Some(r);
                    }
                    let year_x = artist_rect.map_or(rect.left(), |r| r.right());
                    ui.painter().galley(pos2(year_x, sub_y), year_g, pal.text_muted);
                    let over_artist = |p: Option<egui::Pos2>| artist_rect.zip(p).is_some_and(|(r, p)| link_contains(r, p));
                    let resp = if over_artist(ui.ctx().pointer_hover_pos()) && card_hovered {
                        resp.on_hover_text_at_pointer(tr.t("menu.show_artist"))
                    } else {
                        resp
                    };

                    if resp.clicked() && !play_clicked {
                        if over_artist(resp.interact_pointer_pos()) {
                            actions.push(UiAction::OpenArtistAs { name: album.artist.clone(), album_artist: album.album_artist });
                        } else {
                            actions.push(UiAction::OpenAlbum(album.key.clone()));
                        }
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
                            actions.push(UiAction::OpenArtistAs { name: album.artist.clone(), album_artist: album.album_artist });
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
    use egui_kittest::Harness;

    /// Hovering the card's play button for any number of frames keeps it in place, and a
    /// click on it emits only `PlayAlbum` (it used to flicker and open the album instead).
    #[test]
    fn play_button_is_stable_and_click_only_plays() {
        let albums = vec![AlbumItem { key: "k".into(), title: "T".into(), artist: "A".into(), year: None, track_count: 1, album_artist: false }];
        let play_id = Id::new(("album_grid", "t")).with(("play", 0usize));
        for frames in 2..8 {
            let albums = albums.clone();
            let mut h = Harness::builder().with_size(vec2(420.0, 420.0)).build_ui_state(
                move |ui, actions: &mut Vec<UiAction>| {
                    let props = AlbumGridProps { id_salt: "t", albums: &albums, now_playing: None, card_size: ALBUM_CARD_DEFAULT, tr: Tr::JA };
                    actions.extend(album_grid(ui, &props, &mut |_| None));
                },
                Vec::new(),
            );
            h.run_steps(2);
            // Hover the card so the button appears, then find it.
            h.hover_at(egui::pos2(80.0, 80.0));
            h.run_steps(3);
            let button = h.ctx.read_response(play_id).expect("play button shown on hover").rect;
            h.hover_at(button.center());
            for _ in 0..frames {
                h.run_steps(1);
                let r = h.ctx.read_response(play_id).expect("button stays while hovered");
                assert!(r.rect.contains(button.center()), "button moved");
            }
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: button.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
                h.run_steps(1);
            }
            h.run_steps(1);
            assert_eq!(h.state(), &vec![UiAction::PlayAlbum("k".into())], "frames={frames}");
        }
    }

    /// The three hit targets of a card: the artist text opens the artist (in the mode that
    /// matches the name shown), the rest of the card opens the album.
    #[test]
    fn card_click_targets() {
        let albums = vec![AlbumItem { key: "k".into(), title: "T".into(), artist: "Artist".into(), year: Some(2020), track_count: 1, album_artist: true }];
        let card_id = Id::new(("album_grid", "t")).with(0usize);
        let click = |pos_of: &dyn Fn(Rect) -> egui::Pos2| {
            let albums = albums.clone();
            let mut h = Harness::builder().with_size(vec2(420.0, 420.0)).build_ui_state(
                move |ui, actions: &mut Vec<UiAction>| {
                    let props = AlbumGridProps { id_salt: "t", albums: &albums, now_playing: None, card_size: ALBUM_CARD_DEFAULT, tr: Tr::JA };
                    actions.extend(album_grid(ui, &props, &mut |_| None));
                },
                Vec::new(),
            );
            h.run_steps(2);
            let card = h.ctx.read_response(card_id).expect("card").rect;
            let pos = pos_of(card);
            h.hover_at(pos);
            h.run_steps(3);
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
                h.run_steps(1);
            }
            h.run_steps(1);
            h.state().clone()
        };
        // artist text: first characters of the subtitle line under the title
        let artist = click(&|r: Rect| pos2(r.left() + 6.0, r.top() + r.width() + 9.0 + 19.0 + 7.0));
        assert_eq!(artist, vec![UiAction::OpenArtistAs { name: "Artist".into(), album_artist: true }]);
        // the year after the artist, and the artwork: open the album
        let year = click(&|r: Rect| pos2(r.left() + 70.0, r.top() + r.width() + 9.0 + 19.0 + 7.0));
        assert_eq!(year, vec![UiAction::OpenAlbum("k".into())]);
        let art = click(&|r: Rect| pos2(r.left() + 20.0, r.top() + 20.0));
        assert_eq!(art, vec![UiAction::OpenAlbum("k".into())]);
    }

    #[test]
    fn grid_metrics_fill_width() {
        for pref in [ALBUM_CARD_MIN, ALBUM_CARD_DEFAULT, ALBUM_CARD_MAX] {
            for w in [100.0, 300.0, 640.0, 1000.0, 1600.0] {
                let (cols, card) = grid_metrics(w, pref);
                assert!(cols >= 1);
                assert!(card <= pref * 1.25 + 0.01 || cols == 1);
                let used = cols as f32 * card + (cols as f32 - 1.0) * GAP;
                assert!((used - w).abs() < 0.5 || w < pref, "w={w} used={used}");
            }
        }
        // bigger preference -> fewer, larger cards
        assert!(grid_metrics(1000.0, 300.0).0 < grid_metrics(1000.0, 140.0).0);
    }
}
