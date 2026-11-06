//! Artists: searchable list, and the artist page (albums strip + all tracks).

use std::path::Path;

use eframe::egui::{self, pos2, vec2, CursorIcon, FontId, Rect, ScrollArea, Sense, Ui};

use super::{albums::{detail_art_size, detail_header}, avatar, back_nav, hash_key, loading, tracks_summary};
use crate::presentation::app::{shell::playlist_entries, ArtistViewMode, GuiApp};
use crate::presentation::ui::{
    collate::collate,
    format::format_count,
    i18n::Tr,
    icons::Icon,
    theme,
    views::{normalize, track_table, Column, IndexMode, TableLinks, TrackTableProps},
    widgets::{button, empty_state, paint_artwork, search_box_with_hint, section_header, segmented, ButtonKind, Tex},
    Page, UiAction,
};

const ROW_H: f32 = 56.0;

fn other_mode(mode: ArtistViewMode) -> ArtistViewMode {
    if mode == ArtistViewMode::AlbumArtist {
        ArtistViewMode::Artist
    } else {
        ArtistViewMode::AlbumArtist
    }
}

pub(super) fn show(app: &mut GuiApp, ui: &mut Ui, tr: Tr) -> Vec<UiAction> {
    if let Some(name) = app.ui.artist_detail.clone() {
        return detail(app, ui, tr, &name);
    }
    let mut actions = Vec::new();
    let mode = app.artist_view_mode;
    let sub = format!("{} {}", format_count(app.library.artists_for(mode).len()), tr.t("artists.count"));
    let focus = app.ui.focus_search;
    let query = &mut app.ui.artist_query;
    let mut new_mode = None;
    // The Artist / Album artist switch sits under the header (left-aligned), so it never
    // competes with the search box for width.
    section_header(ui, tr.t("nav.artists"), Some(&sub), |ui| {
        let w = (ui.available_width() * 0.5).clamp(140.0, 320.0);
        search_box_with_hint(ui, egui::Id::new("artist_search"), query, w, focus, &tr, tr.t("search.artists"));
    });
    ui.add_space(10.0);
    let labels = [tr.t("artists.mode_artist"), tr.t("artists.mode_album_artist")];
    let current = usize::from(mode == ArtistViewMode::AlbumArtist);
    if let Some(i) = segmented(ui, egui::Id::new("artist_view_mode"), &labels, current) {
        new_mode = Some(if i == 1 { ArtistViewMode::AlbumArtist } else { ArtistViewMode::Artist });
    }
    if let Some(m) = new_mode {
        app.artist_view_mode = m;
        app.save_state();
    }
    let mode = app.artist_view_mode;
    ui.add_space(12.0);
    if !app.library.loaded {
        loading(ui, tr.t("library.loading"));
        return actions;
    }
    let q = normalize(app.ui.artist_query.trim());
    let visible: Vec<usize> = app
        .library
        .artists_for(mode)
        .iter()
        .enumerate()
        .filter(|(_, a)| q.is_empty() || normalize(&a.name).contains(&q))
        .map(|(i, _)| i)
        .collect();
    if visible.is_empty() {
        let (title, hint) = if app.library.artists_for(mode).is_empty() {
            (tr.t("artists.empty"), tr.t("library.empty_hint"))
        } else {
            (tr.t("table.no_results"), tr.t("table.no_results_hint"))
        };
        empty_state(ui, Icon::Artist, title, hint, None);
        return actions;
    }
    let pal = theme::palette(ui);
    let width = ui.available_width();
    ScrollArea::vertical().id_salt("artists_list").auto_shrink([false, false]).show_rows(ui, ROW_H, visible.len(), |ui, range| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        for pos in range {
            let a = &app.library.artists_for(mode)[visible[pos]];
            let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
            let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
            if resp.hovered() {
                ui.painter().rect_filled(rect.shrink2(vec2(2.0, 2.0)), 8.0, pal.hover);
            }
            let av = Rect::from_center_size(pos2(rect.left() + 30.0, rect.center().y), vec2(40.0, 40.0));
            let mut aui = ui.new_child(egui::UiBuilder::new().max_rect(av));
            avatar(&mut aui, 40.0, &a.name, None);
            let name = ui.painter().layout_job({
                let mut j = egui::text::LayoutJob::simple_singleline(a.name.clone(), theme::bold_font(ui.ctx(), theme::size::BODY), pal.text);
                j.wrap = egui::text::TextWrapping::truncate_at_width(width - 120.0);
                j
            });
            ui.painter().galley(pos2(av.right() + 14.0, rect.center().y - name.size().y - 1.0), name, pal.text);
            let sub = format!(
                "{} {} · {} {}",
                format_count(a.albums.len()),
                tr.t("albums.count"),
                format_count(a.tracks.len()),
                tr.t("table.tracks_count")
            );
            let g = ui.painter().layout_no_wrap(sub, FontId::proportional(12.0), pal.text_muted);
            ui.painter().galley(pos2(av.right() + 14.0, rect.center().y + 2.0), g, pal.text_muted);
            if resp.clicked() {
                actions.push(UiAction::OpenArtist(a.name.clone()));
            }
        }
    });
    actions
}

