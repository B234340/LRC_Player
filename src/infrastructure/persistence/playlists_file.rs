use crate::application::dto::PlaylistDto;
use crate::infrastructure::persistence::json_store::JsonStore;
use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::path::{Path, PathBuf};

pub struct PlaylistsFile {
    store: JsonStore,
}

impl PlaylistsFile {
    fn default_path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("com", "example", "lrc_player")
            .context("unable to determine project directories")?;
        Ok(dirs.data_dir().join("playlists.json"))
    }

    pub fn open() -> Result<Self> {
        let path = Self::default_path()?;
        Self::open_with_path(path)
    }

    pub fn open_with_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let store = JsonStore::open(path)?;
        Ok(PlaylistsFile { store })
    }

    pub fn load(&self) -> Result<Vec<PlaylistDto>> {
        let playlists = self
            .store
            .load()
            .with_context(|| format!("loading playlists from {}", self.store.path().display()))?;
        Ok(playlists)
    }

    pub fn save(&self, playlists: &[PlaylistDto]) -> Result<()> {
        self.store.save(playlists)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn playlists_file_roundtrips_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("playlists.json");
        let file = PlaylistsFile::open_with_path(path).unwrap();

        let original = vec![PlaylistDto {
            id: "playlist-1".into(),
            name: "Favorites".into(),
            tracks: Vec::new(),
        }];
        file.save(&original).unwrap();

        let loaded = file.load().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, original[0].id);
        assert_eq!(loaded[0].name, original[0].name);
    }
}
