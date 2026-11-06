//! Playlists: list (create / rename / delete with confirmation) and the playlist page
//! (drag to reorder, Delete / context menu to remove, drop tracks onto the sidebar entry).

use std::path::Path;

use eframe::egui::{self, pos2, vec2, CursorIcon, FontId, Rect, ScrollArea, Sense, Ui, Vec2};

use super::{albums::{detail_art_size, detail_header}, back_nav, hash_key, tracks_summary};
use crate::presentation::app::{playlists as pl, shell::playlist_entries, GuiApp, Modal};
use crate::presentation::ui::{
    format::format_count,
    i18n::Tr,
    icons::{icon_button, paint_icon, Icon},
    theme,
    views::{track_table, Column, IndexMode, TrackDragPayload, TableLinks, TrackTableProps},
    widgets::{button, empty_state, section_header, ButtonKind},
    Page, UiAction,
};

const ROW_H: f32 = 60.0;

fn playlist_tile(ui: &mut Ui, size: f32, favorite: bool, seed: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let pal = theme::palette(ui);
    let hue = crate::presentation::ui::widgets::artwork::seed_hue(seed);
    let c0: egui::Color32 = egui::ecolor::Hsva::new(hue, 0.45, if pal.dark { 0.45 } else { 0.75 }, 1.0).into();
    let c1: egui::Color32 = egui::ecolor::Hsva::new((hue + 0.1) % 1.0, 0.5, if pal.dark { 0.25 } else { 0.55 }, 1.0).into();
    crate::presentation::ui::widgets::artwork::gradient_rounded_rect(ui.painter(), rect, size * 0.08, c0, c1);
    let icon = if favorite { Icon::HeartFilled } else { Icon::Playlist };
    paint_icon(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(size * 0.4)), icon, egui::Color32::WHITE);
}

pub(super) fn show(app: &mut GuiApp, ui: &mut Ui, tr: Tr) -> Vec<UiAction> {
    if let Some(id) = app.ui.playlist_detail.clone() {
        if app.playlist_index(&id).is_some() {
            return detail(app, ui, tr, &id);
        }
        app.ui.playlist_detail = None;
    }
    let mut actions = Vec::new();
    let sub = format!("{} {}", app.playlists.len(), tr.t("playlist.count"));
    let mut create = false;
    section_header(ui, tr.t("nav.playlists"), Some(&sub), |ui| {
        create = button(ui, ButtonKind::Primary, Some(Icon::Plus), tr.t("playlist.new")).clicked();
    });
    if create {
        let name = app.suggest_playlist_name(tr.t("playlist.default_name"));
        app.ui.modal = Some(Modal::CreatePlaylist { name, tracks: Vec::new() });
    }
    ui.add_space(12.0);
    let pal = theme::palette(ui);
    let width = ui.available_width();
    ScrollArea::vertical().id_salt("playlists_list").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
        for p in &app.playlists {
            let favorite = pl::is_favorite(p);
            let name = pl::display_name(p, &tr).to_string();
            let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
            let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
            let drop_hover = resp.dnd_hover_payload::<TrackDragPayload>().is_some();
            // Geometric hover so the row stays highlighted over its action buttons.
            if ui.rect_contains_pointer(rect) || drop_hover {
                ui.painter().rect_filled(rect, 8.0, pal.hover);
            }
            if drop_hover {
                ui.painter().rect_stroke(rect, 8.0, egui::Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
            }
            if let Some(payload) = resp.dnd_release_payload::<TrackDragPayload>() {
                actions.push(UiAction::AddToPlaylist { playlist_id: p.id.clone(), paths: payload.paths.clone() });
            }
            let tile = Rect::from_min_size(pos2(rect.left() + 10.0, rect.center().y - 22.0), Vec2::splat(44.0));
            let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(tile));
            playlist_tile(&mut tui, 44.0, favorite, &p.id);
            let g = ui.painter().layout_no_wrap(name.clone(), crate::presentation::ui::text::font_id(&name, theme::size::BODY, true), pal.text);
            ui.painter().galley(pos2(tile.right() + 14.0, rect.center().y - g.size().y - 1.0), g, pal.text);
            let g = ui.painter().layout_no_wrap(
                format!("{} {}", format_count(p.tracks.len()), tr.t("table.tracks_count")),
                FontId::proportional(12.0),
                pal.text_muted,
            );
            ui.painter().galley(pos2(tile.right() + 14.0, rect.center().y + 2.0), g, pal.text_muted);

            // Row actions (right side).
            let mut bar = Rect::from_min_max(pos2(rect.right() - 130.0, rect.top()), rect.max);
            bar.max.x -= 8.0;
            let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)));
            let mut row_action = false;
            if !favorite {
                if icon_button(&mut bui, Icon::Trash, 16.0, false, tr.t("playlist.delete")).clicked() {
                    app.ui.modal = Some(Modal::DeletePlaylist { id: p.id.clone(), name: name.clone() });
                    row_action = true;
                }
                if icon_button(&mut bui, Icon::Edit, 16.0, false, tr.t("playlist.rename")).clicked() {
                    app.ui.modal = Some(Modal::RenamePlaylist { id: p.id.clone(), name: p.name.clone() });
                    row_action = true;
                }
            }
            if !p.tracks.is_empty() && icon_button(&mut bui, Icon::Play, 16.0, false, tr.t("menu.play")).clicked() {
                actions.push(UiAction::PlayTracks { paths: p.tracks.iter().map(Into::into).collect(), start: 0 });
                row_action = true;
            }
            if resp.clicked() && !row_action {
                actions.push(UiAction::OpenPlaylist(p.id.clone()));
            }
        }
    });
    actions
}

