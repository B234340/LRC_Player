use crate::domain::ids::TrackId;
use crate::domain::model::track::Track;

#[derive(Debug, Default, Clone)]
pub struct Library {
    pub tracks: Vec<Track>,
}

impl Library {
    pub fn new() -> Self {
        Self { tracks: Vec::new() }
    }

    pub fn add_track(&mut self, track: Track) {
        if self.tracks.iter().any(|existing| existing.id == track.id) {
            return;
        }
        self.tracks.push(track);
    }

    pub fn find_track(&self, track_id: &TrackId) -> Option<&Track> {
        self.tracks.iter().find(|track| &track.id == track_id)
    }

    pub fn remove_track(&mut self, track_id: &TrackId) -> bool {
        if let Some(pos) = self.tracks.iter().position(|track| &track.id == track_id) {
            self.tracks.remove(pos);
            true
        } else {
            false
        }
    }
}


