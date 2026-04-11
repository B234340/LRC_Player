use crate::app::view_models::TrackRowVm;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryTab {
    pub root_node_count: usize,
    pub visible_tracks: Vec<TrackRowVm>,
}
