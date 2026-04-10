use super::LrcPlayerApp;
use eframe::egui;

impl eframe::App for LrcPlayerApp {
    fn update(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.begin_frame();
    }
}

impl LrcPlayerApp {
    #[allow(dead_code)]
    fn render_tab_bar(&mut self, _ui: &mut egui::Ui) {}

    #[allow(dead_code)]
    fn render_tab_button(&mut self, _ui: &mut egui::Ui, _label: &str) {}

    #[allow(dead_code)]
    fn render_active_tab(&mut self, _ui: &mut egui::Ui, _ctx: &egui::Context) {}
}
