use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub resampler_taps: usize,
    pub preferred_output_device: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            resampler_taps: 32,
            preferred_output_device: None,
        }
    }
}

impl Config {
    fn path() -> Option<PathBuf> {
        ProjectDirs::from("com", "example", "lrc_player")
            .map(|pd| pd.config_dir().join("config.json"))
    }

    pub fn load() -> Self {
        if let Some(path) = Self::path() {
            if let Ok(s) = fs::read_to_string(&path) {
                if let Ok(cfg) = serde_json::from_str(&s) {
                    return cfg;
                }
            }
        }
        Default::default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        if let Some(path) = Self::path() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let tmp = path.with_extension("tmp");
            let mut f = fs::File::create(&tmp)?;
            let s = serde_json::to_string_pretty(self)?;
            f.write_all(s.as_bytes())?;
            f.sync_all()?;
            fs::rename(tmp, &path)?;
        }
        Ok(())
    }
}
