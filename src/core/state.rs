use crate::core::config::{AppConfig, ConfigStore};
use crate::core::library::LibraryIndex;
use crate::core::models::{Playlist, QueueMode, QueueState, RepeatMode, Track, TrackId, TrackLocation};
use crate::core::player::{
    NullPlayer, PlaybackCommand, PlaybackSnapshot, PlaybackStatus, PlayerBackend,
    SymphoniaCpalPlayer,
};
use lofty::prelude::{Accessor, AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use rand::seq::SliceRandom;
use std::fs;
use std::path::{Path, PathBuf};

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "wav", "ogg", "m4a", "aac"];

pub enum AppCommand {
    AddLibraryFolder(String),
    RemoveLibraryFolder(String),
    AddExcludedFolder(String),
    RemoveExcludedFolder(String),
    RescanLibrary,
    BuildQueueAndPlay {
        track_ids: Vec<TrackId>,
        start_track_id: TrackId,
    },
    JumpToQueueTrack(TrackId),
    QueueTrackNext(TrackId),
    AppendTrackToQueue(TrackId),
    MoveQueueItem { from: usize, to: usize },
    SetRepeatMode(RepeatMode),
    SetQueueMode(QueueMode),
    PlaySelected,
    Pause,
    Resume,
    PreviousTrack,
    NextTrack,
    Seek(u64),
    CreatePlaylist(String),
    DeletePlaylist(String),
    SelectPlaylist(Option<String>),
    AddTrackToPlaylist { playlist_name: String, track_id: TrackId },
    RemoveTrackFromPlaylist { playlist_name: String, track_id: TrackId },
    MovePlaylistItem { playlist_name: String, from: usize, to: usize },
    SetTheme(String),
    SetQueueFollowCurrent(bool),
    SetShowLibraryTrackPaths(bool),
    SetShowLibraryLyricPaths(bool),
}

