use crate::domain::model::Playlist;
use crate::domain::services::PlaylistService;

/// Add a track to the given playlist through application business logic.
pub fn execute(service: &PlaylistService, playlist: &mut Playlist, track_id: &str) {
    service.add_track(playlist, track_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_track_to_playlist() {
        let service = PlaylistService::new();
        let mut playlist = Playlist::new("p1", "Favorites");

        execute(&service, &mut playlist, "track-1");

        assert_eq!(playlist.track_ids.len(), 1);
        assert_eq!(playlist.track_ids[0], "track-1");
    }

    #[test]
    fn does_not_duplicate_track() {
        let service = PlaylistService::new();
        let mut playlist = Playlist::new("p1", "Favorites");

        execute(&service, &mut playlist, "track-1");
        execute(&service, &mut playlist, "track-1");

        assert_eq!(playlist.track_ids, vec!["track-1"]);
    }
}
