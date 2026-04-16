use crate::application::view_model::AppVm;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, vm: &AppVm) {
    ui.heading("Queue");
    ui.label("Playback queue will be shown here.");
    ui.separator();
    if vm.queue.tracks.is_empty() {
        ui.label("Queue is empty.");
    } else {
        for track in &vm.queue.tracks {
            ui.label(format!("- {}", track));
        }
    }
}
