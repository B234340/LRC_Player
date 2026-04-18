use crate::infrastructure::persistence::MetaCache;
use crate::presentation::egui_app::{ArtistViewMode, GuiApp};
use eframe::egui;
use std::collections::HashMap;

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    let selected_artist = app.app_vm.library.selected_artist.clone();
    let selected_artist_tracks = app.app_vm.library.selected_artist_tracks.clone();
    let artist_names: Vec<String> = app
        .app_vm
        .library
        .artists
        .iter()
        .map(|a| a.name.clone())
        .collect();
    let mut action: Option<Option<String>> = None;
    let mut pending_play_index: Option<usize> = None;
    let mut pending_play_list: Vec<String> = Vec::new();
    let mut pending_play_now_path: Option<String> = None;
    let mut pending_next_path: Option<String> = None;
    let mut pending_enqueue_path: Option<String> = None;
    let mut pending_add_to_playlist: Option<(usize, String)> = None;
    let mut pending_show_details_path: Option<String> = None;
    let mut pending_add_favorite = false;
    let playlist_targets: Vec<(usize, String)> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(idx, p)| (idx, p.name.clone()))
        .collect();
    let meta_cache = MetaCache::open_default().ok();
    let mut track_metadata_cache: HashMap<String, (String, String)> = HashMap::new();

    ui.heading("Artists");
    ui.horizontal(|ui| {
        ui.label("Type:");
        let mut mode = app.artist_view_mode;
        if ui
            .selectable_value(&mut mode, ArtistViewMode::Artist, ArtistViewMode::Artist.label())
            .changed()
        {
            app.set_artist_view_mode(mode);
        }
        if ui
            .selectable_value(
                &mut mode,
                ArtistViewMode::AlbumArtist,
                ArtistViewMode::AlbumArtist.label(),
            )
            .changed()
        {
            app.set_artist_view_mode(mode);
        }
        if ui
            .selectable_value(&mut mode, ArtistViewMode::Composer, ArtistViewMode::Composer.label())
            .changed()
        {
            app.set_artist_view_mode(mode);
        }
    });
    ui.label(format!("{} artists in library", artist_names.len()));
    ui.separator();

    if artist_names.is_empty() {
        ui.label("No artists available.");
    } else {
        let list_row_height = 24.0_f32;
        let track_row_height = 22.0_f32;
        let body_height = ui.available_height();

        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), body_height),
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| {
                let left_width = (ui.available_width() * 0.42).clamp(220.0, 460.0);

                ui.allocate_ui_with_layout(
                    egui::vec2(left_width, body_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Artist list");
                            if ui.button("Clear").clicked() {
                                action = Some(None);
                            }
                        });
                        ui.separator();

                        let list_block_width = ui.available_width();
                        egui::ScrollArea::vertical()
                            .id_source("artist_list_scroll")
                            .auto_shrink([false, false])
                            .max_width(list_block_width)
                            .max_height(ui.available_height())
                            .show_rows(ui, list_row_height, artist_names.len(), |ui, row_range| {
                                for row in row_range {
                                    let name = artist_names[row].as_str();
                                    let is_selected = selected_artist.as_deref() == Some(name);
                                    let row = egui::SelectableLabel::new(is_selected, name);
                                    if ui
                                        .add_sized([ui.available_width(), list_row_height], row)
                                        .clicked()
                                    {
                                        action = Some(Some(name.to_string()));
                                    }
                                }
                            });
                    },
                );

                ui.separator();

                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), body_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.heading("Tracks");
                        if let Some(selected) = selected_artist.as_deref() {
                            ui.label(format!("Artist: {}", selected));
                            if !selected_artist_tracks.is_empty()
                                && ui.button("Add all to favorite").clicked()
                            {
                                pending_add_favorite = true;
                            }
                            ui.separator();

                            if selected_artist_tracks.is_empty() {
                                ui.label("No tracks found for this artist.");
                            } else {
                                ui.label("Left click: create queue from this list and play. Right click: common actions.");
                                egui::ScrollArea::vertical()
                                    .id_source("artist_tracks_scroll")
                                    .auto_shrink([false, false])
                                    .max_height(ui.available_height())
                                    .show_rows(
                                        ui,
                                        track_row_height,
                                        selected_artist_tracks.len(),
                                        |ui, row_range| {
                                            let displayed_tracks: Vec<String> = selected_artist_tracks
                                                .iter()
                                                .map(|track| track.title.clone())
                                                .collect();
                                            for row in row_range {
                                                let track_path = &selected_artist_tracks[row].title;
                                                let file_name = std::path::Path::new(track_path)
                                                    .file_name()
                                                    .and_then(|n| n.to_str())
                                                    .unwrap_or(track_path);

                                                let (title, album) = track_metadata_cache
                                                    .entry(track_path.clone())
                                                    .or_insert_with(|| {
                                                        meta_cache
                                                            .as_ref()
                                                            .and_then(|cache| {
                                                                cache.track_metadata_by_location(track_path)
                                                                    .ok()
                                                                    .flatten()
                                                            })
                                                            .map(|(t, _a, al)| {
                                                                let t = if t.trim().is_empty() {
                                                                    file_name.to_string()
                                                                } else {
                                                                    t
                                                                };
                                                                let al = if al.trim().is_empty() {
                                                                    "Unknown album".to_string()
                                                                } else {
                                                                    al
                                                                };
                                                                (t, al)
                                                            })
                                                            .unwrap_or_else(|| {
                                                                (file_name.to_string(), "Unknown album".to_string())
                                                            })
                                                    })
                                                    .clone();

                                                let label = format!("[{}] {}", album, title);
                                                let response = ui.selectable_label(false, label);
                                                if response.clicked() {
                                                    pending_play_index = Some(row);
                                                    pending_play_list = displayed_tracks.clone();
                                                }
                                                response.context_menu(|ui| {
                                                    if ui.button("Play now").clicked() {
                                                        pending_play_now_path = Some(track_path.clone());
                                                        ui.close_menu();
                                                    }
                                                    if ui.button("Play next").clicked() {
                                                        pending_next_path = Some(track_path.clone());
                                                        ui.close_menu();
                                                    }
                                                    if ui.button("Add to queue").clicked() {
                                                        pending_enqueue_path = Some(track_path.clone());
                                                        ui.close_menu();
                                                    }
                                                    ui.separator();
                                                    ui.label("Add to playlist");
                                                    for (target_index, target_name) in &playlist_targets {
                                                        if ui.button(target_name).clicked() {
                                                            pending_add_to_playlist =
                                                                Some((*target_index, track_path.clone()));
                                                            ui.close_menu();
                                                        }
                                                    }
                                                    ui.separator();
                                                    if ui.button("Show details").clicked() {
                                                        pending_show_details_path = Some(track_path.clone());
                                                        ui.close_menu();
                                                    }
                                                });
                                            }
                                        },
                                    );
                            }
                        } else {
                            ui.label("Select an artist to view tracks.");
                        }
                    },
                );
            },
        );
    }

    if let Some(next) = action {
        app.select_artist(next);
    }
    if let Some(index) = pending_play_index {
        app.play_from_displayed_list(&pending_play_list, index);
    }
    if let Some(path) = pending_play_now_path {
        app.play_track_now(&path);
    }
    if let Some(path) = pending_next_path {
        app.add_track_next_in_queue(&path);
    }
    if let Some(path) = pending_enqueue_path {
        app.enqueue_track_path(&path);
    }
    if let Some((target_index, track)) = pending_add_to_playlist {
        app.add_track_to_playlist(target_index, &track);
    }
    if let Some(path) = pending_show_details_path {
        app.open_track_details(&path);
    }
    if pending_add_favorite {
        let tracks: Vec<String> = selected_artist_tracks
            .iter()
            .map(|track| track.title.clone())
            .collect();
        app.add_tracks_to_favorite_playlist(&tracks);
    }
}
