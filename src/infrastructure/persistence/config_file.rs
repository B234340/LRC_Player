use crate::config::Config;
use crate::infrastructure::persistence::json_store::JsonStore;
use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::path::{Path, PathBuf};

pub struct ConfigFile {
    store: JsonStore,
}

impl ConfigFile {
    fn default_path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("com", "example", "lrc_player")
            .context("unable to determine project directories")?;
        Ok(dirs.config_dir().join("config.json"))
    }

    pub fn open() -> Result<Self> {
        let path = Self::default_path()?;
        Self::open_with_path(path)
    }

    pub fn open_with_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let store = JsonStore::open(path)?;
        Ok(ConfigFile { store })
    }

    pub fn load(&self) -> Result<Config> {
        let cfg = self
            .store
            .load()
            .with_context(|| format!("loading config from {}", self.store.path().display()))?;
        Ok(cfg)
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        self.store.save(config)
    }
}


