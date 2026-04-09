use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::PathBuf;

const DATA_DIR_NAME: &str = "data";
const CONFIG_FILE_NAME: &str = "config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub theme: String,
    pub library_folders: Vec<String>,
    pub last_playlist: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: String::from("system"),
            library_folders: Vec::new(),
            last_playlist: None,
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(serde_json::Error),
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Parse(err) => write!(f, "JSON error: {err}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for ConfigError {
    fn from(value: serde_json::Error) -> Self {
        Self::Parse(value)
    }
}

pub struct ConfigStore {
    data_dir: PathBuf,
    config_path: PathBuf,
}

impl ConfigStore {
    pub fn new() -> Self {
        let base_dir = portable_base_dir();
        let data_dir = base_dir.join(DATA_DIR_NAME);
        let config_path = data_dir.join(CONFIG_FILE_NAME);

        Self {
            data_dir,
            config_path,
        }
    }

    pub fn load_or_create(&self) -> Result<AppConfig, ConfigError> {
        self.ensure_layout()?;

        if !self.config_path.exists() {
            let default_config = AppConfig::default();
            self.save(&default_config)?;
            return Ok(default_config);
        }

        let raw = fs::read_to_string(&self.config_path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), ConfigError> {
        self.ensure_layout()?;
        let json = serde_json::to_string_pretty(config)?;
        fs::write(&self.config_path, json)?;
        Ok(())
    }

    fn ensure_layout(&self) -> Result<(), ConfigError> {
        fs::create_dir_all(self.data_dir.join("cache"))?;
        fs::create_dir_all(self.data_dir.join("logs"))?;
        Ok(())
    }
}

fn portable_base_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}