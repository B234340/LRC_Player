use crate::core::models::{Track, TrackId};

#[derive(Debug, Default, Clone)]
pub struct LibraryIndex {
    folders: Vec<String>,
    tracks: Vec<Track>,
}

impl LibraryIndex {
    pub fn new(folders: Vec<String>) -> Self {
        Self {
            folders,
            tracks: Vec::new(),
        }
    }

    pub fn folders(&self) -> &[String] {
        &self.folders
    }

    pub fn add_folder(&mut self, folder: impl Into<String>) {
        self.folders.push(folder.into());
    }

    pub fn replace_tracks(&mut self, tracks: Vec<Track>) {
        self.tracks = tracks;
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn track_by_id(&self, track_id: &TrackId) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == *track_id)
    }
}