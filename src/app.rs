use crate::core::config::ConfigStore;
use crate::core::state::{AppCommand, AppState};
use eframe::egui;

pub struct LrcPlayerApp {
    config_store: ConfigStore,
    state: AppState,
}

impl LrcPlayerApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let config_store = ConfigStore::new();
        let state = AppState::bootstrap(&config_store);

        Self {
            config_store,
            state,
        }
    }
}

impl eframe::App for LrcPlayerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("LRC Player");
                ui.separator();
                ui.label("Portable audio player scaffold");
            });
        });

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(220.0)
            .show(ctx, |ui| {
                ui.heading("Library");
                ui.label("Folders");

                if self.state.config.library_folders.is_empty() {
                    ui.label("No library folders configured.");
                } else {
                    for folder in &self.state.config.library_folders {
                        ui.monospace(folder);
                    }
                }

                ui.add_space(12.0);
                ui.heading("Playlists");
                ui.label("Playlist persistence will live in ./data/playlists.json.");
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Now Playing");
            ui.label(self.state.playback_summary());

            ui.add_space(16.0);
            ui.group(|ui| {
                ui.label("Current design constraints");
                ui.label("- All mutable files stay inside the application directory.");
                ui.label("- Core logic remains independent from UI toolkit details.");
                ui.label("- Audio backend will be injected behind a player interface.");
            });

            ui.add_space(16.0);

            if ui.button("Save current config").clicked() {
                self.state
                    .handle(&self.config_store, AppCommand::SaveConfig);
            }

            ui.add_space(12.0);
            ui.label(format!(
                "Indexed tracks: {}",
                self.state.library.track_count()
            ));
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(&self.state.status_line);
                ui.separator();
                ui.label(format!("Theme: {}", self.state.config.theme));
            });
        });
    }
}
