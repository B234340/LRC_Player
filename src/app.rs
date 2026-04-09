mod layout;
mod tabs;
mod types;
mod utils;

use crate::core::config::ConfigStore;
use crate::core::models::TrackId;
use crate::core::state::AppState;
use eframe::egui;
use std::collections::HashMap;

use self::types::{AlbumView, AppTab, ArtistKind, ArtistView};
use self::utils::configure_fonts;

pub struct LrcPlayerApp {
    config_store: ConfigStore,
    state: AppState,
    include_path_input: String,
    exclude_path_input: String,
    playlist_name_input: String,
    active_tab: AppTab,
    artist_kind: ArtistKind,
    selected_album_key: Option<String>,
    selected_artist_name: Option<String>,
    revealed_queue_anchor_id: Option<TrackId>,
    queue_follow_paused_by_user: bool,
    last_queue_current_track_id: Option<TrackId>,
    detail_track_id: Option<TrackId>,
    artwork_cache: HashMap<String, egui::TextureHandle>,
    cached_library_revision: u64,
    cached_albums: Vec<AlbumView>,
    cached_artist_revision: u64,
    cached_artist_kind: ArtistKind,
    cached_artists: Vec<ArtistView>,
}

impl LrcPlayerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_fonts(&cc.egui_ctx);

        let config_store = ConfigStore::new();
        let state = AppState::bootstrap(&config_store);

        Self {
            config_store,
            state,
            include_path_input: String::new(),
            exclude_path_input: String::new(),
            playlist_name_input: String::new(),
            active_tab: AppTab::Play,
            artist_kind: ArtistKind::Artist,
            selected_album_key: None,
            selected_artist_name: None,
            revealed_queue_anchor_id: None,
            queue_follow_paused_by_user: false,
            last_queue_current_track_id: None,
            detail_track_id: None,
            artwork_cache: HashMap::new(),
            cached_library_revision: 0,
            cached_albums: Vec::new(),
            cached_artist_revision: 0,
            cached_artist_kind: ArtistKind::Artist,
            cached_artists: Vec::new(),
        }
    }
}