//! アプリケーション状態の永続化と基本操作を担当

use std::collections::{HashMap, HashSet, VecDeque};

use cpal::traits::{DeviceTrait, HostTrait};
use eframe::CreationContext;
use lrc_player_application::config::Config;
use lrc_player_application::dto::{LibraryDto, PlaylistDto, StateDto};
use lrc_player_application::infrastructure::audio;
use lrc_player_application::infrastructure::persistence::{MetaCache, PlaylistsFile, StateFile};
use lrc_player_application::view_model::QueueVm;
use lrc_player_domain::model::{PlaybackMode, PlaybackState};

use crate::presentation::actions::{library, play, playlist, queue, scan};
use crate::presentation::app::{ArtistViewMode, GuiApp, Tab};

impl GuiApp {
    pub(crate) fn load_state() -> StateDto {
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
                scan_lrc_targets: Vec::new(),
                scan_excludes: Vec::new(),
                queue_tracks: Vec::new(),
                selected_queue_index: None,
                queue_playback_mode: PlaybackMode::Sequential,
                artist_view_mode: ArtistViewMode::Artist,
                lrc_overlay_enabled: false,
                side_panel_expanded: true,
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
            scan_lrc_targets: self.scan_lrc_targets.clone(),
            scan_excludes: self.scan_excludes.clone(),
            queue_tracks: self.queue.tracks.clone(),
            selected_queue_index: self.selected_queue_index,
            queue_playback_mode: self.playback_mode,
            artist_view_mode: self.artist_view_mode,
            lrc_overlay_enabled: self.lrc_overlay_enabled,
            side_panel_expanded: self.side_panel_expanded,
            favorite_directories: self.favorite_directories.clone(),
        };

