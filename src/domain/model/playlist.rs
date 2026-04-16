use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub track_ids: Vec<String>,
}

impl Playlist {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            track_ids: Vec::new(),
        }
    }

    /// Add a track to the playlist if it does not already exist.
    pub fn add_track(&mut self, track_id: impl Into<String>) {
        let track_id = track_id.into();
        if !self.track_ids.contains(&track_id) {
            self.track_ids.push(track_id);
        }
    }
}
