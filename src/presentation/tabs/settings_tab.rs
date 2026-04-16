use crate::application::view_model::AppVm;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, vm: &AppVm) {
    ui.heading("Settings");
    ui.label("Application settings and preferences.");
    ui.separator();
    ui.label(format!("Preferred output device: {}", vm.settings.preferred_output_device.as_deref().unwrap_or("<none>")));
    ui.label(format!("Resampler taps: {}", vm.settings.resampler_taps));
}
