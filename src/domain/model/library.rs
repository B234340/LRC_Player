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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::TrackId;
    use crate::domain::model::track::Track;

    #[test]
    fn library_add_dedupes_identical_track_ids() {
        let mut library = Library::new();
        let track = Track {
            id: TrackId("track-1".into()),
            title: "First".into(),
        };

        library.add_track(track.clone());
        library.add_track(track);

        assert_eq!(library.tracks.len(), 1);
    }
}
