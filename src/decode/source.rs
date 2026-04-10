use std::path::PathBuf;

use crate::core::ids::TrackId;
use crate::core::library::Library;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedSource {
    pub track_id: TrackId,
    pub path: PathBuf,
}

impl DecodedSource {
    pub fn from_library(track_id: TrackId, library: &Library) -> Option<Self> {
        let path = library.track_path(track_id)?;
        Some(Self {
            track_id,
            path: PathBuf::from(path),
        })
    }
}