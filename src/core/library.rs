use crate::core::models::{Track, TrackId};

#[derive(Debug, Default, Clone)]
pub struct LibraryIndex {
    tracks: Vec<Track>,
}

impl LibraryIndex {
    pub fn new() -> Self {
        Self { tracks: Vec::new() }
    }

    pub fn replace_tracks(&mut self, tracks: Vec<Track>) {
        self.tracks = tracks;
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn track_by_id(&self, track_id: &TrackId) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == *track_id)
    }
}