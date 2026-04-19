use crate::application::dto::{LibraryDto, PlaylistDto, SettingsDto, StateDto};
use crate::application::usecases::build_app_vm::build_app_vm;
use crate::application::view_model::{AppVm, QueueVm};
use crate::config::{Config, Language, LogLevel, Theme};
use crate::domain::model::{PlaybackMode, PlaybackState};
use crate::infrastructure::audio;
use crate::infrastructure::decode::SymphoniaDecoder;
use crate::ports::audio_output::{AudioFormat, AudioOutput, SampleFormat};
use crossbeam_channel::{Receiver, Sender};
use crate::infrastructure::lyrics::Lyrics;
use crate::infrastructure::persistence::{MetaCache, PlaylistsFile, StateFile};
use image::io::Reader as ImageReader;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use crate::localization::{language_name, supported_languages, translate};
use crate::presentation::tabs;
use crate::shared::ipc as ipc_mod;
use cpal::traits::{DeviceTrait, HostTrait};
use eframe::{egui, App, CreationContext};
use fontdb::{Database, Family, Source, Style};
use rand::seq::IteratorRandom;
use rand::thread_rng;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Library,
    Play,
    Playlist,
    Album,
    Artist,
    Queue,
    Scan,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtistViewMode {
    Artist,
    AlbumArtist,
    Composer,
}

impl ArtistViewMode {
    pub fn label(self) -> &'static str {
        match self {
            ArtistViewMode::Artist => "Artist",
            ArtistViewMode::AlbumArtist => "Album Artist",
            ArtistViewMode::Composer => "Composer",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            ArtistViewMode::Artist => "artist",
            ArtistViewMode::AlbumArtist => "album_artist",
            ArtistViewMode::Composer => "composer",
        }
    }
}

impl Tab {
    fn label(&self) -> &'static str {
        match self {
            Tab::Library => "Library",
            Tab::Play => "Play",
            Tab::Playlist => "Playlists",
            Tab::Album => "Albums",
            Tab::Artist => "Artists",
            Tab::Queue => "Queue",
            Tab::Scan => "Scan",
            Tab::Settings => "Settings",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "Library" => Some(Tab::Library),
            "Play" => Some(Tab::Play),
            "Playlists" => Some(Tab::Playlist),
            "Albums" => Some(Tab::Album),
            "Artists" => Some(Tab::Artist),
            "Queue" => Some(Tab::Queue),
            "Scan" => Some(Tab::Scan),
            "Settings" => Some(Tab::Settings),
            _ => None,
        }
    }
}

pub struct GuiApp {
    pub app_vm: AppVm,
    active_tab: Tab,
    device_names: Vec<String>,
    selected: Option<usize>,
    device_filter: String,
    pub(crate) play_path: String,
    pub(crate) lyrics_files: Vec<std::path::PathBuf>,
    pub(crate) selected_lyrics_index: Option<usize>,
    pub(crate) loaded_lyrics: Option<Lyrics>,
    pub(crate) playback_state: PlaybackState,
    pub(crate) playback_mode: crate::domain::model::PlaybackMode,
    pub(crate) current_queue_index: Option<usize>,
    playback_status_receiver: Option<Receiver<String>>,
    playback_cancel_sender: Option<Sender<()>>,
    playback_thread_handle: Option<std::thread::JoinHandle<()>>,
    playback_start_instant: Option<Instant>,
    playback_start_ms: u64,
    pending_play_request: bool,
    playback_state_before_seek: Option<PlaybackState>,
    pending_seek_ms: Option<u64>,
    seek_last_change_at: Option<Instant>,
    seek_interacting: bool,
    seek_long_press_start: Option<Instant>,
    seek_key_hold_direction: i8,
    seek_key_hold_started_at: Option<Instant>,
    seek_key_last_step_at: Option<Instant>,
    pub(crate) queue_scroll_to_playing: bool,
    pub(crate) artist_view_mode: ArtistViewMode,
    pub(crate) play_position_ms: u64,
    pub(crate) play_duration_ms: Option<u64>,
    pub(crate) play_title: Option<String>,
    pub(crate) play_artist: Option<String>,
    pub(crate) play_album: Option<String>,
    pub(crate) play_artwork_handle: Option<egui::TextureHandle>,
    pub(crate) play_artwork_source: Option<PathBuf>,
    pub(crate) lyrics_offset_ms: i64,
    pub(crate) play_status: Option<String>,
    scan_target_input: String,
    scan_exclude_input: String,
    scan_targets: Vec<String>,
    pub(crate) scan_excludes: Vec<String>,
    selected_album: Option<String>,
    selected_artist: Option<String>,
    selected_directory: Option<String>,
    favorite_directories: Vec<String>,
    favorite_directory_input: String,
    library_expanded_directories: std::collections::HashSet<String>,
    pub(crate) playlists: Vec<PlaylistDto>,
    pub(crate) new_playlist_name: String,
    pub(crate) selected_playlist: Option<usize>,
    pub(crate) playlist_track_input: String,
    queue: QueueVm,
    pub(crate) queue_item_input: String,
    pub(crate) selected_queue_index: Option<usize>,
    pub(crate) selected_queue_indices: Vec<usize>,
    queue_selection_anchor: Option<usize>,
    pub(crate) queue_handle_hold_index: Option<usize>,
    pub(crate) queue_handle_hold_started_at: Option<Instant>,
    pub(crate) queue_drag_source_index: Option<usize>,
    pub(crate) taps: usize,
    pub(crate) theme: Theme,
    pub(crate) log_level: LogLevel,
    pub(crate) language: Language,
    last_scan_status: Option<String>,
    album_meta_cache: Option<MetaCache>,
    album_artwork_path_cache: HashMap<String, Option<PathBuf>>,
    album_artwork_texture_cache: HashMap<String, egui::TextureHandle>,
    album_artwork_pending: VecDeque<String>,
    album_artwork_queued: HashSet<String>,
    album_artwork_album_titles: Vec<String>,
    track_details_dialog: Option<TrackDetailsDialog>,
}

#[derive(Debug, Clone)]
struct TrackDetailsDialog {
    path: String,
    title: String,
    artist: String,
    album: String,
}

impl GuiApp {
    fn default_app_vm(
        active_tab: Tab,
        config: &Config,
        last_scan_status: Option<String>,
        scan_targets: Vec<String>,
        scan_excludes: Vec<String>,
        library: LibraryDto,
        playlists: &[PlaylistDto],
        queue: QueueVm,
    ) -> AppVm {
        let settings = SettingsDto::new(
            config.preferred_output_device.clone(),
            config.taps_for_device(config.preferred_output_device.as_ref()),
        );
        let state = StateDto {
            last_tab: Some(active_tab.label().to_string()),
            last_play_path: None,
            last_play_position_ms: None,
            last_scan_status,
            scan_scanned: None,
            scan_parsed: None,
            scan_registered: None,
            scan_targets,
            scan_excludes,
            queue_tracks: Vec::new(),
            selected_queue_index: None,
            queue_playback_mode: PlaybackMode::Sequential,
            favorite_directories: Vec::new(),
        };

        build_app_vm("LRC Player", &settings, &state, playlists, &library, queue)
    }

    fn load_state() -> StateDto {
        StateFile::open()
            .and_then(|store| store.load())
            .unwrap_or(StateDto {
                last_tab: None,
                last_play_path: None,
                last_scan_status: None,
                scan_scanned: None,
                scan_parsed: None,
                scan_registered: None,
                scan_targets: Vec::new(),
                scan_excludes: Vec::new(),
                queue_tracks: Vec::new(),
                selected_queue_index: None,
                queue_playback_mode: PlaybackMode::Sequential,
                last_play_position_ms: None,
                favorite_directories: Vec::new(),
            })
    }

    pub(crate) fn save_state(&self) {
        let state = StateDto {
            last_tab: Some(self.active_tab.label().to_string()),
            last_play_path: if self.play_path.is_empty() {
                None
            } else {
                Some(self.play_path.clone())
            },
            last_play_position_ms: Some(self.play_position_ms),
            last_scan_status: self.last_scan_status.clone(),
            scan_scanned: None,
            scan_parsed: None,
            scan_registered: None,
            scan_targets: self.scan_targets.clone(),
            scan_excludes: self.scan_excludes.clone(),
            queue_tracks: self.queue.tracks.clone(),
            selected_queue_index: self.selected_queue_index,
            queue_playback_mode: self.playback_mode,
            favorite_directories: self.favorite_directories.clone(),
        };
        if let Ok(store) = StateFile::open() {
            let _ = store.save(&state);
        }
    }

    fn refresh_scan_status(&mut self) {
        if let Ok(store) = StateFile::open() {
            if let Ok(state) = store.load() {
                self.last_scan_status = state.last_scan_status.clone();
                self.app_vm.scan.last_scan_status = state.last_scan_status.clone();
            }
        }
    }

    fn load_playlists() -> Vec<PlaylistDto> {
        if let Ok(file) = PlaylistsFile::open() {
            let playlists = file.load().unwrap_or_default();
            Self::normalize_playlists(playlists)
        } else {
            Self::normalize_playlists(Vec::new())
        }
    }