fn detail(app: &mut GuiApp, ui: &mut Ui, tr: Tr, name: &str) -> Vec<UiAction> {
    let mut actions = Vec::new();
    if let Some(a) = back_nav(app, ui, &tr, Page::Artists) {
        actions.push(a);
        return actions;
    }
    ui.add_space(8.0);
    let mode = app.artist_view_mode;
    let Some(entry) = app.library.artist(name, mode).or_else(|| app.library.artist(name, other_mode(mode))).cloned() else {
        if app.library.loaded {
            ui.label(theme::muted(ui, tr.t("artists.not_found")));
        } else {
            loading(ui, tr.t("library.loading"));
        }
        return actions;
    };
    let library = &app.library;
    app.ui.detail_rows.ensure(hash_key(("artist", name, mode == ArtistViewMode::AlbumArtist, library.revision)), || {
        // Album order of the artist's albums strip, then disc / track / title.
        let r = &library.rows;
        let album_pos = |i: usize| {
            library.album_key_of(&r[i].path).and_then(|k| entry.albums.iter().position(|a| *a == k)).unwrap_or(usize::MAX)
        };
        let mut idx: Vec<(usize, usize)> = entry.tracks.iter().map(|&i| (album_pos(i), i)).collect();
        idx.sort_by(|&(pa, a), &(pb, b)| {
            pa.cmp(&pb)
                .then((r[a].disc_no, r[a].track_no).cmp(&(r[b].disc_no, r[b].track_no)))
                .then_with(|| collate(&r[a].title, &r[b].title))
                .then_with(|| a.cmp(&b))
        });
        idx.iter().map(|&(_, i)| r[i].clone()).collect()
    });
    let meta = format!(
        "{} {} · {}",
        format_count(entry.albums.len()),
        tr.t("albums.count"),
        tracks_summary(&app.ui.detail_rows.rows, &tr)
    );
    let paths: Vec<std::path::PathBuf> = app.ui.detail_rows.rows.iter().map(|r| r.path.clone()).collect();
    detail_header(
        ui,
        |ui| {
            avatar(ui, detail_art_size(ui, 140.0), name, None);
        },
        tr.t("artists.kicker"),
        name,
        None,
        &meta,
        |ui| {
            if button(ui, ButtonKind::Primary, Some(Icon::Play), tr.t("artists.play_all")).clicked() {
                actions.push(UiAction::PlayTracks { paths: paths.clone(), start: 0 });
            }
            if button(ui, ButtonKind::Secondary, Some(Icon::Queue), tr.t("menu.add_to_queue")).clicked() {
                actions.push(UiAction::Enqueue(paths.clone()));
            }
        },
    );

    // Albums strip.
    if !entry.albums.is_empty() {
        ui.add_space(16.0);
        let pal = theme::palette(ui);
        ui.label(theme::title(ui, tr.t("nav.albums")).color(pal.text));
        ui.add_space(6.0);
        let card = 118.0;
        ScrollArea::horizontal().id_salt(("artist_albums", name)).max_height(card + 46.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                for key in &entry.albums {
                    let Some(album) = app.library.album(key) else { continue };
                    let (rect, resp) = ui.allocate_exact_size(vec2(card, card + 40.0), Sense::click());
                    let resp = resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(&album.title);
                    let art = Rect::from_min_size(rect.min, vec2(card, card));
                    if ui.is_rect_visible(rect) {
                        let tex = app.artwork.album_cover(&album.title);
                        paint_artwork(ui, art, tex.as_ref().map(Tex::from), 8, &album.title);
                        if resp.hovered() {
                            ui.painter().rect_filled(art, 8.0, egui::Color32::from_black_alpha(40));
                        }
                        let mut j = egui::text::LayoutJob::simple_singleline(album.title.clone(), FontId::proportional(12.5), pal.text);
                        j.wrap = egui::text::TextWrapping::truncate_at_width(card);
                        let g = ui.painter().layout_job(j);
                        ui.painter().galley(pos2(art.left(), art.bottom() + 6.0), g, pal.text);
                        if let Some(y) = album.year {
                            let g = ui.painter().layout_no_wrap(y.to_string(), FontId::proportional(11.5), pal.text_muted);
                            ui.painter().galley(pos2(art.left(), art.bottom() + 23.0), g, pal.text_muted);
                        }
                    }
                    if resp.clicked() {
                        actions.push(UiAction::OpenAlbum(key.clone()));
                    }
                }
            });
        });
    }

    ui.add_space(12.0);
    let entries = playlist_entries(&app.playlists, &tr);
    let now = (!app.play_path.is_empty()).then(|| Path::new(app.play_path.as_str()));
    let columns = [Column::Index, Column::Title, Column::Album, Column::Duration];
    actions.extend(track_table(
        ui,
        &TrackTableProps {
            id_salt: "artist_detail",
            rows: &app.ui.detail_rows.rows,
            rows_revision: app.ui.detail_rows.revision,
            query: "",
            columns: &columns,
            now_playing: now,
            playing: app.is_playing(),
            index_mode: IndexMode::Position,
            reorderable: false,
            playlists: &entries,
            removable: false,
            optional_columns: &[],
            // The artist column would point to this page.
            links: TableLinks { title: true, artist: false, album: true },
            tr,
        },
        &mut app.ui.detail_table,
    ));
    actions
}
