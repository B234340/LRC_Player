#[cfg(test)]
mod tests {
    use lrc_player::domain::model::{Library, Track};
    use lrc_player::domain::services::library_service::*;
    use lrc_player::domain::ids::TrackId;

    #[test]
    fn add_and_find_track() {
        let service = LibraryService::new();
        let mut library = Library::new();
        let track = Track {
            id: TrackId("track-1".into()),
            title: "Test Track".into(),
        };

        service.add_track(&mut library, track.clone());

        let found = service.find_track(&library, &track.id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "Test Track");
    }

    #[test]
    fn remove_track_returns_true() {
        let service = LibraryService::new();
        let mut library = Library::new();
        let track = Track {
            id: TrackId("track-2".into()),
            title: "Remove Me".into(),
        };

        service.add_track(&mut library, track.clone());
        let removed = service.remove_track(&mut library, &track.id);

        assert!(removed);
        assert!(service.find_track(&library, &track.id).is_none());
    }
}
