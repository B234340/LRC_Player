#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod audio;
mod core;
mod logging;
mod decode;
mod ui;

use app::Application;
use ui::egui_app::LrcPlayerEguiApp;

fn main() {
    #[cfg(target_os = "linux")]
    unsafe {
        std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
        std::env::set_var("MESA_LOADER_DRIVER_OVERRIDE", "llvmpipe");
    }

    let application = Application::default();
    let options = eframe::NativeOptions::default();

    if let Err(error) = eframe::run_native(
        "LRC Player",
        options,
        Box::new(|cc| Ok(Box::new(LrcPlayerEguiApp::new(cc, application)))),
    ) {
        eprintln!("failed to start UI: {error}");
    }
}
