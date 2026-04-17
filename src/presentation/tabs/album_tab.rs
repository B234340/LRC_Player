use crate::presentation::egui_app::GuiApp;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    let selected_album = app.app_vm.library.selected_album.clone();
    let selected_album_tracks = app.app_vm.library.selected_album_tracks.clone();
    let album_titles: Vec<String> = app
        .app_vm
        .library
        .albums
        .iter()
        .map(|a| a.title.clone())
        .collect();
    let mut action: Option<Option<String>> = None;

    ui.heading("Albums");
    ui.label(format!("{} albums in library", album_titles.len()));
    ui.separator();

    if album_titles.is_empty() {
        ui.label("No albums available.");
    } else {
        for album_title in album_titles.iter() {
            eframe::egui::CollapsingHeader::new(album_title)
                .id_source(album_title)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Select").clicked() {
                            action = Some(Some(album_title.clone()));
                        }
                        if ui.button("Clear").clicked() {
                            action = Some(None);
                        }
                    });

                    if let Some(selected) = selected_album.as_deref() {
                        if selected == album_title {
                            if selected_album_tracks.is_empty() {
                                ui.label("No tracks found for this album.");
                            } else {
                                for track in selected_album_tracks.iter() {
                                    ui.label(format!("- {}", track.title));
                                }
                            }
                        }
                    }
                });
        }
    }

    if let Some(next) = action {
        app.select_album(next);
    }
}
