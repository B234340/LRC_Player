use crate::application::view_model::AppVm;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, vm: &AppVm) {
    ui.heading("Play");
    ui.label("Current playback controls are not yet connected.");
    ui.separator();
    ui.label(format!("Title: {}", vm.title));
}
