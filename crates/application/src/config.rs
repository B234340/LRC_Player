use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Ja,
    Zh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtistViewMode {
    Artist,
    AlbumArtist,
    Composer,
}

impl ArtistViewMode {
    pub fn label(self) -> &'static str {
        match self {
            ArtistViewMode::Artist => "Artist",
            ArtistViewMode::AlbumArtist => "Album Artist",
            ArtistViewMode::Composer => "Composer",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            ArtistViewMode::Artist => "artist",
            ArtistViewMode::AlbumArtist => "album_artist",
            ArtistViewMode::Composer => "composer",
        }
    }
}

impl Default for ArtistViewMode {
    fn default() -> Self {
        ArtistViewMode::Artist
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    #[default]
    Info,
    Error,
    Warn,
    Debug,
    Trace,
}

impl LogLevel {
    /// Filter directive understood by `tracing_subscriber::EnvFilter` / `LevelFilter`.
    pub fn as_filter_str(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

/// Resampler used by the audio engine when the file rate differs from the device rate.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ResamplerSetting {
    /// Synchronous FFT resampler (best quality / CPU for fixed rate pairs).
    #[default]
    Fft,
    /// Windowed-sinc resampler whose filter length is the "resampler taps" setting.
    Sinc,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeviceSettings {
    pub resampler_taps: usize,
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self { resampler_taps: 32 }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub resampler_taps: usize,
    pub preferred_output_device: Option<String>,
    #[serde(default)]
    pub device_settings: HashMap<String, DeviceSettings>,
    #[serde(default)]
    pub language: Language,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub log_level: LogLevel,
    /// Engine resampler (applied when the engine starts).
    #[serde(default)]
    pub resampler: ResamplerSetting,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            resampler_taps: 32,
            preferred_output_device: None,
            device_settings: HashMap::new(),
            language: Language::default(),
            theme: Theme::default(),
            log_level: LogLevel::default(),
            resampler: ResamplerSetting::default(),
        }
    }
}

impl Config {
    pub fn taps_for_device(&self, device: Option<&String>) -> usize {
        if let Some(device_name) = device {
            if let Some(settings) = self.device_settings.get(device_name) {
                return settings.resampler_taps;
            }
        }
        self.resampler_taps
    }

    pub fn set_taps_for_device(&mut self, device: String, taps: usize) {
        self.device_settings
            .entry(device)
            .or_default()
            .resampler_taps = taps;
    }

    fn path() -> Option<PathBuf> {
        let exe_path = std::env::current_exe().ok()?;
        let exe_dir = exe_path.parent()?;
        let data_dir = exe_dir.join("data");
        std::fs::create_dir_all(&data_dir).ok()?;
        Some(data_dir.join("config.json"))
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


