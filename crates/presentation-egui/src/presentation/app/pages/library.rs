//! Library: every track in a virtualized, sortable, searchable table.

use std::path::Path;

use eframe::egui::{self, Ui};

use super::loading;
use crate::presentation::app::{shell::playlist_entries, GuiApp};
use crate::presentation::ui::{
    format::format_count,
    i18n::Tr,
    views::{track_table, Column, IndexMode, TrackTableProps},
    widgets::{search_box, section_header},
    UiAction,
};

pub(super) fn show(app: &mut GuiApp, ui: &mut Ui, tr: Tr) -> Vec<UiAction> {
    let total = app.library.rows.len();
    let shown = if app.ui.library_query.trim().is_empty() {
        total
    } else {
        app.ui.library_table.view(&app.library.rows, app.library.revision, &app.ui.library_query).len()
    };
    let count = if shown == total {
        format!("{} {}", format_count(total), tr.t("table.tracks_count"))
    } else {
        format!("{} / {} {}", format_count(shown), format_count(total), tr.t("table.tracks_count"))
    };
    let focus = app.ui.focus_search;
    let query = &mut app.ui.library_query;
    section_header(ui, tr.t("nav.library"), Some(&count), |ui| {
        let w = (ui.available_width() * 0.5).clamp(180.0, 320.0);
        search_box(ui, egui::Id::new("library_search"), query, w, focus, &tr);
    });
    ui.add_space(12.0);
    if !app.library.loaded {
        loading(ui, tr.t("library.loading"));
        return Vec::new();
    }
    let playing = app.is_playing();
    let entries = playlist_entries(&app.playlists, &tr);
    let now = (!app.play_path.is_empty()).then(|| Path::new(app.play_path.as_str()));
    track_table(
        ui,
        &TrackTableProps {
            id_salt: "library",
            rows: &app.library.rows,
            rows_revision: app.library.revision,
            query: &app.ui.library_query,
            columns: &Column::DEFAULT,
            now_playing: now,
            playing,
            index_mode: IndexMode::Position,
            reorderable: false,
            playlists: &entries,
            removable: false,
            tr,
        },
        &mut app.ui.library_table,
    )
}
