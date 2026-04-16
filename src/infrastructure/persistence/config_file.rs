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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn config_file_roundtrips_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let file = ConfigFile::open_with_path(path).unwrap();

        let original = Config {
            resampler_taps: 64,
            preferred_output_device: Some("usb_audio".into()),
            language: crate::config::Language::En,
        };
        file.save(&original).unwrap();

        let loaded = file.load().unwrap();
        assert_eq!(loaded.resampler_taps, original.resampler_taps);
        assert_eq!(
            loaded.preferred_output_device,
            original.preferred_output_device
        );
    }
}
