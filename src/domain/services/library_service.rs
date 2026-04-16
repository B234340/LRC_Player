use crate::domain::ids::TrackId;
use crate::domain::model::{Library, Track};

/// Pure library business logic.
pub struct LibraryService;

impl LibraryService {
    pub fn new() -> Self {
        LibraryService
    }

    pub fn add_track(&self, library: &mut Library, track: Track) {
        library.add_track(track);
    }

    pub fn find_track<'a>(&self, library: &'a Library, track_id: &TrackId) -> Option<&'a Track> {
        library.find_track(track_id)
    }

    pub fn remove_track(&self, library: &mut Library, track_id: &TrackId) -> bool {
        library.remove_track(track_id)
    }
}

impl Default for LibraryService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::TrackId;
    use crate::domain::model::track::Track;

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
