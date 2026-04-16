use crate::application::view_model::AppVm;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, vm: &AppVm) {
    ui.heading("Scan");
    ui.label("Scan status and controls will appear here.");
    ui.separator();
    if let Some(status) = vm.scan.last_scan_status.as_deref() {
        ui.label(format!("Last scan status: {}", status));
    } else {
        ui.label("No scan activity recorded yet.");
    }
}
