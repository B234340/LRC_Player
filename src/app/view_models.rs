//! UI が消費する ViewModel 型群
//!
//! これらの型は UI 描画用に軽量化された DTO で、`AppFacade::view_model()` が組み立てて UI に渡します。
use crate::core::artwork::ArtworkHandle;
use crate::core::ids::{AlbumId, ArtistId, PlaylistId, TrackId};
use crate::core::library::{ArtistKind, DirectoryNodeId};
use crate::core::persistence::TabId;
use crate::core::playback::PlaybackState;
use crate::core::queue::RepeatMode;
use crate::core::scan::ScanState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FavoriteStateVm {
    Included,
    Excluded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackRowVm {
    pub track_id: TrackId,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artwork: Option<ArtworkHandle>,
    pub favorite_state: FavoriteStateVm,
    pub is_current: bool,
    pub is_selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueTabVm {
    pub rows: Vec<TrackRowVm>,
    pub repeat_mode: RepeatMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayTabVm {
    pub current_track: Option<TrackRowVm>,
    pub playback_state: PlaybackState,
    pub position_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanTabVm {
    pub include_dirs: Vec<String>,
    pub exclude_dirs: Vec<String>,
    pub visible_status: ScanState,
    pub cancel_visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumEntryVm {
    pub album_id: AlbumId,
    pub title: String,
    pub album_artist: String,
    pub track_count: usize,
    pub is_provisional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeferredListStateVm {
    pub total_entries: usize,
    pub hydrated_entries: usize,
    pub is_loading: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumTabVm {
    pub albums: Vec<AlbumEntryVm>,
    pub expanded_album: Option<AlbumId>,
    pub visible_tracks: Vec<TrackRowVm>,
    pub loading: DeferredListStateVm,
    pub tracks_loading: bool,
    pub tracks_loaded: usize,
    pub tracks_total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistEntryVm {
    pub artist_id: ArtistId,
    pub name: String,
    pub track_count: usize,
    pub is_provisional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistTabVm {
    pub kind: ArtistKind,
    pub artists: Vec<ArtistEntryVm>,
    pub selected_artist: Option<ArtistId>,
    pub visible_tracks: Vec<TrackRowVm>,
    pub loading: DeferredListStateVm,
    pub tracks_loading: bool,
    pub tracks_loaded: usize,
    pub tracks_total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistEntryVm {
    pub playlist_id: PlaylistId,
    pub name: String,
    pub track_count: usize,
    pub is_system: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistTabVm {
    pub playlists: Vec<PlaylistEntryVm>,
    pub selected_playlist: Option<PlaylistId>,
    pub visible_tracks: Vec<TrackRowVm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntryVm {
    pub node_id: DirectoryNodeId,
    pub name: String,
    pub child_count: usize,
    pub track_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryTabVm {
    pub nodes: Vec<DirectoryEntryVm>,
    pub selected_directory: Option<DirectoryNodeId>,
    pub visible_tracks: Vec<TrackRowVm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingItemVm {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsTabVm {
    pub items: Vec<SettingItemVm>,
    pub available_output_devices: Vec<String>,
    pub preferred_output_device: Option<String>,
    pub debug_mode: bool,
    pub available_log_categories: Vec<String>,
    pub selected_log_filters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppViewModel {
    pub current_tab: TabId,
    pub queue: QueueTabVm,
    pub play: PlayTabVm,
    pub album: AlbumTabVm,
    pub artist: ArtistTabVm,
    pub playlist: PlaylistTabVm,
    pub library: LibraryTabVm,
    pub scan: ScanTabVm,
    pub settings: SettingsTabVm,
}
