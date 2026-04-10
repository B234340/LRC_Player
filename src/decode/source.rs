use std::path::PathBuf;

use crate::core::ids::TrackId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedSource {
    pub track_id: TrackId,
    pub path: PathBuf,
}