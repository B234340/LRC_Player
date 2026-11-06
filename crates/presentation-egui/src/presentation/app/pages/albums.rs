//! Albums: responsive card grid, and the album page (header + track table).

use std::path::Path;

use eframe::egui::{self, Ui};

use super::{back_nav, hash_key, loading, tracks_summary};
use crate::presentation::app::{shell::playlist_entries, GuiApp};
use crate::presentation::ui::{
    i18n::Tr,
    icons::{paint_icon, Icon},
    theme,
    views::album_grid::{ALBUM_CARD_MAX, ALBUM_CARD_MIN},
    views::{album_grid, track_table, AlbumGridProps, Column, IndexMode, TableLinks, TrackTableProps},
    widgets::{artwork, button, section_header, slim_slider, ButtonKind, SlimSliderStyle, Tex},
    Page, UiAction,
};

/// Size of the detail header's art square: `max` on wide pages, shrinking (down to 96 px)
/// when the central area is narrow (e.g. with the right panel open).
pub(super) fn detail_art_size(ui: &Ui, max: f32) -> f32 {
    (ui.available_width() * 0.24).clamp(96.0, max)
}

/// Header shared by the album / artist / playlist pages: `art` draws the square on the left.
pub(super) fn detail_header(
    ui: &mut Ui,
    art: impl FnOnce(&mut Ui),
    kicker: &str,
    title: &str,
    // (link text, tooltip)
    subtitle_link: Option<(&str, &str)>,
    meta: &str,
    buttons: impl FnOnce(&mut Ui),
) -> bool {
    let pal = theme::palette(ui);
    let mut link_clicked = false;
    ui.horizontal(|ui| {
        art(ui);
        ui.add_space(20.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.add_space(8.0);
            ui.label(theme::caption(ui, kicker));
            ui.add(egui::Label::new(crate::presentation::ui::text::rich(title, theme::size::DISPLAY, true).color(pal.text)).truncate());
            if let Some((link, link_tip)) = subtitle_link.filter(|l| !l.0.is_empty()) {
                // Link styled like the track-table links: accent + underline on hover.
                let id = ui.id().with("detail_subtitle_link");
                let hovered = ui.ctx().read_response(id).is_some_and(|r| r.contains_pointer());
                let color = if hovered { pal.accent_fg } else { pal.text };
                let resp = ui
                    .add(egui::Label::new(crate::presentation::ui::text::rich(link, 15.0, false).color(color)).truncate());
                let resp = ui.interact(resp.rect, id, egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text_at_pointer(link_tip);
                if resp.contains_pointer() {
                    ui.painter().hline(resp.rect.x_range(), resp.rect.bottom(), egui::Stroke::new(1.0, color));
                }
                link_clicked = resp.clicked();
            }
            ui.label(theme::muted(ui, meta));
            ui.add_space(8.0);
            // Wrap the buttons onto a second line when the page is narrow.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                buttons(ui);
            });
        });
    });
    link_clicked
}

pub(super) fn show(app: &mut GuiApp, ui: &mut Ui, tr: Tr) -> Vec<UiAction> {
    if let Some(key) = app.ui.album_detail.clone() {
        return detail(app, ui, tr, &key);
    }
    let sub = format!("{} {}", app.library.albums.len(), tr.t("albums.count"));
    let mut size = app.ui.album_card_size;
    let mut commit = false;
    section_header(ui, tr.t("nav.albums"), Some(&sub), |ui| {
        // Card size: big icon on the right, small on the left of a slim slider.
        let pal = theme::palette(ui);
        ui.spacing_mut().item_spacing.x = 6.0;
        let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
        paint_icon(ui.painter(), r, Icon::Album, pal.text_muted);
        let t = (size - ALBUM_CARD_MIN) / (ALBUM_CARD_MAX - ALBUM_CARD_MIN);
        let out = slim_slider(ui, egui::Id::new("album_card_size"), t, 110.0, SlimSliderStyle::themed(&pal));
        if let Some(v) = out.dragging.or(out.committed) {
            size = ALBUM_CARD_MIN + v * (ALBUM_CARD_MAX - ALBUM_CARD_MIN);
        }
        commit = out.committed.is_some();
        out.response.on_hover_text(tr.t("albums.card_size"));
        let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(12.0), egui::Sense::hover());
        paint_icon(ui.painter(), r, Icon::Album, pal.text_faint);
    });
    if size != app.ui.album_card_size {
        app.ui.album_card_size = size;
    }
    if commit {
        app.save_state();
    }
    ui.add_space(16.0);
    if !app.library.loaded {
        loading(ui, tr.t("library.loading"));
        return Vec::new();
    }
    let now_key = app.library.album_key_of(Path::new(&app.play_path));
    let albums = &app.library.albums;
    let loader = &mut app.artwork;
    album_grid(
        ui,
        &AlbumGridProps { id_salt: "albums", albums, now_playing: now_key.as_deref(), card_size: app.ui.album_card_size, tr },
        &mut |key| {
            let title = albums.iter().find(|a| a.key == key)?.title.as_str();
            loader.album_cover(title).as_ref().map(Tex::from)
        },
    )
}

