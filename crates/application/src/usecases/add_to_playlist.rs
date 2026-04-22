use lrc_player_domain::model::Playlist;
use lrc_player_domain::services::PlaylistService;

/// Add a track to the given playlist through application business logic.
pub fn execute(service: &PlaylistService, playlist: &mut Playlist, track_id: &str) {
    service.add_track(playlist, track_id);
}