    fn normalize_playlists(playlists: Vec<PlaylistDto>) -> Vec<PlaylistDto> {
        let mut favorite_tracks: Vec<String> = Vec::new();
        let mut seen_tracks: HashSet<String> = HashSet::new();
        let mut favorite_id: Option<String> = None;
        let mut others: Vec<PlaylistDto> = Vec::new();

        for playlist in playlists {
            if playlist.name.eq_ignore_ascii_case("favorite") {
                if favorite_id.is_none() {
                    favorite_id = Some(playlist.id.clone());
                }
                for track in playlist.tracks {
                    if seen_tracks.insert(track.clone()) {
                        favorite_tracks.push(track);
                    }
                }
            } else {
                others.push(playlist);
            }
        }

        let favorite = PlaylistDto {
            id: favorite_id.unwrap_or_else(|| "playlist-favorite".to_string()),
            name: "favorite".to_string(),
            tracks: favorite_tracks,
        };

        let mut normalized = Vec::with_capacity(others.len() + 1);
        normalized.push(favorite);
        normalized.extend(others);
        normalized
    }

    fn save_playlists(&self) {
        if let Ok(file) = PlaylistsFile::open() {
            let _ = file.save(&self.playlists);
        }
    }

    pub(crate) fn add_playlist(&mut self) {
        let name = self.new_playlist_name.trim().to_string();
        if name.is_empty() {
            return;
        }
        if name.eq_ignore_ascii_case("favorite") {
            return;
        }
        if self
            .playlists
            .iter()
            .any(|playlist| playlist.name.eq_ignore_ascii_case(&name))
        {
            return;
        }

        let id = format!(
            "playlist-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );

        self.playlists.push(PlaylistDto { id, name, tracks: Vec::new() });
        self.new_playlist_name.clear();
        self.save_playlists();
        self.update_app_vm();
    }

    pub(crate) fn remove_playlist(&mut self, index: usize) {
        if index < self.playlists.len()
            && !self.playlists[index].name.eq_ignore_ascii_case("favorite")
        {
            self.playlists.remove(index);
            if self.selected_playlist == Some(index) {
                self.selected_playlist = None;
            }
            self.save_playlists();
            self.update_app_vm();
        }
    }

    pub(crate) fn move_playlist_up(&mut self, index: usize) {
        if index <= 1 || index >= self.playlists.len() {
            return;
        }
        if self.playlists[index - 1].name.eq_ignore_ascii_case("favorite") {
            return;
        }

        self.playlists.swap(index - 1, index);
        if let Some(selected) = self.selected_playlist {
            if selected == index {
                self.selected_playlist = Some(index - 1);
            } else if selected == index - 1 {
                self.selected_playlist = Some(index);
            }
        }
        self.save_playlists();
        self.update_app_vm();
    }

    pub(crate) fn move_playlist_down(&mut self, index: usize) {
        if index == 0 || index + 1 >= self.playlists.len() {
            return;
        }
        if self.playlists[index].name.eq_ignore_ascii_case("favorite") {
            return;
        }

        self.playlists.swap(index, index + 1);
        if let Some(selected) = self.selected_playlist {
            if selected == index {
                self.selected_playlist = Some(index + 1);
            } else if selected == index + 1 {
                self.selected_playlist = Some(index);
            }
        }
        self.save_playlists();
        self.update_app_vm();
    }

    pub(crate) fn add_queue_item(&mut self) {
        let item = self.queue_item_input.trim().to_string();
        if item.is_empty() {
            return;
        }
        self.queue.tracks.push(item);
        self.queue_item_input.clear();
        self.update_app_vm();
    }

    pub(crate) fn is_queue_item_selected(&self, index: usize) -> bool {
        self.selected_queue_indices.contains(&index)
    }

    pub(crate) fn clear_queue_selection(&mut self) {
        self.selected_queue_index = None;
        self.selected_queue_indices.clear();
        self.queue_selection_anchor = None;
    }

    pub(crate) fn clear_library(&mut self) {
        if let Ok(cache) = MetaCache::open_default() {
            let _ = cache.clear_library();
        }
        self.clear_album_artwork_cache();
        self.selected_album = None;
        self.selected_artist = None;
        self.selected_directory = None;
        self.last_scan_status = Some("Library cleared".to_string());
        self.update_app_vm();
    }

    pub(crate) fn select_queue_item_single(&mut self, index: usize) {
        self.selected_queue_index = Some(index);
        self.selected_queue_indices.clear();
        self.selected_queue_indices.push(index);
        self.queue_selection_anchor = Some(index);
    }

    pub(crate) fn toggle_queue_item_selection(&mut self, index: usize) {
        if let Some(pos) = self.selected_queue_indices.iter().position(|&i| i == index) {
            self.selected_queue_indices.remove(pos);
            self.selected_queue_index = self.selected_queue_indices.last().copied();
            if self.selected_queue_indices.is_empty() {
                self.queue_selection_anchor = None;
            }
        } else {
            self.selected_queue_indices.push(index);
            self.selected_queue_index = Some(index);
            if self.queue_selection_anchor.is_none() {
                self.queue_selection_anchor = Some(index);
            }
        }
    }

    pub(crate) fn select_queue_item_range_to(&mut self, index: usize) {
        let anchor = self.queue_selection_anchor.or(self.selected_queue_index).unwrap_or(index);
        let (start, end) = if anchor <= index {
            (anchor, index)
        } else {
            (index, anchor)
        };
        self.selected_queue_indices = (start..=end).collect();
        self.selected_queue_index = Some(index);
        self.queue_selection_anchor = Some(anchor);
    }

    pub(crate) fn remove_selected_queue_items(&mut self) {
        if self.selected_queue_indices.is_empty() {
            if let Some(index) = self.selected_queue_index {
                self.remove_queue_item(index);
            }
            return;
        }

        let current_index = self.current_queue_index;
        let mut indices = self.selected_queue_indices.clone();
        indices.sort_unstable();
        indices.dedup();
        let current_removed = current_index
            .map(|current| indices.iter().any(|&i| i == current))
            .unwrap_or(false);

        indices.reverse();
        for index in indices.iter().copied() {
            if index < self.queue.tracks.len() {
                self.queue.tracks.remove(index);
            }
        }

        let removed_before_current = current_index.map(|old_current| {
            indices.iter().filter(|&&removed| removed < old_current).count()
        });

        self.clear_queue_selection();

        if current_removed {
            if self.queue.tracks.is_empty() {
                self.clear_queue();
                self.stop_playback_and_clear_play_tab();
            } else if let Some(old_index) = current_index {
                let next_index = if old_index < self.queue.tracks.len() {
                    old_index
                } else {
                    self.queue.tracks.len() - 1
                };
                self.select_queue_item_single(next_index);
                match self.playback_state {
                    PlaybackState::Paused => {
                        self.load_selected_queue_item_paused(next_index);
                    }
                    PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => {
                        self.play_selected_queue_item();
                    }
                    PlaybackState::Stopped => {
                        self.load_selected_queue_item_stopped(next_index);
                    }
                }
            }
        } else if let Some(old_current) = current_index {
            let shift = removed_before_current.unwrap_or(0);
            self.current_queue_index = Some(old_current.saturating_sub(shift));
        }

        self.update_app_vm();
    }

    pub(crate) fn clear_queue_handle_hold(&mut self) {
        self.queue_handle_hold_index = None;
        self.queue_handle_hold_started_at = None;
    }

    pub(crate) fn cancel_queue_drag(&mut self) {
        self.queue_drag_source_index = None;
        self.clear_queue_handle_hold();
    }

    pub(crate) fn move_queue_item_to(&mut self, from: usize, to: usize) {
        if from >= self.queue.tracks.len() || to >= self.queue.tracks.len() || from == to {
            return;
        }

        let item = self.queue.tracks.remove(from);
        let insert_at = if to > from { to - 1 } else { to };
        self.queue.tracks.insert(insert_at, item);

        let remap_index = |idx: usize| {
            if idx == from {
                insert_at
            } else if from < insert_at {
                if idx > from && idx <= insert_at {
                    idx - 1
                } else {
                    idx
                }
            } else if insert_at < from {
                if idx >= insert_at && idx < from {
                    idx + 1
                } else {
                    idx
                }
            } else {
                idx
            }
        };

        self.selected_queue_indices = self
            .selected_queue_indices
            .iter()
            .copied()
            .map(remap_index)
            .collect();
        self.selected_queue_indices.sort_unstable();
        self.selected_queue_indices.dedup();

        self.selected_queue_index = self.selected_queue_index.map(remap_index);
        self.queue_selection_anchor = self.queue_selection_anchor.map(remap_index);
        self.current_queue_index = self.current_queue_index.map(remap_index);

        self.clear_queue_selection();
        self.update_app_vm();
    }

    pub(crate) fn remove_queue_item(&mut self, index: usize) {
        if index < self.queue.tracks.len() {
            let current_removed = self.current_queue_index == Some(index);
            self.queue.tracks.remove(index);
            self.selected_queue_indices = self
                .selected_queue_indices
                .iter()
                .filter_map(|&i| {
                    if i == index {
                        None
                    } else if i > index {
                        Some(i - 1)
                    } else {
                        Some(i)
                    }
                })
                .collect();
            if self.selected_queue_index == Some(index) {
                self.selected_queue_index = self.selected_queue_indices.last().copied();
            } else if let Some(selected) = self.selected_queue_index {
                if selected > index {
                    self.selected_queue_index = Some(selected - 1);
                }
            }
            if let Some(anchor) = self.queue_selection_anchor {
                self.queue_selection_anchor = if anchor == index {
                    self.selected_queue_index
                } else if anchor > index {
                    Some(anchor - 1)
                } else {
                    Some(anchor)
                };
            }

            if current_removed {
                if self.queue.tracks.is_empty() {
                    self.clear_queue();
                    self.stop_playback_and_clear_play_tab();
                } else {
                    let next_index = if index < self.queue.tracks.len() {
                        index
                    } else {
                        self.queue.tracks.len() - 1
                    };
                    self.select_queue_item_single(next_index);
                    match self.playback_state {
                        PlaybackState::Paused => {
                            self.load_selected_queue_item_paused(next_index);
                        }
                        PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => {
                            self.play_selected_queue_item();
                        }
                        PlaybackState::Stopped => {
                            self.load_selected_queue_item_stopped(next_index);
                        }
                    }
                }
            } else {
                self.current_queue_index = if let Some(current) = self.current_queue_index {
                    if current > index {
                        Some(current - 1)
                    } else {
                        Some(current)
                    }
                } else {
                    None
                };
            }

            self.update_app_vm();
        }
    }

    pub(crate) fn move_queue_item_up(&mut self, index: usize) {
        if index == 0 || index >= self.queue.tracks.len() {
            return;
        }
        self.queue.tracks.swap(index, index - 1);
        self.select_queue_item_single(index - 1);
        self.update_app_vm();
    }

    pub(crate) fn move_queue_item_down(&mut self, index: usize) {
        if index + 1 >= self.queue.tracks.len() {
            return;
        }
        self.queue.tracks.swap(index, index + 1);
        self.select_queue_item_single(index + 1);
        self.update_app_vm();
    }

    pub(crate) fn clear_queue(&mut self) {
        self.queue.tracks.clear();
        self.clear_queue_selection();
        self.cancel_queue_drag();
        self.current_queue_index = None;
        self.update_app_vm();
    }

    fn stop_playback_and_clear_play_tab(&mut self) {
        self.signal_playback_stop();
        self.playback_state = PlaybackState::Stopped;
        self.play_status = Some("Playback stopped".to_string());
        self.play_path.clear();
        self.play_position_ms = 0;
        self.play_duration_ms = None;
        self.play_title = None;
        self.play_artist = None;
        self.play_album = None;
        self.play_artwork_handle = None;
        self.play_artwork_source = None;
        self.lyrics_files.clear();
        self.loaded_lyrics = None;
        self.selected_lyrics_index = None;
        self.lyrics_offset_ms = 0;
        self.playback_start_instant = None;
        self.pending_play_request = false;
    }

    fn load_selected_queue_item_paused(&mut self, index: usize) {
        if let Some(path) = self.queue.tracks.get(index).cloned() {
            self.select_queue_item_single(index);
            self.current_queue_index = Some(index);
            self.play_path = path;
            self.play_position_ms = 0;
            self.playback_state = PlaybackState::Paused;
            self.play_status = Some("Paused at start of selected track".to_string());
            self.discover_lyrics_for_current_play_path();
            self.refresh_play_metadata();
            self.playback_start_instant = None;
            self.pending_play_request = false;
        }
    }

    fn load_selected_queue_item_stopped(&mut self, index: usize) {
        if let Some(path) = self.queue.tracks.get(index).cloned() {
            self.select_queue_item_single(index);
            self.current_queue_index = Some(index);
            self.play_path = path;
            self.play_position_ms = 0;
            self.playback_state = PlaybackState::Stopped;
            self.play_status = Some("Stopped at start of selected track".to_string());
            self.discover_lyrics_for_current_play_path();
            self.refresh_play_metadata();
            self.playback_start_instant = None;
            self.pending_play_request = false;
        }
    }

    pub(crate) fn add_track_to_selected_playlist(&mut self) {
        let track = self.playlist_track_input.trim().to_string();
        if track.is_empty() {
            return;
        }
        if let Some(index) = self.selected_playlist {
            if let Some(playlist) = self.playlists.get_mut(index) {
                if !playlist.tracks.contains(&track) {
                    playlist.tracks.push(track.clone());
                    self.playlist_track_input.clear();
                    self.save_playlists();
                    self.update_app_vm();
                }
            }
        }
    }

    pub(crate) fn add_track_to_playlist(&mut self, playlist_index: usize, track: &str) {
        let item = track.trim();
        if item.is_empty() {
            return;
        }
        if let Some(playlist) = self.playlists.get_mut(playlist_index) {
            if !playlist.tracks.iter().any(|existing| existing == item) {
                playlist.tracks.push(item.to_string());
                self.save_playlists();
                self.update_app_vm();
            }
        }
    }

    pub(crate) fn add_track_next_in_queue(&mut self, track: &str) {
        let item = track.trim();
        if item.is_empty() {
            return;
        }

        let insert_at = self
            .current_queue_index
            .or(self.selected_queue_index)
            .map(|index| (index + 1).min(self.queue.tracks.len()))
            .unwrap_or(self.queue.tracks.len());

        self.queue.tracks.insert(insert_at, item.to_string());
        self.update_app_vm();
    }

    pub(crate) fn play_from_displayed_list(&mut self, displayed_tracks: &[String], clicked_index: usize) {
        if displayed_tracks.is_empty() || clicked_index >= displayed_tracks.len() {
            return;
        }

        self.queue.tracks = displayed_tracks.to_vec();
        self.current_queue_index = None;
        self.clear_queue_selection();
        self.select_queue_item_single(clicked_index);
        self.update_app_vm();
        self.play_selected_queue_item();
    }

    pub(crate) fn open_track_details(&mut self, path: &str) {
        let item = path.trim();
        if item.is_empty() {
            return;
        }

        let fallback_title = Path::new(item)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(item)
            .to_string();

        let (title, artist, album) = MetaCache::open_default()
            .ok()
            .and_then(|cache| cache.track_metadata_by_location(item).ok().flatten())
            .map(|(title, artist, album)| {
                let title = if title.trim().is_empty() {
                    fallback_title.clone()
                } else {
                    title
                };
                let artist = if artist.trim().is_empty() {
                    "Unknown artist".to_string()
                } else {
                    artist
                };
                let album = if album.trim().is_empty() {
                    "Unknown album".to_string()
                } else {
                    album
                };
                (title, artist, album)
            })
            .unwrap_or((
                fallback_title,
                "Unknown artist".to_string(),
                "Unknown album".to_string(),
            ));

        self.track_details_dialog = Some(TrackDetailsDialog {
            path: item.to_string(),
            title,
            artist,
            album,
        });
    }

    pub(crate) fn remove_track_from_selected_playlist(&mut self, track_index: usize) {
        if let Some(index) = self.selected_playlist {
            if let Some(playlist) = self.playlists.get_mut(index) {
                if track_index < playlist.tracks.len() {
                    playlist.tracks.remove(track_index);
                    self.save_playlists();
                    self.update_app_vm();
                }
            }
        }
    }

    pub(crate) fn select_album(&mut self, album: Option<String>) {
        self.selected_album = album;
        self.selected_artist = None;
        self.selected_directory = None;
        self.update_app_vm();
    }

    pub(crate) fn select_artist(&mut self, artist: Option<String>) {
        self.selected_artist = artist;
        self.selected_album = None;
        self.selected_directory = None;
        self.update_app_vm();
    }

    pub(crate) fn set_artist_view_mode(&mut self, mode: ArtistViewMode) {
        if self.artist_view_mode == mode {
            return;
        }
        self.artist_view_mode = mode;
        self.selected_artist = None;
        self.update_app_vm();
    }

    pub(crate) fn play_track_now(&mut self, path: &str) {
        if path.trim().is_empty() {
            return;
        }
        self.play_path = path.to_string();
        self.play_position_ms = 0;
        self.playback_state = PlaybackState::Buffering;
        self.discover_lyrics_for_current_play_path();
        self.refresh_play_metadata();
        self.request_play();
    }

    pub(crate) fn enqueue_track_path(&mut self, path: &str) {
        let item = path.trim();
        if item.is_empty() {
            return;
        }
        self.queue.tracks.push(item.to_string());
        self.update_app_vm();
    }

    pub(crate) fn add_tracks_to_favorite_playlist(&mut self, tracks: &[String]) {
        if tracks.is_empty() {
            return;
        }

        let favorite_index = self.ensure_favorite_playlist();
        let mut changed = false;
        if let Some(playlist) = self.playlists.get_mut(favorite_index) {
            for track in tracks {
                if !playlist.tracks.contains(track) {
                    playlist.tracks.push(track.clone());
                    changed = true;
                }
            }
        }

        if changed {
            self.save_playlists();
            self.update_app_vm();
        }
    }

    fn ensure_favorite_playlist(&mut self) -> usize {
        if self.playlists.is_empty() || !self.playlists[0].name.eq_ignore_ascii_case("favorite") {
            self.playlists = Self::normalize_playlists(std::mem::take(&mut self.playlists));
        }
        0
    }

    pub(crate) fn is_track_favorite(&self, track: &str) -> bool {
        self.playlists
            .first()
            .map(|playlist| playlist.tracks.contains(&track.to_string()))
            .unwrap_or(false)
    }

    pub(crate) fn add_track_to_favorite_playlist(&mut self, track: &str) {
        let track = track.trim();
        if track.is_empty() {
            return;
        }
        let favorite_index = self.ensure_favorite_playlist();
        if let Some(playlist) = self.playlists.get_mut(favorite_index) {
            if !playlist.tracks.contains(&track.to_string()) {
                playlist.tracks.push(track.to_string());
                self.save_playlists();
                self.update_app_vm();
            }
        }
    }

    pub(crate) fn remove_track_from_favorite_playlist(&mut self, track: &str) {
        if self.playlists.is_empty() {
            return;
        }
        let favorite_index = self.ensure_favorite_playlist();
        if let Some(playlist) = self.playlists.get_mut(favorite_index) {
            if let Some(pos) = playlist.tracks.iter().position(|existing| existing == track) {
                playlist.tracks.remove(pos);
                self.save_playlists();
                self.update_app_vm();
            }
        }
    }

    pub(crate) fn toggle_favorite_track(&mut self, track: &str) {
        if self.is_track_favorite(track) {
            self.remove_track_from_favorite_playlist(track);
        } else {
            self.add_track_to_favorite_playlist(track);
        }
    }

    pub(crate) fn select_directory(&mut self, directory: Option<String>) {
        self.selected_directory = directory;
        self.selected_album = None;
        self.selected_artist = None;
        self.update_app_vm();
    }

    pub(crate) fn toggle_library_directory_expanded(&mut self, path: &str) {
        if self.library_expanded_directories.contains(path) {
            self.library_expanded_directories.remove(path);
        } else {
            self.library_expanded_directories.insert(path.to_string());
        }
    }

    pub(crate) fn is_library_directory_expanded(&self, path: &str) -> bool {
        self.library_expanded_directories.contains(path)
    }

    fn add_favorite_directory(&mut self) {
        let dir = self.favorite_directory_input.trim().to_string();
        if dir.is_empty() || self.favorite_directories.contains(&dir) {
            return;
        }

        self.favorite_directories.push(dir);
        self.favorite_directory_input.clear();
        self.save_state();
        self.update_app_vm();
    }

    fn remove_favorite_directory(&mut self, index: usize) {
        if index < self.favorite_directories.len() {
            self.favorite_directories.remove(index);
            self.save_state();
            self.update_app_vm();
        }
    }

    fn signal_playback_stop(&mut self) {
        if let Some(tx) = self.playback_cancel_sender.take() {
            let _ = tx.send(());
        }
    }

    fn detach_playback_thread(&mut self) {
        // Dropping JoinHandle detaches the thread, avoiding UI stalls on stop/resume.
        let _ = self.playback_thread_handle.take();
    }

    fn update_playback_position_from_clock(&mut self) {
        if let Some(start_instant) = self.playback_start_instant {
            let elapsed = start_instant.elapsed();
            let elapsed_ms = elapsed.as_millis() as u64;
            let position = self.playback_start_ms.saturating_add(elapsed_ms);
            if let Some(duration) = self.play_duration_ms {
                self.play_position_ms = position.min(duration);
            } else {
                self.play_position_ms = position;
            }
        }
    }

    fn pause_playback(&mut self) {
        self.update_playback_position_from_clock();
        self.signal_playback_stop();
        self.playback_state = PlaybackState::Paused;
        self.play_status = Some("Paused".to_string());
        self.playback_start_instant = None;
        self.pending_play_request = false;
    }

    pub(crate) fn set_seek_interacting(&mut self, interacting: bool) {
        self.seek_interacting = interacting;
    }

    pub(crate) fn is_seek_interacting(&self) -> bool {
        self.seek_interacting
    }

    pub(crate) fn set_seek_long_press_start(&mut self, start: Option<Instant>) {
        self.seek_long_press_start = start;
    }

    pub(crate) fn seek_long_press_elapsed(&self) -> Option<Duration> {
        self.seek_long_press_start.map(|start| start.elapsed())
    }

    fn step_seek_by(&mut self, direction: i8, step_ms: u64, duration_ms: u64) {
        let next = if direction < 0 {
            self.play_position_ms.saturating_sub(step_ms)
        } else {
            self.play_position_ms.saturating_add(step_ms).min(duration_ms)
        };
        if next != self.play_position_ms {
            self.queue_seek_position(next);
        }
    }

    pub(crate) fn handle_play_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        let (space_pressed, left_pressed, right_pressed, left_down, right_down) =
            ctx.input(|input| {
                (
                    input.key_pressed(egui::Key::Space),
                    input.key_pressed(egui::Key::ArrowLeft),
                    input.key_pressed(egui::Key::ArrowRight),
                    input.key_down(egui::Key::ArrowLeft),
                    input.key_down(egui::Key::ArrowRight),
                )
            });

        if space_pressed {
            self.toggle_play_pause();
        }

        let now = Instant::now();
        let Some(duration_ms) = self.play_duration_ms else {
            self.set_seek_interacting(false);
            self.seek_key_hold_direction = 0;
            self.seek_key_hold_started_at = None;
            self.seek_key_last_step_at = None;
            return;
        };

        if left_pressed && !right_pressed {
            self.step_seek_by(-1, 10_000, duration_ms);
            self.seek_key_hold_direction = -1;
            self.seek_key_hold_started_at = Some(now);
            self.seek_key_last_step_at = Some(now);
            self.set_seek_interacting(false);
        } else if right_pressed && !left_pressed {
            self.step_seek_by(1, 10_000, duration_ms);
            self.seek_key_hold_direction = 1;
            self.seek_key_hold_started_at = Some(now);
            self.seek_key_last_step_at = Some(now);
            self.set_seek_interacting(false);
        }

        let hold_direction = match (left_down, right_down) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };

