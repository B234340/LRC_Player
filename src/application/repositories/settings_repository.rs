use crate::infrastructure::persistence::config_file::ConfigFile;
use anyhow::Result;

/// Application-level repository for settings persistence.
pub trait SettingsRepository {
    fn load_settings(&self) -> Result<()>;
    fn save_settings(&self) -> Result<()>;
}

impl SettingsRepository for ConfigFile {
    fn load_settings(&self) -> Result<()> {
        ConfigFile::load()
    }

    fn save_settings(&self) -> Result<()> {
        ConfigFile::save()
    }
}