pub struct AppState {
    pub config: AppConfig,
    pub library: LibraryIndex,
    pub playlists: Vec<Playlist>,
    pub queue: QueueState,
    pub selected_track_id: Option<TrackId>,
    pub selected_playlist_name: Option<String>,
    pub status_line: String,
    player: Box<dyn PlayerBackend>,
    last_playback_status: PlaybackStatus,
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
                let playlist_load_result = config_store.load_playlists();
                let mut playlist_status = None;
                let playlists = match playlist_load_result {
                    Ok(playlists) => playlists,
                    Err(err) => {
                        playlist_status = Some(format!(" Playlist load failed: {err}"));
                        Vec::new()
                    }
                };
                let mut state = Self {
                    selected_playlist_name: config.last_playlist.clone(),
                    config,
                    library: LibraryIndex::new(),
                    playlists,
                    queue: QueueState::default(),
                    selected_track_id: None,
                    status_line: format!(
                        "Portable mode active: using local ./data directory. {player_status}{}",
                        playlist_status.unwrap_or_default()
                    ),
                    player,
                    last_playback_status: PlaybackStatus::Stopped,
                };
                state.restore_library_from_config();
                state
            }
            Err(err) => Self {
                config: AppConfig::default(),
                library: LibraryIndex::new(),
                playlists: Vec::new(),
                queue: QueueState::default(),
                selected_track_id: None,
                selected_playlist_name: None,
                status_line: format!("Config load failed, using defaults: {err}. {player_status}"),
                player,
                last_playback_status: PlaybackStatus::Stopped,
            },
        }
    }

    pub fn refresh(&mut self) {
        let snapshot = self.player.snapshot();
        let track_finished = self.last_playback_status == PlaybackStatus::Playing
            && snapshot.status == PlaybackStatus::Stopped
            && snapshot.position_ms > 0;

        if track_finished {
            self.advance_after_track_end();
            let updated_snapshot = self.player.snapshot();
            self.sync_queue_with_snapshot(&updated_snapshot);
            self.last_playback_status = updated_snapshot.status;
            return;
        }

        self.sync_queue_with_snapshot(&snapshot);
        self.last_playback_status = snapshot.status;
    }

    pub fn handle(&mut self, config_store: &ConfigStore, command: AppCommand) {
        match command {
            AppCommand::AddLibraryFolder(folder) => {
                self.add_library_folder(config_store, folder);
            }
            AppCommand::RemoveLibraryFolder(folder) => {
                self.remove_library_folder(config_store, &folder);
            }
            AppCommand::AddExcludedFolder(folder) => {
                self.add_excluded_folder(config_store, folder);
            }
            AppCommand::RemoveExcludedFolder(folder) => {
                self.remove_excluded_folder(config_store, &folder);
            }
            AppCommand::RescanLibrary => {
                self.restore_library_from_config();
                self.persist_config(config_store, "Library configuration saved.");
            }
            AppCommand::BuildQueueAndPlay {
                track_ids,
                start_track_id,
            } => {
                self.build_queue(track_ids, start_track_id, true);
            }
            AppCommand::JumpToQueueTrack(track_id) => {
                self.jump_to_queue_track(&track_id);
            }
            AppCommand::QueueTrackNext(track_id) => {
                self.queue_track_next(track_id);
            }
            AppCommand::AppendTrackToQueue(track_id) => {
                self.append_track_to_queue(track_id);
            }
            AppCommand::MoveQueueItem { from, to } => {
                self.move_queue_item(from, to);
            }
            AppCommand::SetRepeatMode(mode) => {
                self.queue.repeat_mode = mode;
            }
            AppCommand::SetQueueMode(mode) => {
                self.queue.queue_mode = mode;
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
            AppCommand::PreviousTrack => {
                self.previous_track();
            }
            AppCommand::NextTrack => {
                self.next_track();
            }
            AppCommand::Seek(position_ms) => {
                self.apply_playback(PlaybackCommand::Seek(position_ms));
            }
            AppCommand::CreatePlaylist(name) => {
                self.create_playlist(name);
                if !self.status_line.starts_with("Playlist save failed:") {
                    self.persist_playlists(config_store, &self.status_line.clone());
                }
            }
            AppCommand::DeletePlaylist(name) => {
                self.delete_playlist(&name);
                if !self.status_line.starts_with("Playlist save failed:") {
                    self.persist_playlists(config_store, &self.status_line.clone());
                }
            }
            AppCommand::SelectPlaylist(name) => {
                self.selected_playlist_name = name.clone();
                self.config.last_playlist = name;
                self.persist_config(config_store, "Playlist selection saved.");
            }
            AppCommand::AddTrackToPlaylist {
                playlist_name,
                track_id,
            } => {
                self.add_track_to_playlist(&playlist_name, track_id);
                if !self.status_line.starts_with("Playlist not found.") {
                    self.persist_playlists(config_store, &self.status_line.clone());
                }
            }
            AppCommand::RemoveTrackFromPlaylist {
                playlist_name,
                track_id,
            } => {
                self.remove_track_from_playlist(&playlist_name, &track_id);
                self.persist_playlists(config_store, &self.status_line.clone());
            }
            AppCommand::MovePlaylistItem {
                playlist_name,
                from,
                to,
            } => {
                self.move_playlist_item(config_store, &playlist_name, from, to);
            }
            AppCommand::SetTheme(theme) => {
                self.config.theme = theme;
                self.persist_config(config_store, "Theme saved.");
            }
            AppCommand::SetQueueFollowCurrent(enabled) => {
                self.config.queue_follow_current = enabled;
                self.persist_config(config_store, "Queue follow setting saved.");
            }
            AppCommand::SetShowLibraryTrackPaths(enabled) => {
                self.config.show_library_track_paths = enabled;
                self.persist_config(config_store, "Library path setting saved.");
            }
            AppCommand::SetShowLibraryLyricPaths(enabled) => {
                self.config.show_library_lyric_paths = enabled;
                self.persist_config(config_store, "Library lyric path setting saved.");
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

    fn add_library_folder(&mut self, config_store: &ConfigStore, folder: String) {
        let Some(canonical) = canonicalize_directory(&folder) else {
            self.status_line = String::from("The selected path is not a directory.");
            return;
        };

        if !self.config.library_folders.contains(&canonical) {
            self.config.library_folders.push(canonical.clone());
        }

        self.restore_library_from_config();
        self.persist_config(config_store, &format!("Added scan folder: {canonical}"));
    }

    fn remove_library_folder(&mut self, config_store: &ConfigStore, folder: &str) {
        self.config.library_folders.retain(|entry| entry != folder);
        self.restore_library_from_config();
        self.persist_config(config_store, "Scan folder removed.");
    }

    fn add_excluded_folder(&mut self, config_store: &ConfigStore, folder: String) {
        let Some(canonical) = canonicalize_directory(&folder) else {
            self.status_line = String::from("The selected exclusion path is not a directory.");
            return;
        };

        if !self.config.excluded_folders.contains(&canonical) {
            self.config.excluded_folders.push(canonical.clone());
        }

        self.restore_library_from_config();
        self.persist_config(config_store, &format!("Added excluded folder: {canonical}"));
    }

    fn remove_excluded_folder(&mut self, config_store: &ConfigStore, folder: &str) {
        self.config.excluded_folders.retain(|entry| entry != folder);
        self.restore_library_from_config();
        self.persist_config(config_store, "Excluded folder removed.");
    }

    fn persist_config(&mut self, config_store: &ConfigStore, success_message: &str) {
        match config_store.save(&self.config) {
            Ok(()) => {
                self.status_line = success_message.to_string();
            }
            Err(err) => {
                self.status_line = format!("Config save failed: {err}");
            }
        }
    }

    fn persist_playlists(&mut self, config_store: &ConfigStore, success_message: &str) {
        match config_store.save_playlists(&self.playlists) {
            Ok(()) => {
                self.status_line = success_message.to_string();
            }
            Err(err) => {
                self.status_line = format!("Playlist save failed: {err}");
            }
        }
    }

    fn play_selected_track(&mut self) {
        if let Some(track_id) = self.selected_track_id.clone() {
            if self.queue.track_ids.is_empty() || !self.queue.track_ids.contains(&track_id) {
                let library_track_ids = self
                    .library
                    .tracks()
                    .iter()
                    .map(|track| track.id.clone())
                    .collect::<Vec<_>>();
                self.build_queue(library_track_ids, track_id, true);
                return;
            }

            self.jump_to_queue_track(&track_id);
            return;
        }

        self.status_line = String::from("Select a track before pressing play.");
    }

    fn build_queue(&mut self, track_ids: Vec<TrackId>, start_track_id: TrackId, start_playback: bool) {
        if track_ids.is_empty() {
            self.status_line = String::from("There are no tracks to queue.");
            return;
        }

        let Some(start_index) = track_ids.iter().position(|track_id| *track_id == start_track_id) else {
            self.status_line = String::from("The selected track is not part of the requested queue.");
            return;
        };

        self.queue.track_ids = track_ids;
        self.queue.current_index = Some(start_index);
        self.selected_track_id = Some(start_track_id.clone());

        if start_playback {
            self.play_queue_index(start_index);
        }
    }

    fn jump_to_queue_track(&mut self, track_id: &TrackId) {
        let Some(index) = self.queue.track_ids.iter().position(|queued_track_id| queued_track_id == track_id) else {
            self.status_line = String::from("The selected track is not in the queue.");
            return;
        };

        self.play_queue_index(index);
    }

    fn play_queue_index(&mut self, index: usize) {
        let Some(track_id) = self.queue.track_ids.get(index).cloned() else {
            return;
        };
        let Some(track) = self.library.track_by_id(&track_id).cloned() else {
            self.status_line = String::from("The queued track could not be found in the library.");
            return;
        };

        self.queue.current_index = Some(index);
        self.selected_track_id = Some(track.id.clone());
        let file_path = PathBuf::from(track.file_path());
        self.apply_playback(PlaybackCommand::PlayTrack {
            track_id: track.id,
            file_path,
        });
    }

    fn queue_track_next(&mut self, track_id: TrackId) {
        if !self.library.tracks().iter().any(|track| track.id == track_id) {
            self.status_line = String::from("Cannot queue a track that is not in the library.");
            return;
        }

        if self.queue.track_ids.is_empty() {
            self.queue.track_ids.push(track_id.clone());
            self.queue.current_index = Some(0);
            self.selected_track_id = Some(track_id.clone());
            self.status_line = String::from("Track inserted into an empty queue.");
            return;
        }

        let insert_at = self.queue.current_index.map(|index| index + 1).unwrap_or(0);
        self.queue.track_ids.insert(insert_at, track_id.clone());
        if let Some(current_index) = self.queue.current_index {
            if insert_at <= current_index {
                self.queue.current_index = Some(current_index + 1);
            }
        }
        self.status_line = format!("Queued {} to play next.", self.describe_track(&track_id));
    }

    fn append_track_to_queue(&mut self, track_id: TrackId) {
        if !self.library.tracks().iter().any(|track| track.id == track_id) {
            self.status_line = String::from("Cannot queue a track that is not in the library.");
            return;
        }

        self.queue.track_ids.push(track_id.clone());
        if self.queue.current_index.is_none() {
            self.queue.current_index = Some(0);
        }
        self.status_line = format!("Appended {} to the queue.", self.describe_track(&track_id));
    }

    fn move_queue_item(&mut self, from: usize, to: usize) {
        if from >= self.queue.track_ids.len() || to >= self.queue.track_ids.len() || from == to {
            return;
        }

        let track_id = self.queue.track_ids.remove(from);
        self.queue.track_ids.insert(to, track_id);

        if let Some(current_index) = self.queue.current_index {
            self.queue.current_index = Some(adjust_current_index_after_move(current_index, from, to));
        }
    }

    fn next_track(&mut self) {
        if self.queue.track_ids.is_empty() {
            self.status_line = String::from("Queue is empty.");
            return;
        }

        let Some(next_index) = self.resolve_next_index(false) else {
            self.apply_playback(PlaybackCommand::Stop);
            return;
        };

        self.play_queue_index(next_index);
    }

    fn previous_track(&mut self) {
        if self.queue.track_ids.is_empty() {
            self.status_line = String::from("Queue is empty.");
            return;
        }

        let Some(previous_index) = self.resolve_next_index(true) else {
            self.play_queue_index(0);
            return;
        };

        self.play_queue_index(previous_index);
    }

    fn resolve_next_index(&self, backwards: bool) -> Option<usize> {
        if self.queue.track_ids.is_empty() {
            return None;
        }

        let current_index = self.queue.current_index.unwrap_or(0);

        if self.queue.repeat_mode == RepeatMode::One {
            return Some(current_index);
        }

        if self.queue.queue_mode == QueueMode::Shuffle && self.queue.track_ids.len() > 1 {
            let mut indices = (0..self.queue.track_ids.len()).collect::<Vec<_>>();
            indices.retain(|index| *index != current_index);
            let mut rng = rand::thread_rng();
            return indices.choose(&mut rng).copied();
        }

        if backwards {
            if current_index > 0 {
                return Some(current_index - 1);
            }

            return if self.queue.repeat_mode == RepeatMode::All {
                Some(self.queue.track_ids.len().saturating_sub(1))
            } else {
                None
            };
        }

        let next_index = current_index + 1;
        if next_index < self.queue.track_ids.len() {
            return Some(next_index);
        }

        if self.queue.repeat_mode == RepeatMode::All {
            Some(0)
        } else {
            None
        }
    }

    fn advance_after_track_end(&mut self) {
        let Some(next_index) = self.resolve_next_index(false) else {
            self.queue.current_index = None;
            return;
        };

        self.play_queue_index(next_index);
    }

    fn create_playlist(&mut self, name: String) {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            self.status_line = String::from("Playlist name cannot be empty.");
            return;
        }

        if self.playlists.iter().any(|playlist| playlist.name == trimmed) {
            self.status_line = String::from("A playlist with that name already exists.");
            return;
        }

        self.playlists.push(Playlist {
            name: trimmed.to_string(),
            track_ids: Vec::new(),
        });
        self.selected_playlist_name = Some(trimmed.to_string());
        self.config.last_playlist = Some(trimmed.to_string());
        self.status_line = format!("Created playlist: {trimmed}");
    }

    fn delete_playlist(&mut self, name: &str) {
        self.playlists.retain(|playlist| playlist.name != name);
        if self.selected_playlist_name.as_deref() == Some(name) {
            self.selected_playlist_name = None;
        }
        if self.config.last_playlist.as_deref() == Some(name) {
            self.config.last_playlist = None;
        }
        self.status_line = format!("Deleted playlist: {name}");
    }

    fn add_track_to_playlist(&mut self, playlist_name: &str, track_id: TrackId) {
        let Some(playlist) = self.playlists.iter_mut().find(|playlist| playlist.name == playlist_name) else {
            self.status_line = String::from("Playlist not found.");
            return;
        };

        playlist.track_ids.push(track_id.clone());
        self.status_line = format!("Added {} to playlist {playlist_name}.", self.describe_track(&track_id));
    }

    fn remove_track_from_playlist(&mut self, playlist_name: &str, track_id: &TrackId) {
        let Some(playlist) = self.playlists.iter_mut().find(|playlist| playlist.name == playlist_name) else {
            return;
        };

        playlist.track_ids.retain(|candidate| candidate != track_id);
        self.status_line = format!("Removed {} from playlist {playlist_name}.", self.describe_track(track_id));
    }

    fn move_playlist_item(&mut self, config_store: &ConfigStore, playlist_name: &str, from: usize, to: usize) {
        let Some(playlist) = self.playlists.iter_mut().find(|playlist| playlist.name == playlist_name) else {
            self.status_line = String::from("Playlist not found.");
            return;
        };

        if from >= playlist.track_ids.len() || to >= playlist.track_ids.len() || from == to {
            return;
        }

        let track_id = playlist.track_ids.remove(from);
        playlist.track_ids.insert(to, track_id);
        self.persist_playlists(config_store, &format!("Reordered playlist: {playlist_name}"));
    }

    fn describe_track(&self, track_id: &TrackId) -> String {
        self.library
            .track_by_id(track_id)
            .map(|track| track.title.clone())
            .unwrap_or_else(|| String::from("track"))
    }

    fn track_title_for_snapshot(&self, snapshot: &PlaybackSnapshot) -> Option<String> {
        let track_id = snapshot.current_track.as_ref()?;
        self.library.track_by_id(track_id).map(|track| track.title.clone())
    }

    fn sync_queue_with_snapshot(&mut self, snapshot: &PlaybackSnapshot) {
        if let Some(track_id) = snapshot.current_track.as_ref() {
            if let Some(index) = self.queue.track_ids.iter().position(|queued_track_id| queued_track_id == track_id) {
                self.queue.current_index = Some(index);
                self.selected_track_id = Some(track_id.clone());
            }
        }
    }

    fn restore_library_from_config(&mut self) {
        self.library = LibraryIndex::new();

        if self.config.library_folders.is_empty() {
            self.library.replace_tracks(Vec::new());
            self.selected_track_id = None;
            self.queue.track_ids.clear();
            self.queue.current_index = None;
            self.retain_playlist_tracks();
            return;
        }

        let scan_result = scan_configured_directories(
            &self.config.library_folders,
            &self.config.excluded_folders,
        );
        self.apply_playback(PlaybackCommand::Stop);

        match scan_result {
            Ok(scan_summary) => {
                self.library.replace_tracks(scan_summary.tracks);
                self.selected_track_id = self.library.tracks().first().map(|track| track.id.clone());

                self.queue.track_ids.retain(|track_id| self.library.track_by_id(track_id).is_some());
                self.retain_playlist_tracks();
                if let Some(current_index) = self.queue.current_index {
                    if current_index >= self.queue.track_ids.len() {
                        self.queue.current_index = self.queue.track_ids.len().checked_sub(1);
                    }
                }

                if self.library.track_count() == 0 {
                    self.status_line = String::from(
                        "Configured directories were scanned, but no supported audio files were found.",
                    );
                } else if scan_summary.invalid_directories.is_empty() {
                    self.status_line = format!(
                        "Scanned {} folder{} and loaded {} tracks.",
                        self.config.library_folders.len(),
                        if self.config.library_folders.len() == 1 { "" } else { "s" },
                        self.library.track_count()
                    );
                } else {
                    self.status_line = format!(
                        "Loaded {} tracks. Skipped {} invalid director{}.",
                        self.library.track_count(),
                        scan_summary.invalid_directories.len(),
                        if scan_summary.invalid_directories.len() == 1 { "y" } else { "ies" }
                    );
                }
            }
            Err(err) => {
                self.library.replace_tracks(Vec::new());
                self.selected_track_id = None;
                self.queue.track_ids.clear();
                self.queue.current_index = None;
                self.retain_playlist_tracks();
                self.status_line = format!("Saved directory scan failed: {err}");
            }
        }
    }

    fn retain_playlist_tracks(&mut self) {
        for playlist in &mut self.playlists {
            playlist
                .track_ids
                .retain(|track_id| self.library.track_by_id(track_id).is_some());
        }

        self.playlists.retain(|playlist| !playlist.name.trim().is_empty());

        if let Some(selected_name) = self.selected_playlist_name.as_ref() {
            if !self.playlists.iter().any(|playlist| &playlist.name == selected_name) {
                self.selected_playlist_name = None;
            }
        }
    }
}

struct DirectoryScanSummary {
    tracks: Vec<Track>,
    invalid_directories: Vec<String>,
}

fn scan_configured_directories(
    directories: &[String],
    excluded_directories: &[String],
) -> Result<DirectoryScanSummary, String> {
    let mut tracks = Vec::new();
    let mut invalid_directories = Vec::new();
    let excluded_paths = excluded_directories
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();

    for directory in directories {
        let path = PathBuf::from(directory);
        if !path.is_dir() {
            invalid_directories.push(directory.clone());
            continue;
        }

        if is_excluded_path(&path, &excluded_paths) {
            continue;
        }

        let mut scanned_tracks = scan_audio_directory(&path, &excluded_paths)?;
        tracks.append(&mut scanned_tracks);
    }

    tracks.sort_by(|left, right| left.file_path().cmp(right.file_path()));
    tracks.dedup_by(|left, right| left.file_path() == right.file_path());

    Ok(DirectoryScanSummary {
        tracks,
        invalid_directories,
    })
}

fn scan_audio_directory(directory: &Path, excluded_paths: &[PathBuf]) -> Result<Vec<Track>, String> {
    let mut tracks = Vec::new();
    collect_audio_files(directory, excluded_paths, &mut tracks)?;
    tracks.sort_by(|left, right| left.file_path().cmp(right.file_path()));
    Ok(tracks)
}

fn collect_audio_files(
    directory: &Path,
    excluded_paths: &[PathBuf],
    tracks: &mut Vec<Track>,
) -> Result<(), String> {
    if is_excluded_path(directory, excluded_paths) {
        return Ok(());
    }

    let entries = fs::read_dir(directory)
        .map_err(|err| format!("{}: {err}", directory.to_string_lossy()))?;

    for entry_result in entries {
        let entry = entry_result.map_err(|err| format!("{}: {err}", directory.to_string_lossy()))?;
        let path = entry.path();
        if is_excluded_path(&path, excluded_paths) {
            continue;
        }

        if path.is_dir() {
            collect_audio_files(&path, excluded_paths, tracks)?;
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
            album: metadata.album,
            album_artist: metadata.album_artist,
            composer: metadata.composer,
            duration_ms: metadata.duration_ms,
            track_number: metadata.track_number,
            file_name: metadata.file_name,
            location: TrackLocation::Absolute(path_string),
            lyric_path,
            artwork: metadata.artwork,
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

fn is_excluded_path(path: &Path, excluded_paths: &[PathBuf]) -> bool {
    excluded_paths.iter().any(|excluded| path.starts_with(excluded))
}

fn sidecar_lrc_path(path: &Path) -> Option<PathBuf> {
    let candidate = path.with_extension("lrc");
    candidate.is_file().then_some(candidate)
}

struct TrackMetadata {
    title: String,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    composer: Option<String>,
    duration_ms: Option<u64>,
    track_number: Option<u32>,
    file_name: String,
    artwork: Option<Vec<u8>>,
}

fn read_track_metadata(path: &Path) -> TrackMetadata {
    let fallback_title = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Unknown track")
        .to_string();
    let fallback_album = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .map(|value| value.to_string());
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Unknown file")
        .to_string();

    let probe = match Probe::open(path) {
        Ok(probe) => probe,
        Err(_) => {
            return TrackMetadata {
                title: fallback_title,
                artist: None,
                album: fallback_album,
                album_artist: None,
                composer: None,
                duration_ms: None,
                track_number: None,
                file_name,
                artwork: None,
            }
        }
    };

    let probe = match probe.guess_file_type() {
        Ok(probe) => probe,
        Err(_) => {
            return TrackMetadata {
                title: fallback_title,
                artist: None,
                album: fallback_album,
                album_artist: None,
                composer: None,
                duration_ms: None,
                track_number: None,
                file_name,
                artwork: None,
            }
        }
    };

    let tagged_file = match probe.read() {
        Ok(tagged_file) => tagged_file,
        Err(_) => {
            return TrackMetadata {
                title: fallback_title,
                artist: None,
                album: fallback_album,
                album_artist: None,
                composer: None,
                duration_ms: None,
                track_number: None,
                file_name,
                artwork: None,
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
    let album = tag
        .and_then(|tag| tag.album())
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
        .or(fallback_album);
    let album_artist = tag
        .and_then(|tag| tag.get_string(ItemKey::AlbumArtist))
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty());
    let composer = tag
        .and_then(|tag| tag.get_string(ItemKey::Composer))
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty());
    let track_number = tag.and_then(|tag| tag.track());
    let artwork = tag
        .and_then(|tag| tag.pictures().first())
        .map(|picture| picture.data().to_vec())
        .filter(|bytes| !bytes.is_empty());
    let duration_ms = Some(tagged_file.properties().duration().as_millis().min(u128::from(u64::MAX)) as u64);

    TrackMetadata {
        title,
        artist,
        album,
        album_artist,
        composer,
        duration_ms,
        track_number,
        file_name,
        artwork,
    }
}

fn canonicalize_directory(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }

    let raw_path = PathBuf::from(trimmed);
    let canonical_path = raw_path.canonicalize().unwrap_or(raw_path);
    canonical_path.is_dir().then(|| canonical_path.to_string_lossy().to_string())
}

fn adjust_current_index_after_move(current_index: usize, from: usize, to: usize) -> usize {
    if current_index == from {
        return to;
    }

    if from < current_index && current_index <= to {
        return current_index - 1;
    }

    if to <= current_index && current_index < from {
        return current_index + 1;
    }

    current_index
}