        if hold_direction == 0 {
            self.set_seek_interacting(false);
            self.seek_key_hold_direction = 0;
            self.seek_key_hold_started_at = None;
            self.seek_key_last_step_at = None;
            return;
        }

        if self.seek_key_hold_direction != hold_direction {
            self.seek_key_hold_direction = hold_direction;
            self.seek_key_hold_started_at = Some(now);
            self.seek_key_last_step_at = Some(now);
            self.set_seek_interacting(false);
        }

        let hold_delay = Duration::from_millis(350);
        let repeat_every = Duration::from_millis(120);
        let started_at = self.seek_key_hold_started_at.unwrap_or(now);
        let hold_elapsed = now.duration_since(started_at);
        if hold_elapsed < hold_delay {
            ctx.request_repaint_after(hold_delay - hold_elapsed);
            return;
        }

        self.set_seek_interacting(true);
        let can_step = self
            .seek_key_last_step_at
            .map(|last| now.duration_since(last) >= repeat_every)
            .unwrap_or(true);
        if can_step {
            self.step_seek_by(hold_direction, 1_000, duration_ms);
            self.seek_key_last_step_at = Some(now);
        }

        ctx.request_repaint_after(repeat_every);
    }

    pub(crate) fn queue_seek_position(&mut self, position_ms: u64) {
        self.play_position_ms = position_ms;
        self.pending_seek_ms = Some(position_ms);
        self.seek_last_change_at = Some(Instant::now());
    }

    fn flush_pending_seek(&mut self) {
        let Some(target_ms) = self.pending_seek_ms else {
            return;
        };
        if self.seek_interacting {
            return;
        }

        let debounce = Duration::from_millis(150);
        if let Some(last_change) = self.seek_last_change_at {
            if last_change.elapsed() < debounce {
                return;
            }
        }

        self.pending_seek_ms = None;
        self.seek_last_change_at = None;
        self.play_position_ms = target_ms;

        // Avoid restarting playback when already stopped; keep the chosen position only.
        if self.playback_state != PlaybackState::Stopped {
            self.seek_to_position();
        }
    }

    pub(crate) fn play_selected_queue_item(&mut self) {
        if let Some(index) = self.selected_queue_index {
            if let Some(path) = self.queue.tracks.get(index).cloned() {
                self.select_queue_item_single(index);
                self.current_queue_index = Some(index);
                self.play_path = path;
                self.play_position_ms = 0;
                self.playback_state = PlaybackState::Buffering;
                self.discover_lyrics_for_current_play_path();
                self.refresh_play_metadata();
                self.request_play();
            }
        }
    }

    pub(crate) fn seek_to_position(&mut self) {
        if self.play_path.trim().is_empty() {
            return;
        }
        if self.play_duration_ms.is_none() {
            self.refresh_play_metadata();
        }

        let target_ms = self.play_position_ms;
        match self.playback_state {
            PlaybackState::Stopped | PlaybackState::Paused => {
                self.play_status = Some(format!("Seek position updated: {} ms", target_ms));
            }
            _ => {
                self.playback_state_before_seek = Some(self.playback_state);
                self.playback_state = PlaybackState::Seeking;
                self.play_status = Some(format!("Seeking to {} ms", target_ms));
                self.request_play();
            }
        }
    }

    pub(crate) fn play_current_queue_item(&mut self) {
        let index = self
            .current_queue_index
            .or(self.selected_queue_index)
            .or_else(|| if self.queue.tracks.is_empty() { None } else { Some(0) });
        if let Some(index) = index {
            if let Some(path) = self.queue.tracks.get(index).cloned() {
                self.select_queue_item_single(index);
                self.current_queue_index = Some(index);
                self.play_path = path;
                self.playback_state = PlaybackState::Buffering;
                self.discover_lyrics_for_current_play_path();
                self.refresh_play_metadata();
                self.request_play();
            }
        }
    }

    pub(crate) fn play_next(&mut self) {
        let len = self.queue.tracks.len();
        if len == 0 {
            return;
        }
        let current = self
            .current_queue_index
            .or(self.selected_queue_index)
            .unwrap_or(0);

        let next_index = match self.playback_mode {
            PlaybackMode::Sequential => current.checked_add(1).filter(|&index| index < len),
            PlaybackMode::RepeatOne => Some(current),
            PlaybackMode::RepeatAll => {
                if current + 1 < len {
                    Some(current + 1)
                } else {
                    Some(0)
                }
            }
            PlaybackMode::Shuffle => {
                let mut rng = thread_rng();
                let choices: Vec<usize> = (0..len).filter(|&i| i != current).collect();
                if choices.is_empty() {
                    Some(current)
                } else {
                    choices.into_iter().choose(&mut rng)
                }
            }
        };

        if let Some(index) = next_index {
            self.select_queue_item_single(index);
            self.play_selected_queue_item();
        }
    }

    pub(crate) fn play_previous(&mut self) {
        let len = self.queue.tracks.len();
        if len == 0 {
            return;
        }
        let current = self
            .current_queue_index
            .or(self.selected_queue_index)
            .unwrap_or(0);

        let prev_index = match self.playback_mode {
            PlaybackMode::Sequential => current.checked_sub(1),
            PlaybackMode::RepeatOne => Some(current),
            PlaybackMode::RepeatAll => {
                if current == 0 {
                    Some(len - 1)
                } else {
                    Some(current - 1)
                }
            }
            PlaybackMode::Shuffle => {
                let mut rng = thread_rng();
                let choices: Vec<usize> = (0..len).filter(|&i| i != current).collect();
                if choices.is_empty() {
                    Some(current)
                } else {
                    choices.into_iter().choose(&mut rng)
                }
            }
        };

        if let Some(index) = prev_index {
            self.select_queue_item_single(index);
            self.play_selected_queue_item();
        }
    }

    pub(crate) fn toggle_play_pause(&mut self) {
        match self.playback_state {
            PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => {
                self.pause_playback();
            }
            PlaybackState::Paused | PlaybackState::Stopped => {
                if self.play_path.trim().is_empty() {
                    if !self.queue.tracks.is_empty() {
                        self.play_current_queue_item();
                        return;
                    }
                    self.play_status = Some("No track loaded".to_string());
                    return;
                }
                self.playback_state = PlaybackState::Buffering;
                self.play_status = Some("Resuming playback".to_string());
                self.request_play();
            }
        }
    }

    pub(crate) fn refresh_play_metadata(&mut self) {
        let path = PathBuf::from(self.play_path.clone());
        if path.as_os_str().is_empty() || !path.exists() {
            self.play_duration_ms = None;
            self.play_title = None;
            self.play_artist = None;
            self.play_album = None;
            self.play_artwork_handle = None;
            self.play_artwork_source = None;
            return;
        }

        let decoder = SymphoniaDecoder::new();
        if let Ok(decoded) = decoder.decode_file(&path) {
            let frames = decoded.frames() as u64;
            let sample_rate = decoded.sample_rate as u64;
            if sample_rate > 0 {
                self.play_duration_ms = Some(frames.saturating_mul(1000) / sample_rate);
                if let Some(duration_ms) = self.play_duration_ms {
                    self.play_position_ms = self.play_position_ms.min(duration_ms);
                }
            }
            self.play_title = decoded.title;
            self.play_artist = decoded.artist;
            self.play_album = decoded.album;
        }
    }

    pub(crate) fn ensure_play_artwork(&mut self, ctx: &egui::Context) {
        let path = Path::new(&self.play_path);
        if path.as_os_str().is_empty() || !path.exists() {
            self.play_artwork_handle = None;
            self.play_artwork_source = None;
            return;
        }

        let artwork_path = self.discover_artwork_for_track(path);
        if artwork_path.is_none() {
            self.play_artwork_handle = None;
            self.play_artwork_source = None;
            return;
        }

        let artwork_path = artwork_path.unwrap();
        if self.play_artwork_source.as_ref() == Some(&artwork_path) && self.play_artwork_handle.is_some() {
            return;
        }

        let image_result: Result<image::DynamicImage, image::ImageError> = (|| {
            let reader = ImageReader::open(&artwork_path).map_err(image::ImageError::IoError)?;
            let reader = reader.with_guessed_format().map_err(image::ImageError::IoError)?;
            reader.decode()
        })();

        match image_result {
            Ok(image) => {
                let size = [image.width() as usize, image.height() as usize];
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    size,
                    &image.to_rgba8().into_vec(),
                );
                let texture = ctx.load_texture("play_artwork", color_image, egui::TextureOptions::default());
                self.play_artwork_handle = Some(texture);
                self.play_artwork_source = Some(artwork_path);
            }
            Err(_) => {
                self.play_artwork_handle = None;
                self.play_artwork_source = None;
            }
        }
    }

    fn clear_album_artwork_cache(&mut self) {
        self.album_artwork_path_cache.clear();
        self.album_artwork_texture_cache.clear();
        self.album_artwork_pending.clear();
        self.album_artwork_queued.clear();
        self.album_artwork_album_titles.clear();
        self.album_meta_cache = None;
    }

    fn sync_album_artwork_cache_with_library(&mut self) {
        let current_titles: Vec<String> = self
            .app_vm
            .library
            .albums
            .iter()
            .map(|album| album.title.clone())
            .collect();

        if current_titles == self.album_artwork_album_titles {
            return;
        }

        let keep: HashSet<String> = current_titles.iter().cloned().collect();
        self.album_artwork_path_cache
            .retain(|title, _| keep.contains(title));
        self.album_artwork_texture_cache
            .retain(|title, _| keep.contains(title));
        self.album_artwork_queued.retain(|title| keep.contains(title));
        self.album_artwork_pending
            .retain(|title| keep.contains(title));
        self.album_artwork_album_titles = current_titles;
    }

    fn resolve_album_artwork_path(&mut self, album_title: &str) -> Option<PathBuf> {
        if self.album_meta_cache.is_none() {
            self.album_meta_cache = MetaCache::open_default().ok();
        }

        let cache = self.album_meta_cache.as_ref()?;
        let candidates = cache.album_location_paths(album_title, 32).ok()?;
        for location in candidates {
            if let Some(path) = self.discover_artwork_for_track(Path::new(&location)) {
                return Some(path);
            }
        }
        None
    }

    pub(crate) fn queue_album_artwork_for_visible(&mut self, visible_album_titles: &[String]) {
        for title in visible_album_titles {
            if self.album_artwork_texture_cache.contains_key(title) {
                continue;
            }
            if self.album_artwork_queued.insert(title.clone()) {
                self.album_artwork_pending.push_back(title.clone());
            }
        }
    }

    pub(crate) fn process_album_artwork_queue(&mut self, ctx: &egui::Context, budget: usize) {
        if budget == 0 {
            return;
        }

        for _ in 0..budget {
            let Some(title) = self.album_artwork_pending.pop_front() else {
                break;
            };
            self.album_artwork_queued.remove(&title);

            if self.album_artwork_texture_cache.contains_key(&title) {
                continue;
            }

            let artwork_path = if let Some(cached) = self.album_artwork_path_cache.get(&title).cloned() {
                cached
            } else {
                let resolved = self.resolve_album_artwork_path(&title);
                self.album_artwork_path_cache
                    .insert(title.clone(), resolved.clone());
                resolved
            };

            let Some(path) = artwork_path else {
                continue;
            };

            let image_result: Result<image::DynamicImage, image::ImageError> = (|| {
                let reader = ImageReader::open(&path).map_err(image::ImageError::IoError)?;
                let reader = reader.with_guessed_format().map_err(image::ImageError::IoError)?;
                reader.decode()
            })();

            if let Ok(image) = image_result {
                let size = [image.width() as usize, image.height() as usize];
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    size,
                    &image.to_rgba8().into_vec(),
                );
                let texture_id = format!("album_artwork:{}", title);
                let texture = ctx.load_texture(texture_id, color_image, egui::TextureOptions::default());
                self.album_artwork_texture_cache.insert(title, texture);
            }
        }

        if !self.album_artwork_pending.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    pub(crate) fn album_artwork_texture(&self, album_title: &str) -> Option<egui::TextureHandle> {
        self.album_artwork_texture_cache.get(album_title).cloned()
    }

    fn discover_artwork_for_track(&self, path: &Path) -> Option<PathBuf> {
        let parent = path.parent()?;
        let stem = path.file_stem()?.to_string_lossy();
        let candidates = [
            format!("{}.jpg", stem),
            format!("{}.jpeg", stem),
            format!("{}.png", stem),
            format!("{}.webp", stem),
            format!("folder.jpg"),
            format!("cover.jpg"),
            format!("front.jpg"),
            format!("folder.png"),
            format!("cover.png"),
            format!("front.png"),
        ];

        for candidate in candidates.iter() {
            let candidate_path = parent.join(candidate);
            if candidate_path.exists() {
                return Some(candidate_path);
            }
        }
        None
    }

    pub(crate) fn discover_lyrics_for_current_play_path(&mut self) {
        if self.play_path.is_empty() {
            self.lyrics_files.clear();
            self.selected_lyrics_index = None;
            self.loaded_lyrics = None;
            return;
        }

        self.lyrics_files = Lyrics::discover_for_track(&self.play_path);
        self.selected_lyrics_index = None;
        self.loaded_lyrics = None;
        if let Some(_) = self.lyrics_files.get(0) {
            self.load_lyrics_file(0);
        }
    }

    pub(crate) fn load_lyrics_file(&mut self, index: usize) {
        if let Some(path) = self.lyrics_files.get(index) {
            match Lyrics::from_lrc(path) {
                Ok(lyrics) => {
                    self.loaded_lyrics = Some(lyrics);
                    self.selected_lyrics_index = Some(index);
                }
                Err(err) => {
                    eprintln!("Unable to load lyrics file {}: {}", path.display(), err);
                }
            }
        }
    }

    fn library_dto(&self) -> LibraryDto {
        let mut library = LibraryDto {
            album_count: 0,
            track_count: 0,
            recent_locations: Vec::new(),
            recent_directories: Vec::new(),
            directory_tree: Vec::new(),
            favorite_directories: self.favorite_directories.clone(),
            albums: Vec::new(),
            selected_album: self.selected_album.clone(),
            selected_album_tracks: Vec::new(),
            artists: Vec::new(),
            selected_artist: self.selected_artist.clone(),
            selected_artist_tracks: Vec::new(),
            selected_directory: self.selected_directory.clone(),
            selected_directory_tracks: Vec::new(),
        };

        if let Ok(cache) = MetaCache::open_default() {
            if let Ok((album_count, track_count)) = cache.library_counts() {
                library.album_count = album_count;
                library.track_count = track_count;
            }
            if let Ok(recent_locations) = cache.recent_locations(20) {
                library.recent_locations = recent_locations;
            }
            if let Ok(recent_directories) = cache.recent_directories(20) {
                library.recent_directories = recent_directories;
            }
            if let Ok(directory_tree) = cache.library_directories(20) {
                library.directory_tree = directory_tree;
            }
            if let Ok(albums) = cache.library_albums() {
                library.albums = albums;
            }
            if let Ok(artists) = cache.library_artists_by_kind(self.artist_view_mode.key()) {
                library.artists = artists;
            }
            if let Some(album) = self.selected_album.as_deref() {
                if let Ok(tracks) = cache.library_tracks_by_album_id(album) {
                    library.selected_album_tracks = tracks;
                }
            }
            if let Some(artist) = self.selected_artist.as_deref() {
                if let Ok(tracks) = cache.library_track_locations_by_artist_kind(
                    self.artist_view_mode.key(),
                    artist,
                ) {
                    library.selected_artist_tracks = tracks;
                }
            }
            if let Some(directory) = self.selected_directory.as_deref() {
                if let Ok(tracks) = cache.library_tracks_by_directory(directory) {
                    library.selected_directory_tracks = tracks;
                }
            }
        }

        library
    }

    pub(crate) fn update_app_vm(&mut self) {
        let cfg = Config::load();
        self.language = cfg.language;
        let library = self.library_dto();
        self.app_vm = Self::default_app_vm(
            self.active_tab,
            &cfg,
            self.last_scan_status.clone(),
            self.scan_targets.clone(),
            self.scan_excludes.clone(),
            library,
            &self.playlists,
            self.queue.clone(),
        );
        self.sync_album_artwork_cache_with_library();
        self.save_state();
    }

    fn request_scan(&mut self) {
        let paths = self.scan_targets.clone();
        if paths.is_empty() {
            self.last_scan_status = Some("No scan targets configured".to_string());
            self.save_state();
            self.update_app_vm();
            return;
        }

        self.last_scan_status = Some("Requested scan".to_string());
        self.save_state();
        self.update_app_vm();

        let socket = ipc_mod::default_socket_path();
        let request = ipc_mod::ScanRequest {
            paths,
            excludes: self.scan_excludes.clone(),
            cancel: false,
        };
        std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                let _ = rt.block_on(async move {
                    ipc_mod::send_scan_request(&socket, &request).await
                });
            }
        });
    }

    pub(crate) fn request_play(&mut self) {
        let path = std::path::PathBuf::from(self.play_path.clone());
        if path.as_os_str().is_empty() {
            self.play_status = Some("Play path is empty.".to_string());
            return;
        }
        if !path.exists() {
            self.play_status = Some(format!("Play path does not exist: {}", path.display()));
            return;
        }

        let is_seeking = self.playback_state == PlaybackState::Seeking;
        if self.playback_thread_handle.is_some() {
            self.signal_playback_stop();
            self.pending_play_request = true;
            if !is_seeking {
                self.playback_state = PlaybackState::Buffering;
            }
            self.play_status = Some("Stopping previous playback...".to_string());
            return;
        }

        self.pending_play_request = false;
        if !is_seeking {
            self.playback_state = PlaybackState::Buffering;
        }
        self.play_status = Some(format!("Starting playback: {}", path.display()));
        self.playback_start_instant = Some(Instant::now());
        self.playback_start_ms = self.play_position_ms;
        self.save_state();

        let start_ms = Some(self.play_position_ms);
        let (status_tx, status_rx) = crossbeam_channel::unbounded();
        let (stop_tx, stop_rx) = crossbeam_channel::bounded::<()>(1);
        self.playback_status_receiver = Some(status_rx);
        self.playback_cancel_sender = Some(stop_tx);
        let preferred_device = self
            .selected
            .and_then(|idx| self.device_names.get(idx).cloned());
        let taps = self.taps;

        let handle = std::thread::spawn(move || {
            let mut output = audio::CpalOutput::new();

            let requested = AudioFormat {
                sample_rate: 44_100,
                channels: 2,
                sample_format: SampleFormat::F32,
            };

            let actual = match output.start_with_device(requested, preferred_device) {
                Ok(fmt) => fmt,
                Err(e) => {
                    let _ = status_tx.send(format!("Playback failed to start output: {}", e));
                    return;
                }
            };

            let producer_tx = match output.producer_sender() {
                Some(tx) => tx,
                None => {
                    let _ = status_tx.send("Playback failed: producer channel unavailable".to_string());
                    let _ = output.stop();
                    return;
                }
            };

            let pool = output.pool_handle();
            let chunk_frames = 2048;

            let _ = status_tx.send(format!(
                "Playback running: {} Hz / {} ch",
                actual.sample_rate, actual.channels
            ));

            let decode_handle = audio::decoder::spawn_decode_to_producer(
                path,
                chunk_frames,
                actual.sample_rate,
                actual.channels,
                producer_tx,
                pool,
                taps,
                start_ms,
                stop_rx.clone(),
            );

            loop {
                if stop_rx.try_recv().is_ok() {
                    let _ = status_tx.send("Playback stopped".to_string());
                    let _ = output.stop();
                    return;
                }

                if decode_handle.is_finished() {
                    match decode_handle.join() {
                        Ok(Ok(())) => {
                            break;
                        }
                        Ok(Err(e)) => {
                            let _ = status_tx.send(format!("Playback failed: {}", e));
                            let _ = output.stop();
                            return;
                        }
                        Err(_) => {
                            let _ = status_tx.send("Playback failed: decode thread panicked".to_string());
                            let _ = output.stop();
                            return;
                        }
                    }
                }

                std::thread::sleep(std::time::Duration::from_millis(5));
            }

            // Keep output alive long enough to drain buffered samples, but remain interruptible.
            for _ in 0..150 {
                if stop_rx.try_recv().is_ok() {
                    let _ = status_tx.send("Playback stopped".to_string());
                    let _ = output.stop();
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            if let Err(e) = output.stop() {
                let _ = status_tx.send(format!("Playback stop warning: {}", e));
            }

            let _ = status_tx.send("Playback finished".to_string());
        });
        self.playback_thread_handle = Some(handle);
        // Reflect user action immediately; runtime status updates can still override this.
        self.playback_state = PlaybackState::Playing;
        self.play_status = Some("Playback starting...".to_string());
    }

    fn poll_playback_status(&mut self) {
        if let Some(rx) = &self.playback_status_receiver {
            let mut playback_ended = false;
            let mut playback_failed = false;
            let mut playback_finished = false;
            while let Ok(status) = rx.try_recv() {
                if status.starts_with("Playback running") {
                    self.playback_state = PlaybackState::Playing;
                } else if status == "Playback finished" {
                    self.playback_state = PlaybackState::Stopped;
                    self.playback_start_instant = None;
                    playback_ended = true;
                    playback_finished = true;
                } else if status == "Playback stopped" {
                    if self.playback_state != PlaybackState::Paused {
                        self.playback_state = PlaybackState::Stopped;
                    }
                    self.playback_start_instant = None;
                    playback_ended = true;
                } else if status.starts_with("Playback failed") {
                    self.playback_state = PlaybackState::Stopped;
                    self.playback_start_instant = None;
                    playback_ended = true;
                    playback_failed = true;
                }
                self.play_status = Some(status);
            }
            if playback_ended {
                self.detach_playback_thread();
                self.playback_cancel_sender = None;
                self.playback_status_receiver = None;
                if self.pending_play_request {
                    self.pending_play_request = false;
                    self.request_play();
                    return;
                }
                if playback_failed {
                    if self.current_queue_index.is_some() || self.selected_queue_index.is_some() {
                        self.play_status = Some("Playback failed, skipping to next queued track".to_string());
                        self.play_next();
                        return;
                    }
                }
                if playback_finished {
                    if self.current_queue_index.is_some() || self.selected_queue_index.is_some() {
                        self.play_status = Some("Track finished, advancing to next queued track".to_string());
                        self.play_next();
                        return;
                    }
                }
            }
        }
        if self.playback_state == PlaybackState::Playing
            && !self.seek_interacting
            && self.pending_seek_ms.is_none()
        {
            self.update_playback_position_from_clock();
        }
        self.flush_pending_seek();
    }

    fn request_cancel(&mut self) {
        self.last_scan_status = Some("Requested scan cancel".to_string());
        self.save_state();
        self.update_app_vm();

        let socket = ipc_mod::default_socket_path();
        let request = ipc_mod::ScanRequest {
            paths: Vec::new(),
            excludes: self.scan_excludes.clone(),
            cancel: true,
        };
        std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                let _ = rt.block_on(async move {
                    ipc_mod::send_scan_request(&socket, &request).await
                });
            }
        });
    }

    fn add_scan_target(&mut self) {
        let target = self.scan_target_input.trim().to_string();
        if !target.is_empty() {
            self.scan_targets.push(target);
            self.scan_target_input.clear();
            self.save_state();
            self.update_app_vm();
        }
    }

    fn remove_scan_target(&mut self, index: usize) {
        if index < self.scan_targets.len() {
            self.scan_targets.remove(index);
            self.save_state();
            self.update_app_vm();
        }
    }

    fn add_scan_exclude(&mut self) {
        let exclude = self.scan_exclude_input.trim().to_string();
        if !exclude.is_empty() {
            self.scan_excludes.push(exclude);
            self.scan_exclude_input.clear();
            self.save_state();
            self.update_app_vm();
        }
    }

    fn remove_scan_exclude(&mut self, index: usize) {
        if index < self.scan_excludes.len() {
            self.scan_excludes.remove(index);
            self.save_state();
            self.update_app_vm();
        }
    }

    fn setup_fonts(cc: &CreationContext<'_>) {
        let mut fonts = egui::FontDefinitions::default();
        let fallback_fonts = Self::discover_cjk_fallback_fonts();

        if !fallback_fonts.is_empty() {
            for (name, bytes) in fallback_fonts.iter() {
                fonts
                    .font_data
                    .insert(name.clone(), egui::FontData::from_owned(bytes.clone()));
            }

            let family = fonts.families.entry(egui::FontFamily::Proportional).or_default();
            for (name, _) in fallback_fonts.iter().rev() {
                if !family.contains(name) {
                    family.insert(0, name.clone());
                }
            }

            let monospace = fonts.families.entry(egui::FontFamily::Monospace).or_default();
            for (name, _) in fallback_fonts.iter().rev() {
                if !monospace.contains(name) {
                    monospace.insert(0, name.clone());
                }
            }
        }

        cc.egui_ctx.set_fonts(fonts);
    }

    fn discover_cjk_fallback_fonts() -> Vec<(String, Vec<u8>)> {
        let mut database = Database::new();
        database.load_system_fonts();

        let candidates = [
            "Noto Sans CJK SC",
            "Source Han Sans SC",
            "Microsoft YaHei",
            "PingFang SC",
            "Noto Sans CJK TC",
            "Source Han Sans TC",
            "Microsoft JhengHei",
            "Noto Sans CJK JP",
            "Source Han Sans JP",
            "Noto Sans CJK KR",
            "Source Han Sans KR",
            "Noto Sans JP",
            "Noto Sans SC",
            "Noto Sans TC",
            "IPAPGothic",
            "WenQuanYi Zen Hei",
            "Hiragino Sans",
        ];

        let mut loaded = Vec::new();
        let mut seen = HashSet::new();

        for family_name in candidates.iter() {
            let query = fontdb::Query {
                families: &[Family::Name(family_name)],
                weight: fontdb::Weight::NORMAL,
                stretch: fontdb::Stretch::Normal,
                style: Style::Normal,
            };

            if let Some(face_id) = database.query(&query) {
                if let Some(face) = database.face(face_id) {
                    let font_bytes = match &face.source {
                        Source::File(path) => fs::read(path).ok(),
                        Source::Binary(data) => Some(data.as_ref().as_ref().to_vec()),
                        _ => None,
                    };

                    if let Some(bytes) = font_bytes {
                        let key = family_name.replace(' ', "_");
                        if seen.insert(key.clone()) {
                            loaded.push((key, bytes));
                        }
                    }
                }
            }
        }

        loaded
    }

    pub fn new(cc: &CreationContext<'_>) -> Self {
        Self::setup_fonts(cc);
        let mut device_names = Vec::new();
        if let Ok(devs) = cpal::default_host().output_devices() {
            for d in devs {
                if let Ok(name) = d.name() {
                    device_names.push(name);
                }
            }
        }
        let cfg = Config::load();
        let selected = cfg
            .preferred_output_device
            .as_ref()
            .and_then(|n| device_names.iter().position(|s| s == n));
        let state = Self::load_state();
        let active_tab = state
            .last_tab
            .as_deref()
            .and_then(Tab::from_label)
            .unwrap_or(Tab::Library);

        let playlists = Self::load_playlists();
        let queue = QueueVm::new(state.queue_tracks.clone());
        let selected_queue_index = state.selected_queue_index.filter(|&i| i < queue.tracks.len());

        let mut gui = GuiApp {
            app_vm: Self::default_app_vm(
                active_tab,
                &cfg,
                state.last_scan_status.clone(),
                state.scan_targets.clone(),
                state.scan_excludes.clone(),
                LibraryDto {
                    album_count: 0,
                    track_count: 0,
                    recent_locations: Vec::new(),
                    recent_directories: Vec::new(),
                    directory_tree: Vec::new(),
                    favorite_directories: state.favorite_directories.clone(),
                    albums: Vec::new(),
                    selected_album: None,
                    selected_album_tracks: Vec::new(),
                    artists: Vec::new(),
                    selected_artist: None,
                    selected_artist_tracks: Vec::new(),
                    selected_directory: None,
                    selected_directory_tracks: Vec::new(),
                },
                &playlists,
                queue.clone(),
            ),
            active_tab,
            device_names,
            selected,
            device_filter: String::new(),
            play_path: state.last_play_path.clone().unwrap_or_default(),
            lyrics_files: Vec::new(),
            selected_lyrics_index: None,
            loaded_lyrics: None,
            play_position_ms: state.last_play_position_ms.unwrap_or(0),
            play_duration_ms: None,
            play_title: None,
            play_artist: None,
            play_album: None,
            play_artwork_handle: None,
            play_artwork_source: None,
            lyrics_offset_ms: 0,
            play_status: None,
            playback_state: PlaybackState::Stopped,
            playback_mode: state.queue_playback_mode,
            current_queue_index: selected_queue_index,
            playback_status_receiver: None,
            playback_cancel_sender: None,
            playback_thread_handle: None,
            playback_start_instant: None,
            playback_start_ms: state.last_play_position_ms.unwrap_or(0),
            pending_play_request: false,
            playback_state_before_seek: None,
            pending_seek_ms: None,
            seek_last_change_at: None,
            seek_interacting: false,
            seek_long_press_start: None,
            seek_key_hold_direction: 0,
            seek_key_hold_started_at: None,
            seek_key_last_step_at: None,
            queue_scroll_to_playing: active_tab == Tab::Queue,
            artist_view_mode: ArtistViewMode::Artist,
            scan_target_input: String::new(),
            scan_exclude_input: String::new(),
            scan_targets: state.scan_targets,
            scan_excludes: state.scan_excludes,
            selected_album: None,
            selected_artist: None,
            selected_directory: None,
            favorite_directories: state.favorite_directories,
            favorite_directory_input: String::new(),
            library_expanded_directories: std::collections::HashSet::new(),
            playlists,
            new_playlist_name: String::new(),
            selected_playlist: None,
            playlist_track_input: String::new(),
            queue,
            queue_item_input: String::new(),
            selected_queue_index,
            selected_queue_indices: selected_queue_index.into_iter().collect(),
            queue_selection_anchor: selected_queue_index,
            queue_handle_hold_index: None,
            queue_handle_hold_started_at: None,
            queue_drag_source_index: None,
            taps: cfg.taps_for_device(cfg.preferred_output_device.as_ref()),
            theme: cfg.theme,
            log_level: cfg.log_level,
            language: cfg.language,
            last_scan_status: state.last_scan_status,
            album_meta_cache: MetaCache::open_default().ok(),
            album_artwork_path_cache: HashMap::new(),
            album_artwork_texture_cache: HashMap::new(),
            album_artwork_pending: VecDeque::new(),
            album_artwork_queued: HashSet::new(),
            album_artwork_album_titles: Vec::new(),
            track_details_dialog: None,
        };

        gui.discover_lyrics_for_current_play_path();
        gui.refresh_play_metadata();
        gui.update_app_vm();
        gui
    }
}