fn detail(app: &mut GuiApp, ui: &mut Ui, tr: Tr, key: &str) -> Vec<UiAction> {
    let mut actions = Vec::new();
    if let Some(a) = back_nav(app, ui, &tr, Page::Albums) {
        actions.push(a);
        return actions;
    }
    ui.add_space(8.0);
    let Some(album) = app.library.album(key).cloned() else {
        if app.library.loaded {
            ui.label(theme::muted(ui, tr.t("albums.not_found")));
        } else {
            loading(ui, tr.t("library.loading"));
        }
        return actions;
    };
    let rows_idx = app.library.album_rows(key).to_vec();
    let library = &app.library;
    app.ui.detail_rows.ensure(hash_key(("album", key, library.revision)), || {
        rows_idx.iter().map(|&i| library.rows[i].clone()).collect()
    });
    let cover = app.artwork.album_cover(&album.title);
    let mut meta = tracks_summary(&app.ui.detail_rows.rows, &tr);
    if let Some(y) = album.year {
        meta = format!("{y} · {meta}");
    }
    let paths: Vec<std::path::PathBuf> = app.ui.detail_rows.rows.iter().map(|r| r.path.clone()).collect();
    let link = detail_header(
        ui,
        |ui| {
            artwork(ui, cover.as_ref().map(Tex::from), detail_art_size(ui, 200.0), 12, &album.title);
        },
        tr.t("albums.kicker"),
        &album.title,
        Some((album.artist.as_str(), tr.t("menu.show_artist"))),
        &meta,
        |ui| {
            if button(ui, ButtonKind::Primary, Some(Icon::Play), tr.t("albums.play")).clicked() {
                actions.push(UiAction::PlayAlbum(album.key.clone()));
            }
            if button(ui, ButtonKind::Secondary, Some(Icon::Next), tr.t("menu.play_next")).clicked() {
                actions.push(UiAction::PlayNext(paths.clone()));
            }
            if button(ui, ButtonKind::Secondary, Some(Icon::Queue), tr.t("menu.add_to_queue")).clicked() {
                actions.push(UiAction::Enqueue(paths.clone()));
            }
        },
    );
    if link {
        actions.push(UiAction::OpenArtistAs { name: album.artist.clone(), album_artist: album.album_artist });
    }
    ui.add_space(16.0);
    let entries = playlist_entries(&app.playlists, &tr);
    let now = (!app.play_path.is_empty()).then(|| Path::new(app.play_path.as_str()));
    let columns = [Column::Index, Column::Title, Column::Artist, Column::Duration];
    actions.extend(track_table(
        ui,
        &TrackTableProps {
            id_salt: "album_detail",
            rows: &app.ui.detail_rows.rows,
            rows_revision: app.ui.detail_rows.revision,
            query: "",
            columns: &columns,
            now_playing: now,
            playing: app.is_playing(),
            index_mode: IndexMode::TrackNumber,
            reorderable: false,
            playlists: &entries,
            removable: false,
            optional_columns: &[],
            // The album column would point to this page.
            links: TableLinks { title: true, artist: true, album: false },
            tr,
        },
        &mut app.ui.detail_table,
    ));
    actions
}
