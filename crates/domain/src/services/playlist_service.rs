use crate::ids::TrackId;
use crate::model::{Library, Playlist, Track};

#[derive(Default)]
pub struct PlaylistService;

impl PlaylistService {
    pub fn new() -> Self {
        Self
    }

    pub fn create_playlist(&self, _name: &str) {
        // skeleton: implement playlist creation business logic
    }

    pub fn add_track(&self, playlist: &mut Playlist, track_id: &str) {
        playlist.add_track(track_id);
    }

    pub fn add_track_to_library(&self, library: &mut Library, track: Track) {
        library.add_track(track);
    }

    pub fn remove_track_from_library(&self, library: &mut Library, track_id: &TrackId) -> bool {
        library.remove_track(track_id)
    }
}
