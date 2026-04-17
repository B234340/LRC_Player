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


