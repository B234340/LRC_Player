#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;

use app::LrcPlayerApp;
use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([720.0, 480.0])
            .with_title("LRC Player"),
        ..Default::default()
    };

    eframe::run_native(
        "LRC Player",
        options,
        Box::new(|cc| Ok(Box::new(LrcPlayerApp::new(cc)))),
    )
}
