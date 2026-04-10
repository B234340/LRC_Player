use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::ids::TrackKey;
use crate::core::queue::RepeatMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TabId {
    #[default]
    Queue,
    Play,
    Album,
    Artist,
    Playlist,
    Library,
    Scan,
    Settings,
}

#[derive(Debug, Clone)]
pub struct PersistencePaths {
    pub config_json: PathBuf,
    pub state_json: PathBuf,
    pub playlists_json: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfigFile {
    pub accent_color: String,
    pub theme: String,
    pub seek_step_ms: u64,
    pub restore_position_policy: String,
    pub full_auto_scan: bool,
    pub fallback_artwork_path: Option<PathBuf>,
    pub play_empty_state: String,
    pub include_dirs: Vec<PathBuf>,
    pub exclude_dirs: Vec<PathBuf>,
}

impl Default for AppConfigFile {
    fn default() -> Self {
        Self {
            accent_color: "#2C6E49".to_string(),
            theme: "system".to_string(),
            seek_step_ms: 10_000,
            restore_position_policy: "resume".to_string(),
            full_auto_scan: false,
            fallback_artwork_path: None,
            play_empty_state: "none".to_string(),
            include_dirs: Vec::new(),
            exclude_dirs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateFile {
    pub last_tab: TabId,
    pub queue: Vec<TrackKey>,
    pub current_index: Option<u32>,
    pub repeat_mode: RepeatMode,
    pub playback_position_ms: u64,
}

impl Default for AppStateFile {
    fn default() -> Self {
        Self {
            last_tab: TabId::Queue,
            queue: Vec::new(),
            current_index: None,
            repeat_mode: RepeatMode::RepeatQueue,
            playback_position_ms: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaylistFile {
    pub id: u32,
    pub name: String,
    pub kind: String,
    pub tracks: Vec<TrackKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaylistsFile {
    pub playlists: Vec<PlaylistFile>,
}

#[derive(Debug, Clone)]
pub struct JsonStore {
    pub paths: PersistencePaths,
}

impl JsonStore {
    pub fn new(paths: PersistencePaths) -> Self {
        Self { paths }
    }

    pub fn load_config(&self) -> io::Result<AppConfigFile> {
        self.load_json(&self.paths.config_json)
    }

    pub fn save_config(&self, config: &AppConfigFile) -> io::Result<()> {
        self.save_json(&self.paths.config_json, config)
    }

    pub fn load_state(&self) -> io::Result<AppStateFile> {
        self.load_json(&self.paths.state_json)
    }

    pub fn save_state(&self, state: &AppStateFile) -> io::Result<()> {
        self.save_json(&self.paths.state_json, state)
    }

    pub fn load_playlists(&self) -> io::Result<PlaylistsFile> {
        self.load_json(&self.paths.playlists_json)
    }

    pub fn save_playlists(&self, playlists: &PlaylistsFile) -> io::Result<()> {
        self.save_json(&self.paths.playlists_json, playlists)
    }

    fn load_json<T>(&self, path: &Path) -> io::Result<T>
    where
        T: for<'de> Deserialize<'de> + Default,
    {
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
            Err(error) => Err(error),
        }
    }

    fn save_json<T>(&self, path: &Path, value: &T) -> io::Result<()>
    where
        T: Serialize,
    {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let tmp_path = path.with_extension("tmp");
        let contents = serde_json::to_vec_pretty(value)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::write(&tmp_path, contents)?;
        fs::rename(tmp_path, path)?;
        Ok(())
    }
}