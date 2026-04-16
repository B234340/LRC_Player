use crate::application::view_model::AppVm;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, _vm: &AppVm) {
    ui.heading("Albums");
    ui.label("Album browsing UI is under development.");
}