fn detail(app: &mut GuiApp, ui: &mut Ui, tr: Tr, id: &str) -> Vec<UiAction> {
    let mut actions = Vec::new();
    if let Some(a) = back_nav(app, ui, &tr, Page::Playlists) {
        actions.push(a);
        return actions;
    }
    ui.add_space(8.0);
    let Some(index) = app.playlist_index(id) else { return actions };
    let list = app.playlists[index].clone();
    let favorite = pl::is_favorite(&list);
    let name = pl::display_name(&list, &tr).to_string();
    app.library.ensure_probed(list.tracks.iter());
    let library = &app.library;
    app.ui.detail_rows.ensure(hash_key(("playlist", id, &list.tracks, library.revision)), || {
        list.tracks.iter().map(|p| library.display_row(p)).collect()
    });
    let meta = tracks_summary(&app.ui.detail_rows.rows, &tr);
    let paths: Vec<std::path::PathBuf> = list.tracks.iter().map(Into::into).collect();
    let selected = !app.ui.detail_table.selected.is_empty();
    let mut remove_selected = false;
    detail_header(
        ui,
        |ui| playlist_tile(ui, detail_art_size(ui, 140.0), favorite, id),
        tr.t("playlist.kicker"),
        &name,
        None,
        &meta,
        |ui| {
            ui.add_enabled_ui(!paths.is_empty(), |ui| {
                if button(ui, ButtonKind::Primary, Some(Icon::Play), tr.t("menu.play")).clicked() {
                    actions.push(UiAction::PlayTracks { paths: paths.clone(), start: 0 });
                }
                if button(ui, ButtonKind::Secondary, Some(Icon::Queue), tr.t("menu.add_to_queue")).clicked() {
                    actions.push(UiAction::Enqueue(paths.clone()));
                }
            });
            if !favorite {
                if button(ui, ButtonKind::Secondary, Some(Icon::Edit), tr.t("playlist.rename")).clicked() {
                    app.ui.modal = Some(Modal::RenamePlaylist { id: id.to_string(), name: list.name.clone() });
                }
                if button(ui, ButtonKind::Danger, Some(Icon::Trash), tr.t("playlist.delete")).clicked() {
                    app.ui.modal = Some(Modal::DeletePlaylist { id: id.to_string(), name: name.clone() });
                }
            }
            if selected && button(ui, ButtonKind::Ghost, Some(Icon::Close), tr.t("playlist.remove_selected")).clicked() {
                remove_selected = true;
            }
        },
    );
    ui.add_space(16.0);
    if list.tracks.is_empty() {
        empty_state(ui, Icon::Playlist, tr.t("playlist.empty"), tr.t("playlist.empty_hint"), Some((Icon::Library, tr.t("nav.library"))))
            .then(|| actions.push(UiAction::Navigate(Page::Library)));
        return actions;
    }
    let entries = playlist_entries(&app.playlists, &tr);
    let now = (!app.play_path.is_empty()).then(|| Path::new(app.play_path.as_str()));
    let table_actions = track_table(
        ui,
        &TrackTableProps {
            id_salt: "playlist_detail",
            rows: &app.ui.detail_rows.rows,
            rows_revision: app.ui.detail_rows.revision,
            query: "",
            columns: &Column::DEFAULT,
            now_playing: now,
            playing: app.is_playing(),
            index_mode: IndexMode::Position,
            reorderable: true,
            playlists: &entries,
            removable: true,
            optional_columns: &[],
            links: TableLinks::ALL,
            tr,
        },
        &mut app.ui.detail_table,
    );
    drop(entries);
    if remove_selected {
        let rows: Vec<usize> = app.ui.detail_table.selected.iter().copied().collect();
        app.remove_from_playlist(id, rows);
    }
    for a in table_actions {
        match a {
            UiAction::Reorder { from, to } => app.reorder_playlist(id, from, to),
            UiAction::RemoveRows(rows) => app.remove_from_playlist(id, rows),
            other => actions.push(other),
        }
    }
    actions
}
