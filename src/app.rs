use crate::core::config::{AppConfig, ConfigStore};
use eframe::egui;

pub struct LrcPlayerApp {
    config_store: ConfigStore,
    config: AppConfig,
    status_line: String,
}

impl LrcPlayerApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let config_store = ConfigStore::new();
        let (config, status_line) = match config_store.load_or_create() {
            Ok(config) => (config, String::from("Portable mode active: using local ./data directory.")),
            Err(err) => (
                AppConfig::default(),
                format!("Config load failed, using defaults: {err}"),
            ),
        };

        Self {
            config_store,
            config,
            status_line,
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

                if self.config.library_folders.is_empty() {
                    ui.label("No library folders configured.");
                } else {
                    for folder in &self.config.library_folders {
                        ui.monospace(folder);
                    }
                }

                ui.add_space(12.0);
                ui.heading("Playlists");
                ui.label("Playlist persistence will live in ./data/playlists.json.");
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Now Playing");
            ui.label("Playback engine is not wired yet.");

            ui.add_space(16.0);
            ui.group(|ui| {
                ui.label("Current design constraints");
                ui.label("- All mutable files stay inside the application directory.");
                ui.label("- Core logic remains independent from UI toolkit details.");
                ui.label("- Audio backend will be injected behind a player interface.");
            });

            ui.add_space(16.0);

            if ui.button("Save current config").clicked() {
                match self.config_store.save(&self.config) {
                    Ok(()) => {
                        self.status_line = String::from("Config saved to ./data/config.json.");
                    }
                    Err(err) => {
                        self.status_line = format!("Config save failed: {err}");
                    }
                }
            }
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(&self.status_line);
                ui.separator();
                ui.label(format!("Theme: {}", self.config.theme));
            });
        });
    }
}
