#[cfg(test)]
mod tests {
    use lrc_player::domain::model::Playlist;
    use lrc_player::domain::services::PlaylistService;
    use lrc_player::application::usecases::add_to_playlist::*;

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