impl App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.active_tab == Tab::Scan {
            self.refresh_scan_status();
        }
        self.poll_playback_status();

        // eframe is mostly event-driven; request periodic repaint while playback is active
        // so seek bar/time visualization does not stall for long intervals.
        if matches!(
            self.playback_state,
            PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking
        ) || self.pending_seek_ms.is_some()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.heading(translate("title", self.language));
            ui.horizontal(|ui| {
                for tab in [
                    Tab::Library,
                    Tab::Play,
                    Tab::Playlist,
                    Tab::Album,
                    Tab::Artist,
                    Tab::Queue,
                    Tab::Scan,
                    Tab::Settings,
                ] {
                    if ui
                        .selectable_label(self.active_tab == tab, tab.label())
                        .clicked()
                    {
                        self.active_tab = tab;
                        if tab == Tab::Queue {
                            self.queue_scroll_to_playing = true;
                        }
                        self.update_app_vm();
                    }
                }
            });
        });

        egui::SidePanel::left("side_panel").resizable(false).show(ctx, |ui| {
            ui.heading(translate("output_device", self.language));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label(translate("filter", self.language));
                ui.text_edit_singleline(&mut self.device_filter);
            });

            let filter = self.device_filter.to_lowercase();
            let filtered: Vec<(usize, &String)> = if filter.is_empty() {
                self.device_names.iter().enumerate().collect()
            } else {
                self.device_names
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| n.to_lowercase().contains(&filter))
                    .collect()
            };

            if filtered.is_empty() {
                ui.label(translate("no_matching_devices", self.language));
            } else {
                let default_sel = self.selected.unwrap_or(0);
                let mut sel_in_filtered = 0usize;
                for (i, (orig_idx, _)) in filtered.iter().enumerate() {
                    if *orig_idx == default_sel {
                        sel_in_filtered = i;
                        break;
                    }
                }

                egui::ComboBox::from_id_source("device_combo")
                    .selected_text(filtered.get(sel_in_filtered).map(|(_, s)| s.as_str()).unwrap_or(""))
                    .show_ui(ui, |ui| {
                        for (i, (_orig_idx, name)) in filtered.iter().enumerate() {
                            ui.selectable_value(&mut sel_in_filtered, i, name.as_str());
                        }
                    });

                if let Some((orig_idx, _)) = filtered.get(sel_in_filtered) {
                    let new_selected = Some(*orig_idx);
                    if self.selected != new_selected {
                        self.selected = new_selected;
                        if let Some(name) = self.device_names.get(*orig_idx) {
                            let cfg = Config::load();
                            self.taps = cfg.taps_for_device(Some(name));
                        }
                    }
                }
            }

            if ui.button(translate("save_device", self.language)).clicked() {
                if let Some(idx) = self.selected {
                    let name = self.device_names[idx].clone();
                    let mut cfg = Config::load();
                    cfg.preferred_output_device = Some(name.clone());
                    if let Some(settings) = cfg.device_settings.get(&name) {
                        self.taps = settings.resampler_taps;
                    }
                    let _ = cfg.save();
                    self.update_app_vm();
                }
            }

            ui.add_space(8.0);
            ui.label(translate("resampler_taps", self.language));
            ui.add(egui::DragValue::new(&mut self.taps).clamp_range(4..=256));
            if ui.button(translate("save_taps", self.language)).clicked() {
                let mut cfg = Config::load();
                if let Some(idx) = self.selected {
                    let name = self.device_names[idx].clone();
                    cfg.set_taps_for_device(name, self.taps);
                } else {
                    cfg.resampler_taps = self.taps;
                }
                let _ = cfg.save();
                self.update_app_vm();
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(translate("language", self.language));
                let mut selected_lang = self.language;
                egui::ComboBox::from_id_source("language_combo")
                    .selected_text(language_name(self.language))
                    .show_ui(ui, |ui| {
                        for lang in supported_languages() {
                            ui.selectable_value(&mut selected_lang, *lang, language_name(*lang));
                        }
                    });

                if selected_lang != self.language {
                    self.language = selected_lang;
                }
            });

            if ui.button("Save language").clicked() {
                let mut cfg = Config::load();
                cfg.language = self.language;
                let _ = cfg.save();
                self.update_app_vm();
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.active_tab {
                    Tab::Library => tabs::library_tab::ui(ui, self),
                Tab::Play => tabs::play_tab::ui(ui, self),
                Tab::Playlist => tabs::playlist_tab::ui(ui, self),
                Tab::Album => tabs::album_tab::ui(ui, self),
                Tab::Artist => tabs::artist_tab::ui(ui, self),
                Tab::Queue => tabs::queue_tab::ui(ui, self),
                Tab::Scan => tabs::scan_tab::ui(ui, self),
                Tab::Settings => tabs::settings_tab::ui(ui, self),
            }

            if self.active_tab == Tab::Scan {
                ui.separator();
                ui.heading("Scan Controls");
                ui.label("Configure scan targets and exclude paths, then press Request scan.");

                ui.group(|ui| {
                    ui.label("Scan target path:");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.scan_target_input);
                        if ui.button("Add target").clicked() {
                            self.add_scan_target();
                        }
                    });

                    if !self.scan_targets.is_empty() {
                        let targets = self.scan_targets.clone();
                        for (index, target) in targets.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(format!("• {}", target));
                                if ui.button("Remove").clicked() {
                                    self.remove_scan_target(index);
                                }
                            });
                        }
                    }
                });

                ui.group(|ui| {
                    ui.label("Exclude path:");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.scan_exclude_input);
                        if ui.button("Add exclude").clicked() {
                            self.add_scan_exclude();
                        }
                    });

                    if !self.scan_excludes.is_empty() {
                        let excludes = self.scan_excludes.clone();
                        for (index, exclude) in excludes.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(format!("• {}", exclude));
                                if ui.button("Remove").clicked() {
                                    self.remove_scan_exclude(index);
                                }
                            });
                        }
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Start scan").clicked() {
                        self.request_scan();
                    }
                    if ui.button("Cancel scan").clicked() {
                        self.request_cancel();
                    }
                });
            }


            if self.active_tab == Tab::Library {
                ui.separator();
                ui.heading("Favorite directories");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.favorite_directory_input);
                    if ui.button("Add favorite").clicked() {
                        self.add_favorite_directory();
                    }
                });


            if let Some(details) = self.track_details_dialog.clone() {
                let mut open = true;
                egui::Window::new("Track details")
                    .open(&mut open)
                    .resizable(true)
                    .show(ctx, |ui| {
                        ui.label(format!("Title: {}", details.title));
                        ui.label(format!("Artist: {}", details.artist));
                        ui.label(format!("Album: {}", details.album));
                        ui.separator();
                        ui.label(format!("File path: {}", details.path));
                    });
                if !open {
                    self.track_details_dialog = None;
                }
            }
                if self.favorite_directories.is_empty() {
                    ui.label("No favorite directories configured.");
                } else {
                    let mut remove_index = None;
                    for (index, favorite) in self.favorite_directories.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("• {}", favorite));
                            if ui.button("Remove").clicked() {
                                remove_index = Some(index);
                            }
                        });
                    }
                    if let Some(index) = remove_index {
                        self.remove_favorite_directory(index);
                    }
                }
            }
        });
    }
}
