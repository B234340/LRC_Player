use crate::application::view_model::LibraryDirectoryVm;
use crate::presentation::egui_app::GuiApp;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    let mut pending_select_directory: Option<String> = None;
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

    let library = app.app_vm.library.clone();
    let selected_directory = library.selected_directory.as_deref();

    ui.heading("Library");
    ui.label("Browse scanned library metadata and recent track locations.");
    ui.separator();
    ui.label(format!("Albums: {}", library.album_count));
    ui.label(format!("Tracks: {}", library.track_count));

    ui.separator();
    ui.heading("Favorites");
    if library.favorite_directories.is_empty() {
        ui.label("No favorite directories configured.");
    } else {
        ui.group(|ui| {
            for favorite in library.favorite_directories.iter() {
                ui.label(format!("• {}", favorite));
            }
        });
    }

    ui.separator();
    ui.heading("Recent directories");
    if library.recent_directories.is_empty() {
        ui.label("No recent directories available.");
    } else {
        ui.group(|ui| {
            for dir in library.recent_directories.iter().take(20) {
                ui.label(dir);
            }
        });
    }

    ui.separator();
    ui.heading("Excluded paths");
    if app.scan_excludes.is_empty() {
        ui.label("No excluded paths configured.");
    } else {
        ui.group(|ui| {
            for exclude in app.scan_excludes.iter() {
                ui.label(exclude);
            }
        });
    }

    ui.separator();
    ui.heading("Directory tree");
    if library.directory_tree.is_empty() {
        ui.label("No library directories available.");
    } else {
        for root in library.directory_tree.iter() {
            render_directory_node(
                ui,
                app,
                root,
                0,
                selected_directory,
                &mut pending_select_directory,
            );
        }
    }

    if let Some(selected) = selected_directory {
        ui.separator();
        ui.heading("Selected directory");
        ui.label(selected);
        if library.selected_directory_tracks.is_empty() {
            ui.label("No tracks found for this directory.");
        } else {
            ui.group(|ui| {
                ui.label("Left click: create queue from this list and play. Right click: common actions.");
                for (index, track) in library.selected_directory_tracks.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.vertical_centered(|ui| {
                            let is_favorite = app.is_track_favorite(&track.title);
                            let star_label = if is_favorite { "★" } else { "☆" };
                            if ui
                                .add_sized([22.0, 22.0], egui::Button::new(star_label))
                                .on_hover_text("Toggle favorite")
                                .clicked()
                            {
                                app.toggle_favorite_track(&track.title);
                            }
                        });
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            let response = ui.selectable_label(false, &track.title);
                            if response.clicked() {
                                pending_play_index = Some(index);
                                pending_play_list = library
                                    .selected_directory_tracks
                                    .iter()
                                    .map(|row| row.title.clone())
                                    .collect();
                            }
                            response.context_menu(|ui| {
                                if ui.button("Play now").clicked() {
                                    pending_play_now_path = Some(track.title.clone());
                                    ui.close_menu();
                                }
                                if ui.button("Play next").clicked() {
                                    pending_next_path = Some(track.title.clone());
                                    ui.close_menu();
                                }
                                if ui.button("Add to queue").clicked() {
                                    pending_enqueue_path = Some(track.title.clone());
                                    ui.close_menu();
                                }
                                ui.separator();
                                ui.label("Add to playlist");
                                for (target_index, target_name) in &playlist_targets {
                                    if ui.button(target_name).clicked() {
                                        pending_add_to_playlist = Some((*target_index, track.title.clone()));
                                        ui.close_menu();
                                    }
                                }
                                ui.separator();
                                if ui.button("Show details").clicked() {
                                    pending_show_details_path = Some(track.title.clone());
                                    ui.close_menu();
                                }
                            });
                        });
                    });
                }
            });
        }
    }

    ui.separator();
    ui.heading("Recent tracks");
    if library.recent_locations.is_empty() {
        ui.label("No recent track locations available.");
    } else {
        ui.group(|ui| {
            for path in library.recent_locations.iter().take(20) {
                ui.label(path);
            }
        });
    }

    if let Some(path) = pending_select_directory {
        app.select_directory(Some(path));
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

fn render_directory_node(
    ui: &mut egui::Ui,
    app: &mut GuiApp,
    node: &LibraryDirectoryVm,
    indent: usize,
    selected_directory: Option<&str>,
    pending_select_directory: &mut Option<String>,
) {
    let has_children = !node.children.is_empty();
    let expanded = app.is_library_directory_expanded(&node.path);

    ui.horizontal(|ui| {
        for _ in 0..indent {
            ui.add_space(12.0);
        }

        if has_children {
            let icon = if expanded { "▼" } else { "▶" };
            if ui.button(icon).clicked() {
                app.toggle_library_directory_expanded(&node.path);
            }
        } else {
            ui.add_space(18.0);
        }

        if ui
            .selectable_label(
                selected_directory.map(|s| s == node.path).unwrap_or(false),
                &node.path,
            )
            .clicked()
        {
            *pending_select_directory = Some(node.path.clone());
        }
    });

    if expanded {
        for child in node.children.iter() {
            render_directory_node(
                ui,
                app,
                child,
                indent + 1,
                selected_directory,
                pending_select_directory,
            );
        }
    }
}
