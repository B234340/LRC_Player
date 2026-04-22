pub use lrc_player_shared::dto::LibraryDirectoryDto;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryDto {
    pub album_count: usize,
    pub track_count: usize,
    pub recent_locations: Vec<String>,
    pub recent_directories: Vec<String>,
    pub directory_tree: Vec<LibraryDirectoryDto>,
    pub favorite_directories: Vec<String>,
    pub albums: Vec<String>,
    pub selected_album: Option<String>,
    pub selected_album_tracks: Vec<String>,
    pub artists: Vec<String>,
    pub selected_artist: Option<String>,
    pub selected_artist_tracks: Vec<String>,
    pub selected_directory: Option<String>,
    pub selected_directory_tracks: Vec<String>,
}
