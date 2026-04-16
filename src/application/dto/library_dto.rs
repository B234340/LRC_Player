use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryDto {
    pub album_count: usize,
    pub track_count: usize,
}