        if let Ok(store) = StateFile::open() {
            let _ = store.save(&state);
        }
    }

    pub(crate) fn refresh_scan_status(&mut self) {
        if let Ok(store) = StateFile::open() {
            if let Ok(state) = store.load() {
                self.last_scan_status = state.last_scan_status.clone();
                self.app_vm.scan.last_scan_status = state.last_scan_status.clone();
            }
        }
    }

    pub(crate) fn load_playlists() -> Vec<PlaylistDto> {
        if let Ok(file) = PlaylistsFile::open() {
            let playlists = file.load().unwrap_or_default();
            Self::normalize_playlists(playlists)
        } else {
            Self::normalize_playlists(Vec::new())
        }
    }

    pub(crate) fn normalize_playlists(playlists: Vec<PlaylistDto>) -> Vec<PlaylistDto> {
        playlist::normalize_playlists(playlists)
    }

    pub(crate) fn save_playlists(&self) {
        playlist::save_playlists(self);
    }

    pub(crate) fn add_playlist(&mut self) {
        playlist::add_playlist(self);
    }

    pub(crate) fn remove_playlist(&mut self, index: usize) {
        playlist::remove_playlist(self, index);
    }

    pub(crate) fn move_playlist_up(&mut self, index: usize) {
        playlist::move_playlist_up(self, index);
    }

    pub(crate) fn move_playlist_down(&mut self, index: usize) {
        playlist::move_playlist_down(self, index);
    }

    pub(crate) fn add_queue_item(&mut self) {
        queue::add_queue_item(self);
    }

    pub(crate) fn clear_queue_selection(&mut self) {
        queue::clear_queue_selection(self);
    }

    pub(crate) fn clear_library(&mut self) {
        library::clear_library(self);
    }

    pub(crate) fn select_queue_item_single(&mut self, index: usize) {
        queue::select_queue_item_single(self, index);
    }

    pub(crate) fn toggle_queue_item_selection(&mut self, index: usize) {
        queue::toggle_queue_item_selection(self, index);
    }

    pub(crate) fn select_queue_item_range_to(&mut self, index: usize) {
        queue::select_queue_item_range_to(self, index);
    }

    pub(crate) fn remove_selected_queue_items(&mut self) {
        queue::remove_selected_queue_items(self);
    }

    pub(crate) fn clear_queue_handle_hold(&mut self) {
        queue::clear_queue_handle_hold(self);
    }

    pub(crate) fn cancel_queue_drag(&mut self) {
        queue::cancel_queue_drag(self);
    }

    pub(crate) fn move_queue_item_to(&mut self, from: usize, to: usize) {
        queue::move_queue_item_to(self, from, to);
    }

    pub(crate) fn remove_queue_item(&mut self, index: usize) {
        queue::remove_queue_item(self, index);
    }

    pub(crate) fn move_queue_item_up(&mut self, index: usize) {
        queue::move_queue_item_up(self, index);
    }

    pub(crate) fn move_queue_item_down(&mut self, index: usize) {
        queue::move_queue_item_down(self, index);
    }

    pub(crate) fn clear_queue(&mut self) {
        queue::clear_queue(self);
    }

    pub(crate) fn select_album(&mut self, album: Option<String>) {
        library::select_album(self, album);
    }

    pub(crate) fn select_artist(&mut self, artist: Option<String>) {
        library::select_artist(self, artist);
    }

    pub(crate) fn set_artist_view_mode(&mut self, mode: ArtistViewMode) {
        library::set_artist_view_mode(self, mode);
    }

    pub(crate) fn select_directory(&mut self, directory: Option<String>) {
        library::select_directory(self, directory);
    }

    pub(crate) fn toggle_library_directory_expanded(&mut self, path: &str) {
        library::toggle_library_directory_expanded(self, path);
    }

    pub(crate) fn is_library_directory_expanded(&self, path: &str) -> bool {
        library::is_library_directory_expanded(self, path)
    }

    pub(crate) fn add_favorite_directory(&mut self) {
        library::add_favorite_directory(self);
    }

    pub(crate) fn remove_favorite_directory(&mut self, index: usize) {
        library::remove_favorite_directory(self, index);
    }

    pub(crate) fn play_selected_queue_item(&mut self) {
        play::play_selected_queue_item(self);
    }

    pub(crate) fn seek_to_position(&mut self) {
        play::seek_to_position(self);
    }

    pub(crate) fn play_next(&mut self) {
        play::play_next(self);
    }

    pub(crate) fn play_previous(&mut self) {
        play::play_previous(self);
    }

    pub(crate) fn toggle_play_pause(&mut self) {
        play::toggle_play_pause(self);
    }

    pub(crate) fn request_scan(&mut self) {
        scan::request_scan(self);
    }

    pub(crate) fn request_cancel(&mut self) {
        scan::request_cancel(self);
    }

    pub(crate) fn add_scan_target(&mut self) {
        scan::add_scan_target(self);
    }

    pub(crate) fn remove_scan_target(&mut self, index: usize) {
        scan::remove_scan_target(self, index);
    }

    pub(crate) fn add_scan_lrc_target(&mut self) {
        scan::add_scan_lrc_target(self);
    }

    pub(crate) fn remove_scan_lrc_target(&mut self, index: usize) {
        scan::remove_scan_lrc_target(self, index);
    }

    pub(crate) fn add_scan_exclude(&mut self) {
        scan::add_scan_exclude(self);
    }

    pub(crate) fn remove_scan_exclude(&mut self, index: usize) {
        scan::remove_scan_exclude(self, index);
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
                state.scan_lrc_targets.clone(),
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
            active_output_device_name: None,
            last_seen_default_output_device_name: audio::cpal_output::default_device_name(),
            last_default_device_poll_at: None,
            playback_state_before_seek: None,
            pending_seek_ms: None,
            seek_last_change_at: None,
            seek_interacting: false,
            seek_long_press_start: None,
            seek_key_hold_direction: 0,
            seek_key_hold_started_at: None,
            seek_key_last_step_at: None,
            queue_scroll_to_playing: active_tab == Tab::Queue,
            artist_view_mode: state.artist_view_mode,
            side_panel_expanded: state.side_panel_expanded,
            scan_target_input: String::new(),
            scan_lrc_target_input: String::new(),
            scan_exclude_input: String::new(),
            scan_targets: state.scan_targets,
            scan_lrc_targets: state.scan_lrc_targets,
            scan_excludes: state.scan_excludes,
            selected_album: None,
            selected_artist: None,
            selected_directory: None,
            favorite_directories: state.favorite_directories,
            favorite_directory_input: String::new(),
            library_expanded_directories: HashSet::new(),
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
            play_artwork_blur_handle: None,
            play_artwork_blur_source: None,
            lrc_overlay_enabled: state.lrc_overlay_enabled,
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
