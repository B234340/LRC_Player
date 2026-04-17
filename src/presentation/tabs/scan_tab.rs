use crate::application::view_model::AppVm;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, vm: &AppVm) {
    ui.heading("Scan");
    ui.label("Configure scan targets and exclude patterns, then start or cancel the scan.");
    ui.separator();

    ui.group(|ui| {
        ui.label("Scan targets:");
        if vm.scan.scan_targets.is_empty() {
            ui.label("No scan targets configured.");
        } else {
            for target in vm.scan.scan_targets.iter() {
                ui.label(format!("• {}", target));
            }
        }
    });

    ui.group(|ui| {
        ui.label("Exclude paths:");
        if vm.scan.scan_excludes.is_empty() {
            ui.label("No exclude paths configured.");
        } else {
            for excludes in vm.scan.scan_excludes.iter() {
                ui.label(format!("• {}", excludes));
            }
        }
    });

    ui.separator();

    if let Some(status) = vm.scan.last_scan_status.as_deref() {
        ui.label(format!("Last scan status: {}", status));
    } else {
        ui.label("No scan activity recorded yet.");
    }

    if vm.scan.scan_scanned.is_some()
        || vm.scan.scan_parsed.is_some()
        || vm.scan.scan_registered.is_some()
    {
        ui.separator();
        ui.heading("Scan progress");
        if let Some(scanned) = vm.scan.scan_scanned {
            ui.label(format!("Scanned: {}", scanned));
        }
        if let Some(parsed) = vm.scan.scan_parsed {
            ui.label(format!("Parsed: {}", parsed));
        }
        if let Some(registered) = vm.scan.scan_registered {
            ui.label(format!("Registered: {}", registered));
        }
    }
}
