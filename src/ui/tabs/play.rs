use crate::app::view_models::TrackRowVm;
use crate::core::playback::PlaybackState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayTab {
    pub current_track: Option<TrackRowVm>,
    pub playback_state: PlaybackState,
    pub position_ms: u64,
}