use crate::core::config::{AppConfig, ConfigStore};
use crate::core::library::LibraryIndex;
use crate::core::models::{Track, TrackId, TrackLocation};
use crate::core::player::{
    NullPlayer, PlaybackCommand, PlaybackSnapshot, PlaybackStatus, PlayerBackend,
    SymphoniaCpalPlayer,
};
use std::path::PathBuf;

pub enum AppCommand {
    SaveConfig,
    AddLibraryFolder(String),
    OpenAudioFile(String),
    PlaySelected,
    Pause,
    Resume,
    Stop,
}

pub struct AppState {
    pub config: AppConfig,
    pub library: LibraryIndex,
    player: Box<dyn PlayerBackend>,
    pub selected_track_id: Option<TrackId>,
    pub status_line: String,
}

impl AppState {
    pub fn bootstrap(config_store: &ConfigStore) -> Self {
        let (player, player_status) = match SymphoniaCpalPlayer::new() {
            Ok(player) => (
                Box::new(player) as Box<dyn PlayerBackend>,
                String::from("Audio backend active: symphonia + cpal."),
            ),
            Err(err) => (
                Box::new(NullPlayer::default()) as Box<dyn PlayerBackend>,
                format!("Audio backend unavailable, using dummy player: {err}"),
            ),
        };

        match config_store.load_or_create() {
            Ok(config) => {
                let library = LibraryIndex::new(config.library_folders.clone());
                Self {
                    config,
                    library,
                    player,
                    selected_track_id: None,
                    status_line: format!(
                        "Portable mode active: using local ./data directory. {player_status}"
                    ),
                }
            }
            Err(err) => {
                let config = AppConfig::default();
                let library = LibraryIndex::new(config.library_folders.clone());
                Self {
                    config,
                    library,
                    player,
                    selected_track_id: None,
                    status_line: format!(
                        "Config load failed, using defaults: {err}. {player_status}"
                    ),
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
            AppCommand::OpenAudioFile(path) => {
                self.open_audio_file(path);
            }
            AppCommand::PlaySelected => {
                self.play_selected_track();
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
            PlaybackStatus::Stopped => match self.track_title_for_snapshot(&snapshot) {
                Some(title) => format!("Loaded {title}. Ready to play."),
                None => String::from("No track loaded."),
            },
            PlaybackStatus::Playing => {
                let track = self
                    .track_title_for_snapshot(&snapshot)
                    .unwrap_or_else(|| String::from("unknown track"));
                format!("Playing {track} at {} ms.", snapshot.position_ms)
            }
            PlaybackStatus::Paused => {
                let track = self
                    .track_title_for_snapshot(&snapshot)
                    .unwrap_or_else(|| String::from("unknown track"));
                format!("Paused {track} at {} ms.", snapshot.position_ms)
            }
        }
    }

    pub fn playback_snapshot(&self) -> PlaybackSnapshot {
        self.player.snapshot()
    }

    pub fn selected_track(&self) -> Option<&Track> {
        let track_id = self.selected_track_id.as_ref()?;
        self.library.track_by_id(track_id)
    }

    fn apply_playback(&mut self, command: PlaybackCommand) {
        if let Err(err) = self.player.dispatch(command) {
            self.status_line = format!("Playback command failed: {err}");
        }
    }

    fn open_audio_file(&mut self, path: String) {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            self.status_line = String::from("Enter an audio file path before loading.");
            return;
        }

        let raw_path = PathBuf::from(trimmed);
        let canonical_path = raw_path.canonicalize().unwrap_or(raw_path);
        if !canonical_path.is_file() {
            self.status_line = String::from("The selected path is not a file.");
            return;
        }

        let track_title = canonical_path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Unknown track")
            .to_string();
        let path_string = canonical_path.to_string_lossy().to_string();
        let track = Track {
            id: TrackId::new(path_string.clone()),
            title: track_title.clone(),
            artist: None,
            album: None,
            duration_ms: None,
            location: TrackLocation::Absolute(path_string),
            lyric_path: None,
        };

        self.library.replace_tracks(vec![track.clone()]);
        self.selected_track_id = Some(track.id.clone());
        self.apply_playback(PlaybackCommand::Stop);
        self.status_line = format!("Loaded audio file: {track_title}");
    }

    fn play_selected_track(&mut self) {
        let Some(track) = self.selected_track().cloned() else {
            self.status_line = String::from("Load an audio file before pressing play.");
            return;
        };
        let track_id = track.id.clone();
        let file_path = PathBuf::from(track.file_path());

        self.apply_playback(PlaybackCommand::PlayTrack {
            track_id,
            file_path,
        });
    }

    fn track_title_for_snapshot(&self, snapshot: &PlaybackSnapshot) -> Option<String> {
        let track_id = snapshot.current_track.as_ref()?;
        self.library
            .track_by_id(track_id)
            .map(|track| track.title.clone())
    }
}