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
        ui.horizontal(|ui| {
            if ui.selectable_label(selected, &playlist.name).clicked() {
                pending_select_playlist = Some(index);
            }
            if ui.button("Delete").clicked() {
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
        if let Some(playlist) = app.playlists.get(selected_index) {
            selected_name = Some(playlist.name.clone());
            selected_len = playlist.tracks.len();
            ui.separator();
            ui.heading(format!("Playlist: {}", playlist.name));
            ui.label(format!("{} tracks", playlist.tracks.len()));

            if playlist.tracks.is_empty() {
                ui.label("No tracks in this playlist.");
            } else {
                for (track_index, track) in playlist.tracks.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}: {}", track_index + 1, track));
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
    }
}
