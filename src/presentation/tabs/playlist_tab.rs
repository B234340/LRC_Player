use crate::presentation::egui_app::GuiApp;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    ui.heading("Playlists");
    ui.label(format!("Saved playlists: {}", app.playlists.len()));
    ui.separator();

    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.new_playlist_name);
        if ui.button("Create playlist").clicked() {
            app.add_playlist();
        }
    });

    if app.playlists.is_empty() {
        ui.label("No playlists available.");
        return;
    }

    ui.separator();
    let mut pending_select_playlist: Option<usize> = None;
    let mut pending_delete_playlist: Option<usize> = None;
    for (index, playlist) in app.playlists.iter().enumerate() {
        let selected = app.selected_playlist == Some(index);
        let is_favorite = playlist.name.eq_ignore_ascii_case("favorite");
        ui.horizontal(|ui| {
            let label = if is_favorite {
                format!("★ {}", playlist.name)
            } else {
                playlist.name.clone()
            };
            if ui.selectable_label(selected, label).clicked() {
                pending_select_playlist = Some(index);
            }
            if ui.add_enabled(!is_favorite, egui::Button::new("Delete")).clicked() {
                pending_delete_playlist = Some(index);
            }
        });
    }

    if let Some(index) = pending_delete_playlist {
        app.remove_playlist(index);
        return;
    }

    if let Some(index) = pending_select_playlist {
        app.selected_playlist = Some(index);
        app.update_app_vm();
    }

    if let Some(selected_index) = app.selected_playlist {
        let mut selected_name: Option<String> = None;
        let mut selected_len: usize = 0;
        let mut pending_remove_track: Option<usize> = None;
        let mut pending_play_index: Option<usize> = None;
        let mut pending_play_list: Vec<String> = Vec::new();
        let mut pending_play_now_path: Option<String> = None;
        let mut pending_next_path: Option<String> = None;
        let mut pending_enqueue_path: Option<String> = None;
        let mut pending_add_to_playlist: Option<(usize, String)> = None;
        let mut pending_show_details_path: Option<String> = None;
        let playlist_targets: Vec<(usize, String)> = app
            .playlists
            .iter()
            .enumerate()
            .map(|(idx, p)| (idx, p.name.clone()))
            .collect();
        if let Some(playlist) = app.playlists.get(selected_index) {
            selected_name = Some(playlist.name.clone());
            selected_len = playlist.tracks.len();
            ui.separator();
            ui.heading(format!("Playlist: {}", playlist.name));
            ui.label(format!("{} tracks", playlist.tracks.len()));

            if playlist.tracks.is_empty() {
                ui.label("No tracks in this playlist.");
            } else {
                ui.label("Left click: create queue from this list and play. Right click: common actions.");
                for (track_index, track) in playlist.tracks.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let label = format!("{}: {}", track_index + 1, track);
                        let response = ui.selectable_label(false, label);
                        if response.clicked() {
                            pending_play_index = Some(track_index);
                            pending_play_list = playlist.tracks.clone();
                        }
                        response.context_menu(|ui| {
                            if ui.button("Play now").clicked() {
                                pending_play_now_path = Some(track.clone());
                                ui.close_menu();
                            }
                            if ui.button("Play next").clicked() {
                                pending_next_path = Some(track.clone());
                                ui.close_menu();
                            }
                            if ui.button("Add to queue").clicked() {
                                pending_enqueue_path = Some(track.clone());
                                ui.close_menu();
                            }
                            ui.separator();
                            ui.label("Add to playlist");
                            for (target_index, target_name) in &playlist_targets {
                                if ui.button(target_name).clicked() {
                                    pending_add_to_playlist =
                                        Some((*target_index, track.clone()));
                                    ui.close_menu();
                                }
                            }
                            ui.separator();
                            if ui.button("Show details").clicked() {
                                pending_show_details_path = Some(track.clone());
                                ui.close_menu();
                            }
                        });
                        if ui.button("Remove").clicked() {
                            pending_remove_track = Some(track_index);
                        }
                    });
                }
            }
        }

        if selected_name.is_some() {
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut app.playlist_track_input);
                if ui.button("Add track").clicked() {
                    app.add_track_to_selected_playlist();
                }
                if selected_len == 0 {
                    ui.label("(empty)");
                }
            });
        }

        if let Some(track_index) = pending_remove_track {
            app.remove_track_from_selected_playlist(track_index);
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
    }
}
