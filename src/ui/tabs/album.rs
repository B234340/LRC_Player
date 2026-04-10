use crate::app::view_models::TrackRowVm;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumTab {
    pub album_count: usize,
    pub visible_tracks: Vec<TrackRowVm>,
}