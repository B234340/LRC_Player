fn main() {
    let options = eframe::NativeOptions::default();
    let _ = eframe::run_native(
        "LRC Player GUI",
        options,
        Box::new(|cc| Box::new(lrc_player::presentation::egui_app::GuiApp::new(cc))),
    );
}
