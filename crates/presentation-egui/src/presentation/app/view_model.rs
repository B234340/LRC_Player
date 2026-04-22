//! ViewModelの構築を担当
//! - AppVmの生成
//! - LibraryDtoの構築

use lrc_player_application::config::Config;
use lrc_player_application::dto::{LibraryDto, PlaylistDto, SettingsDto, StateDto};
use lrc_player_application::infrastructure::persistence::MetaCache;
use lrc_player_application::usecases::build_app_vm::build_app_vm;
use lrc_player_application::view_model::AppVm;
use lrc_player_domain::model::PlaybackMode;

use crate::presentation::app::{GuiApp, Tab};

impl GuiApp {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn default_app_vm(
        active_tab: Tab,
        config: &Config,
        last_scan_status: Option<String>,
        scan_targets: Vec<String>,
        scan_lrc_targets: Vec<String>,
        scan_excludes: Vec<String>,
        library: LibraryDto,
        playlists: &[PlaylistDto],
        queue: lrc_player_application::view_model::QueueVm,
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
            scan_lrc_targets,
            scan_excludes,
            queue_tracks: Vec::new(),
            selected_queue_index: None,
            queue_playback_mode: PlaybackMode::Sequential,
            artist_view_mode: crate::presentation::app::ArtistViewMode::Artist,
            lrc_overlay_enabled: false,
            side_panel_expanded: true,
            favorite_directories: Vec::new(),
        };

        build_app_vm("LRC Player", &settings, &state, playlists, &library, queue)
    }

    pub(crate) fn library_dto(&self) -> LibraryDto {
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
                if let Ok(tracks) =
                    cache.library_track_locations_by_artist_kind(self.artist_view_mode.key(), artist)
                {
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
            self.scan_lrc_targets.clone(),
            self.scan_excludes.clone(),
            library,
            &self.playlists,
            self.queue.clone(),
        );
        self.sync_album_artwork_cache_with_library();
        self.save_state();
    }

    pub(crate) fn switch_to_play_tab(&mut self) {
        if self.active_tab != Tab::Play {
            self.active_tab = Tab::Play;
            self.update_app_vm();
        }
    }
}