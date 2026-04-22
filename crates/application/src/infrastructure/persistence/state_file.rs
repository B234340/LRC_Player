use crate::dto::StateDto;
use crate::infrastructure::persistence::json_store::JsonStore;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct StateFile {
    store: JsonStore,
}

impl StateFile {
    fn default_path() -> Result<PathBuf> {
        let exe_path = std::env::current_exe().context("unable to determine current executable path")?;
        let exe_dir = exe_path
            .parent()
            .context("current executable has no parent directory")?;
        let data_dir = exe_dir.join("data");
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("creating data directory {}", data_dir.display()))?;
        Ok(data_dir.join("state.json"))
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


