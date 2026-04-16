use crate::application::view_model::AppVm;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, vm: &AppVm) {
    ui.heading("Playlists");
    ui.label("Saved playlists will appear here.");
    ui.separator();
    if vm.playlists.is_empty() {
        ui.label("No playlists available.");
    } else {
        for playlist in &vm.playlists {
            ui.label(format!("• {}", playlist.name));
        }
    }
}
