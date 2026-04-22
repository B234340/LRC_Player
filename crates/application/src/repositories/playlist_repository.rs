use crate::dto::PlaylistDto;
use crate::infrastructure::persistence::PlaylistsFile;
use anyhow::Result;

/// Application-level repository for playlist persistence.
pub trait PlaylistRepository {
    fn load_playlists(&self) -> Result<Vec<PlaylistDto>>;
    fn save_playlists(&self, playlists: &[PlaylistDto]) -> Result<()>;
}

impl PlaylistRepository for PlaylistsFile {
    fn load_playlists(&self) -> Result<Vec<PlaylistDto>> {
        self.load()
    }

    fn save_playlists(&self, playlists: &[PlaylistDto]) -> Result<()> {
        self.save(playlists)
    }
}
