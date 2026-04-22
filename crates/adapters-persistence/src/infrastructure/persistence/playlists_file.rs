use crate::infrastructure::persistence::json_store::JsonStore;
use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

pub struct PlaylistsFile<T> {
    store: JsonStore,
    _marker: PhantomData<T>,
}

impl<T> PlaylistsFile<T>
where
    T: Serialize + DeserializeOwned,
{
    fn default_path() -> Result<PathBuf> {
        let exe_path = std::env::current_exe().context("unable to determine current executable path")?;
        let exe_dir = exe_path
            .parent()
            .context("current executable has no parent directory")?;
        let data_dir = exe_dir.join("data");
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("creating data directory {}", data_dir.display()))?;
        Ok(data_dir.join("playlists.json"))
    }

    pub fn open() -> Result<Self> {
        let path = Self::default_path()?;
        Self::open_with_path(path)
    }

    pub fn open_with_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let store = JsonStore::open(path)?;
        Ok(PlaylistsFile { store, _marker: PhantomData })
    }

    pub fn load(&self) -> Result<Vec<T>> {
        let playlists = self
            .store
            .load()
            .with_context(|| format!("loading playlists from {}", self.store.path().display()))?;
        Ok(playlists)
    }

    pub fn save(&self, playlists: &[T]) -> Result<()> {
        self.store.save(playlists)
    }
}


