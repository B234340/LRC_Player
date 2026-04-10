use crate::core::artwork::ArtworkHandle;
use crate::core::ids::TrackId;
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
pub struct AppViewModel {
    pub current_tab: TabId,
    pub queue: QueueTabVm,
    pub play: PlayTabVm,
    pub scan: ScanTabVm,
}