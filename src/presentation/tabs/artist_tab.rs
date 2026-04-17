use crate::presentation::egui_app::GuiApp;
use eframe::egui;

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

    ui.heading("Artists");
    ui.label(format!("{} artists in library", artist_names.len()));
    ui.separator();

    if artist_names.is_empty() {
        ui.label("No artists available.");
    } else {
        for artist_name in artist_names.iter() {
            eframe::egui::CollapsingHeader::new(artist_name)
                .id_source(artist_name)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Select").clicked() {
                            action = Some(Some(artist_name.clone()));
                        }
                        if ui.button("Clear").clicked() {
                            action = Some(None);
                        }
                    });

                    if let Some(selected) = selected_artist.as_deref() {
                        if selected == artist_name {
                            if selected_artist_tracks.is_empty() {
                                ui.label("No tracks found for this artist.");
                            } else {
                                for track in selected_artist_tracks.iter() {
                                    ui.label(format!("- {}", track.title));
                                }
                            }
                        }
                    }
                });
        }
    }

    if let Some(next) = action {
        app.select_artist(next);
    }
}
