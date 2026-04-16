use crate::application::view_model::AppVm;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, vm: &AppVm) {
    ui.heading("Library");
    ui.label("Library overview and browsing will appear here.");
    ui.separator();
    ui.label(format!("Albums: {}", vm.library.album_count));
    ui.label(format!("Tracks: {}", vm.library.track_count));
}
