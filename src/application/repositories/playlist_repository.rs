use crate::infrastructure::persistence::playlists_file::PlaylistsFile;
use anyhow::Result;

/// Application-level repository for playlist persistence.
pub trait PlaylistRepository {
    fn load_playlists(&self) -> Result<()>;
    fn save_playlists(&self) -> Result<()>;
}

impl PlaylistRepository for PlaylistsFile {
    fn load_playlists(&self) -> Result<()> {
        PlaylistsFile::load()
    }

    fn save_playlists(&self) -> Result<()> {
        PlaylistsFile::save()
    }
}
