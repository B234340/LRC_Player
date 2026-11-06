//! Albums: responsive card grid, and the album page (header + track table).

use std::path::Path;

use eframe::egui::{self, Ui};

use super::{back_link, hash_key, loading, tracks_summary};
use crate::presentation::app::{shell::playlist_entries, GuiApp};
use crate::presentation::ui::{
    i18n::Tr,
    icons::Icon,
    theme,
    views::{album_grid, track_table, AlbumGridProps, Column, IndexMode, TrackTableProps},
    widgets::{artwork, button, section_header, ButtonKind, Tex},
    UiAction,
};

/// Header shared by the album / artist / playlist pages: `art` draws the square on the left.
pub(super) fn detail_header(
    ui: &mut Ui,
    art: impl FnOnce(&mut Ui),
    kicker: &str,
    title: &str,
    subtitle_link: Option<&str>,
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
            ui.add(egui::Label::new(theme::display(ui, title).color(pal.text)).truncate());
            if let Some(link) = subtitle_link.filter(|l| !l.is_empty()) {
                let resp = ui
                    .add(egui::Label::new(egui::RichText::new(link).size(15.0).color(pal.text)).sense(egui::Sense::click()).truncate())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if resp.hovered() {
                    ui.painter().hline(resp.rect.x_range(), resp.rect.bottom(), egui::Stroke::new(1.0, pal.text_muted));
                }
                link_clicked = resp.clicked();
            }
            ui.label(theme::muted(ui, meta));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
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
    section_header(ui, tr.t("nav.albums"), Some(&sub), |_| {});
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
        &AlbumGridProps { id_salt: "albums", albums, now_playing: now_key.as_deref(), tr },
        &mut |key| {
            let title = albums.iter().find(|a| a.key == key)?.title.as_str();
            loader.album_cover(title).as_ref().map(Tex::from)
        },
    )
}

fn detail(app: &mut GuiApp, ui: &mut Ui, tr: Tr, key: &str) -> Vec<UiAction> {
    let mut actions = Vec::new();
    if back_link(ui, tr.t("nav.albums")) {
        actions.push(UiAction::Navigate(crate::presentation::ui::Page::Albums));
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
            artwork(ui, cover.as_ref().map(Tex::from), 168.0, 12, &album.title);
        },
        tr.t("albums.kicker"),
        &album.title,
        Some(&album.artist),
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
        actions.push(UiAction::OpenArtist(album.artist.clone()));
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
            tr,
        },
        &mut app.ui.detail_table,
    ));
    actions
}
