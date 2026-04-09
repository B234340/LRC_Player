use crate::core::config::ConfigStore;
use crate::core::state::{AppCommand, AppState};
use eframe::egui;

pub struct LrcPlayerApp {
    config_store: ConfigStore,
    state: AppState,
    audio_path_input: String,
}

impl LrcPlayerApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let config_store = ConfigStore::new();
        let state = AppState::bootstrap(&config_store);

        Self {
            config_store,
            state,
            audio_path_input: String::new(),
        }
    }
}

impl eframe::App for LrcPlayerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("LRC Player");
                ui.separator();
                ui.label("Portable audio player scaffold");
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(&self.state.status_line);
                ui.separator();
                ui.label(format!("Theme: {}", self.state.config.theme));
            });
        });

        let window_width = ctx.available_rect().width();
        let compact_layout = window_width < 860.0;

        if compact_layout {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.render_main_content(ui, true);
                });
            });
        } else {
            egui::SidePanel::left("sidebar")
                .resizable(true)
                .min_width(180.0)
                .default_width(220.0)
                .max_width(320.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.render_library_panel(ui);
                    });
                });

            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.render_main_content(ui, false);
                });
            });
        }
    }
}

impl LrcPlayerApp {
    fn render_library_panel(&self, ui: &mut egui::Ui) {
        ui.heading("Library");
        ui.label("Loaded tracks");

        if self.state.library.tracks().is_empty() {
            ui.label("No audio file loaded.");
        } else {
            for track in self.state.library.tracks() {
                ui.label(&track.title);
                ui.monospace(track.file_path());
                ui.add_space(8.0);
            }
        }

        ui.add_space(12.0);
        ui.heading("Portable data");
        ui.label("Config, cache, playlists, and logs stay under ./data.");
    }

    fn render_main_content(&mut self, ui: &mut egui::Ui, include_library: bool) {
        let playback_snapshot = self.state.playback_snapshot();

        ui.heading("Now Playing");
        ui.label(self.state.playback_summary());

        if let Some(track) = self.state.selected_track() {
            ui.add_space(8.0);
            ui.label(format!("Loaded track: {}", track.title));
            ui.monospace(track.file_path());
        }

        ui.add_space(16.0);
        ui.label("Audio file path");
        ui.add(
            egui::TextEdit::singleline(&mut self.audio_path_input)
                .desired_width(f32::INFINITY)
                .hint_text("/path/to/song.mp3 or C:/Music/song.flac"),
        );

        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Load file").clicked() {
                self.state.handle(
                    &self.config_store,
                    AppCommand::OpenAudioFile(self.audio_path_input.clone()),
                );
            }

            if ui.button("Play").clicked() {
                self.state
                    .handle(&self.config_store, AppCommand::PlaySelected);
            }

            match playback_snapshot.status {
                crate::core::player::PlaybackStatus::Playing => {
                    if ui.button("Pause").clicked() {
                        self.state.handle(&self.config_store, AppCommand::Pause);
                    }
                }
                crate::core::player::PlaybackStatus::Paused => {
                    if ui.button("Resume").clicked() {
                        self.state.handle(&self.config_store, AppCommand::Resume);
                    }
                }
                crate::core::player::PlaybackStatus::Stopped => {}
            }

            if ui.button("Stop").clicked() {
                self.state.handle(&self.config_store, AppCommand::Stop);
            }
        });

        ui.add_space(16.0);
        ui.group(|ui| {
            ui.label("Current design constraints");
            ui.label("- All mutable files stay inside the application directory.");
            ui.label("- Core logic remains independent from UI toolkit details.");
            ui.label("- Audio playback is decoded with symphonia and output via cpal.");
        });

        if include_library {
            ui.add_space(16.0);
            ui.group(|ui| {
                self.render_library_panel(ui);
            });
        }

        ui.add_space(16.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Save current config").clicked() {
                self.state
                    .handle(&self.config_store, AppCommand::SaveConfig);
            }

            ui.label(format!(
                "Indexed tracks: {}",
                self.state.library.track_count()
            ));
        });
    }
}
