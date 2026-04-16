use crate::config::Config;
use crate::infrastructure::persistence::config_file::ConfigFile;
use anyhow::Result;

/// Application-level repository for settings persistence.
pub trait SettingsRepository {
    fn load_settings(&self) -> Result<Config>;
    fn save_settings(&self, config: &Config) -> Result<()>;
}

impl SettingsRepository for ConfigFile {
    fn load_settings(&self) -> Result<Config> {
        self.load()
    }

    fn save_settings(&self, config: &Config) -> Result<()> {
        self.save(config)
    }
}
