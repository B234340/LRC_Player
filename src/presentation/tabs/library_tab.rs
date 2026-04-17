use crate::application::view_model::LibraryDirectoryVm;
use crate::presentation::egui_app::GuiApp;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    let mut pending_select_directory: Option<String> = None;

    let library = &app.app_vm.library;
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
                for track in library.selected_directory_tracks.iter() {
                    ui.label(&track.title);
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
}

fn render_directory_node(
    ui: &mut egui::Ui,
    node: &LibraryDirectoryVm,
    indent: usize,
    selected_directory: Option<&str>,
    pending_select_directory: &mut Option<String>,
) {
    ui.horizontal(|ui| {
        for _ in 0..indent {
            ui.add_space(12.0);
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

    for child in node.children.iter() {
        render_directory_node(
            ui,
            child,
            indent + 1,
            selected_directory,
            pending_select_directory,
        );
    }
}
