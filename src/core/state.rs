use crate::core::config::{AppConfig, ConfigStore};
use crate::core::library::LibraryIndex;
use crate::core::models::TrackId;
use crate::core::player::{NullPlayer, PlaybackCommand, PlaybackStatus, PlayerBackend};

pub enum AppCommand {
    SaveConfig,
    AddLibraryFolder(String),
    PlayTrack(TrackId),
    Pause,
    Resume,
    Stop,
}

pub struct AppState {
    pub config: AppConfig,
    pub library: LibraryIndex,
    player: NullPlayer,
    pub status_line: String,
}

impl AppState {
    pub fn bootstrap(config_store: &ConfigStore) -> Self {
        match config_store.load_or_create() {
            Ok(config) => {
                let library = LibraryIndex::new(config.library_folders.clone());
                Self {
                    config,
                    library,
                    player: NullPlayer::default(),
                    status_line: String::from(
                        "Portable mode active: using local ./data directory.",
                    ),
                }
            }
            Err(err) => {
                let config = AppConfig::default();
                let library = LibraryIndex::new(config.library_folders.clone());
                Self {
                    config,
                    library,
                    player: NullPlayer::default(),
                    status_line: format!("Config load failed, using defaults: {err}"),
                }
            }
        }
    }

    pub fn handle(&mut self, config_store: &ConfigStore, command: AppCommand) {
        match command {
            AppCommand::SaveConfig => match config_store.save(&self.config) {
                Ok(()) => {
                    self.status_line = String::from("Config saved to ./data/config.json.");
                }
                Err(err) => {
                    self.status_line = format!("Config save failed: {err}");
                }
            },
            AppCommand::AddLibraryFolder(folder) => {
                self.library.add_folder(folder.clone());
                self.config.library_folders = self.library.folders().to_vec();
                self.status_line = format!("Library folder added: {folder}");
            }
            AppCommand::PlayTrack(track_id) => {
                self.apply_playback(PlaybackCommand::PlayTrack(track_id));
            }
            AppCommand::Pause => {
                self.apply_playback(PlaybackCommand::Pause);
            }
            AppCommand::Resume => {
                self.apply_playback(PlaybackCommand::Resume);
            }
            AppCommand::Stop => {
                self.apply_playback(PlaybackCommand::Stop);
            }
        }
    }

    pub fn playback_summary(&self) -> String {
        let snapshot = self.player.snapshot();
        match snapshot.status {
            PlaybackStatus::Stopped => String::from("Playback engine is scaffolded but currently idle."),
            PlaybackStatus::Playing => {
                let track = snapshot
                    .current_track
                    .map(|track_id| track_id.0)
                    .unwrap_or_else(|| String::from("unknown track"));
                format!("Playing {track} at {} ms.", snapshot.position_ms)
            }
            PlaybackStatus::Paused => String::from("Playback is paused in the scaffold backend."),
        }
    }

    fn apply_playback(&mut self, command: PlaybackCommand) {
        if let Err(err) = self.player.dispatch(command) {
            self.status_line = format!("Playback command failed: {err}");
        }
    }
}