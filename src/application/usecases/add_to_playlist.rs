use crate::domain::model::Playlist;
use crate::domain::services::PlaylistService;

/// Add a track to the given playlist through application business logic.
pub fn execute(service: &PlaylistService, playlist: &mut Playlist, track_id: &str) {
    service.add_track(playlist, track_id);
}


