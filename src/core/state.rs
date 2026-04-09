use crate::core::config::{AppConfig, ConfigStore};
use crate::core::library::LibraryIndex;
use crate::core::models::{Track, TrackId, TrackLocation};
use crate::core::player::{
    NullPlayer, PlaybackCommand, PlaybackSnapshot, PlaybackStatus, PlayerBackend,
    SymphoniaCpalPlayer,
};
use lofty::prelude::{Accessor, AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "wav", "ogg", "m4a", "aac"];

pub enum AppCommand {
    SaveConfig,
    AddLibraryFolder(String),
    OpenAudioDirectory(String),
    SelectTrack(TrackId),
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
                let mut state = Self {
                    config,
                    library: LibraryIndex::new(Vec::new()),
                    player,
                    selected_track_id: None,
                    status_line: format!(
                        "Portable mode active: using local ./data directory. {player_status}"
                    ),
                };
                state.restore_library_from_config();
                state
            }
            Err(err) => {
                let config = AppConfig::default();
                Self {
                    config,
                    library: LibraryIndex::new(Vec::new()),
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
            AppCommand::OpenAudioDirectory(path) => {
                self.open_audio_directory(config_store, path);
            }
            AppCommand::SelectTrack(track_id) => {
                self.selected_track_id = Some(track_id.clone());
                if let Some(track) = self.library.track_by_id(&track_id) {
                    self.status_line = format!("Selected track: {}", track.title);
                }
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

    fn open_audio_directory(&mut self, config_store: &ConfigStore, path: String) {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            self.status_line = String::from("Enter a directory path before scanning.");
            return;
        }

        let raw_path = PathBuf::from(trimmed);
        let canonical_path = raw_path.canonicalize().unwrap_or(raw_path);
        if !canonical_path.is_dir() {
            self.status_line = String::from("The selected path is not a directory.");
            return;
        }

        self.config.library_folders = vec![canonical_path.to_string_lossy().to_string()];
        self.restore_library_from_config();
        self.persist_config_after_scan(config_store);
    }

    fn play_selected_track(&mut self) {
        let Some(track) = self.selected_track().cloned() else {
            self.status_line = String::from("Scan a directory and select a track before pressing play.");
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

    fn restore_library_from_config(&mut self) {
        self.library = LibraryIndex::new(self.config.library_folders.clone());

        if self.config.library_folders.is_empty() {
            self.library.replace_tracks(Vec::new());
            self.selected_track_id = None;
            return;
        }

        let scan_result = scan_configured_directories(&self.config.library_folders);
        self.apply_playback(PlaybackCommand::Stop);

        match scan_result {
            Ok(scan_summary) => {
                self.library.replace_tracks(scan_summary.tracks);
                self.selected_track_id = self.library.tracks().first().map(|track| track.id.clone());

                if self.library.track_count() == 0 {
                    self.status_line = String::from(
                        "Configured directories were scanned, but no supported audio files were found.",
                    );
                } else if scan_summary.invalid_directories.is_empty() {
                    self.status_line = format!(
                        "Auto-scanned {} saved director{} and loaded {} tracks.",
                        self.config.library_folders.len(),
                        if self.config.library_folders.len() == 1 { "y" } else { "ies" },
                        self.library.track_count()
                    );
                } else {
                    self.status_line = format!(
                        "Loaded {} tracks from saved directories. Skipped {} invalid director{}.",
                        self.library.track_count(),
                        scan_summary.invalid_directories.len(),
                        if scan_summary.invalid_directories.len() == 1 { "y" } else { "ies" }
                    );
                }
            }
            Err(err) => {
                self.library.replace_tracks(Vec::new());
                self.selected_track_id = None;
                self.status_line = format!("Saved directory scan failed: {err}");
            }
        }
    }

    fn persist_config_after_scan(&mut self, config_store: &ConfigStore) {
        if let Err(err) = config_store.save(&self.config) {
            self.status_line = format!("Config save failed after scan: {err}");
        }
    }
}

struct DirectoryScanSummary {
    tracks: Vec<Track>,
    invalid_directories: Vec<String>,
}

fn scan_configured_directories(directories: &[String]) -> Result<DirectoryScanSummary, String> {
    let mut tracks = Vec::new();
    let mut invalid_directories = Vec::new();

    for directory in directories {
        let path = PathBuf::from(directory);
        if !path.is_dir() {
            invalid_directories.push(directory.clone());
            continue;
        }

        let mut scanned_tracks = scan_audio_directory(&path)?;
        tracks.append(&mut scanned_tracks);
    }

    tracks.sort_by(|left, right| left.title.to_lowercase().cmp(&right.title.to_lowercase()));
    tracks.dedup_by(|left, right| left.file_path() == right.file_path());

    Ok(DirectoryScanSummary {
        tracks,
        invalid_directories,
    })
}

fn scan_audio_directory(directory: &Path) -> Result<Vec<Track>, String> {
    let mut tracks = Vec::new();
    collect_audio_files(directory, &mut tracks)?;
    tracks.sort_by(|left, right| left.title.to_lowercase().cmp(&right.title.to_lowercase()));
    Ok(tracks)
}

fn collect_audio_files(directory: &Path, tracks: &mut Vec<Track>) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|err| format!("{}: {err}", directory.to_string_lossy()))?;

    for entry_result in entries {
        let entry = entry_result.map_err(|err| format!("{}: {err}", directory.to_string_lossy()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_audio_files(&path, tracks)?;
            continue;
        }

        if !is_supported_audio_file(&path) {
            continue;
        }

        let path_string = path.to_string_lossy().to_string();
        let metadata = read_track_metadata(&path);
        let lyric_path = sidecar_lrc_path(&path).map(|sidecar| sidecar.to_string_lossy().to_string());

        tracks.push(Track {
            id: TrackId::new(path_string.clone()),
            title: metadata.title,
            artist: metadata.artist,
            album: None,
            duration_ms: metadata.duration_ms,
            location: TrackLocation::Absolute(path_string),
            lyric_path,
        });
    }

    Ok(())
}

fn is_supported_audio_file(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };

    AUDIO_EXTENSIONS
        .iter()
        .any(|supported| extension.eq_ignore_ascii_case(supported))
}

fn sidecar_lrc_path(path: &Path) -> Option<PathBuf> {
    let candidate = path.with_extension("lrc");
    candidate.is_file().then_some(candidate)
}

struct TrackMetadata {
    title: String,
    artist: Option<String>,
    duration_ms: Option<u64>,
}

fn read_track_metadata(path: &Path) -> TrackMetadata {
    let fallback_title = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Unknown track")
        .to_string();

    let probe = match Probe::open(path) {
        Ok(probe) => probe,
        Err(_) => {
            return TrackMetadata {
                title: fallback_title,
                artist: None,
                duration_ms: None,
            }
        }
    };

    let probe = match probe.guess_file_type() {
        Ok(probe) => probe,
        Err(_) => {
            return TrackMetadata {
                title: fallback_title,
                artist: None,
                duration_ms: None,
            }
        }
    };

    let tagged_file = match probe.read() {
        Ok(tagged_file) => tagged_file,
        Err(_) => {
            return TrackMetadata {
                title: fallback_title,
                artist: None,
                duration_ms: None,
            }
        }
    };

    let tag = tagged_file.primary_tag().or_else(|| tagged_file.first_tag());
    let title = tag
        .and_then(|tag| tag.title())
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_title);
    let artist = tag
        .and_then(|tag| tag.artist())
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty());
    let duration_ms = Some(tagged_file.properties().duration().as_millis().min(u128::from(u64::MAX)) as u64);

    TrackMetadata {
        title,
        artist,
        duration_ms,
    }
}