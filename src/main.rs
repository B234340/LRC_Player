mod app;
mod audio;
mod core;
mod decode;
mod ui;

use app::Application;
use ui::egui_app::LrcPlayerEguiApp;

fn main() {
    let application = Application::default();
    let options = eframe::NativeOptions::default();

    if let Err(error) = eframe::run_native(
        "LRC Player",
        options,
        Box::new(|_cc| Ok(Box::new(LrcPlayerEguiApp::new(application)))),
    ) {
        eprintln!("failed to start UI: {error}");
    }
}
