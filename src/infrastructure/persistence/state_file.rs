use crate::application::dto::StateDto;
use crate::infrastructure::persistence::json_store::JsonStore;
use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::path::{Path, PathBuf};

pub struct StateFile {
    store: JsonStore,
}

impl StateFile {
    fn default_path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("com", "example", "lrc_player")
            .context("unable to determine project directories")?;
        Ok(dirs.data_dir().join("state.json"))
    }

    pub fn open() -> Result<Self> {
        let path = Self::default_path()?;
        Self::open_with_path(path)
    }

    pub fn open_with_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let store = JsonStore::open(path)?;
        Ok(StateFile { store })
    }

    pub fn load(&self) -> Result<StateDto> {
        let state = self
            .store
            .load()
            .with_context(|| format!("loading state file from {}", self.store.path().display()))?;
        Ok(state)
    }

    pub fn save(&self, state: &StateDto) -> Result<()> {
        self.store.save(state)
    }
}


