use crate::presentation::egui_app::GuiApp;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    let vm = &app.app_vm;
    ui.heading("Scan");
    ui.label("Configure scan targets and exclude patterns, then start or cancel the scan.");
    ui.label("Scan can discover audio files and related .lrc files; LRC directories are also supported.");
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
        ui.label("LRC scan targets:");
        if vm.scan.scan_lrc_targets.is_empty() {
            ui.label("No LRC scan targets configured.");
        } else {
            for target in vm.scan.scan_lrc_targets.iter() {
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

    if app.app_vm.scan.scan_scanned.is_some()
        || app.app_vm.scan.scan_parsed.is_some()
        || app.app_vm.scan.scan_registered.is_some()
    {
        ui.separator();
        ui.heading("Scan progress");
        if let Some(scanned) = app.app_vm.scan.scan_scanned {
            ui.label(format!("Scanned: {}", scanned));
        }
        if let Some(parsed) = app.app_vm.scan.scan_parsed {
            ui.label(format!("Parsed: {}", parsed));
        }
        if let Some(registered) = app.app_vm.scan.scan_registered {
            ui.label(format!("Registered: {}", registered));
        }
    }

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Clear library").clicked() {
            app.clear_library();
        }
    });
}
