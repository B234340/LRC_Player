use crate::app::view_models::TrackRowVm;
use crate::core::queue::RepeatMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueTab {
    pub rows: Vec<TrackRowVm>,
    pub repeat_mode: RepeatMode,
}