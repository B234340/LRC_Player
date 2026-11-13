use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// UI language. Japanese is the default (the primary users are Japanese).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    En,
    #[default]
    Ja,
    Zh,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtistViewMode {
    #[default]
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

/// Loudness normalization source (ReplayGain tags, or EBU R128 measurements for untagged
/// files).
///
/// Default `Track`: in a shuffled or mixed queue every track plays at the same loudness, which
/// is what ReplayGain-aware players (foobar2000, Plexamp, Spotify-style "normalize volume")
/// default to. `Album` keeps the level differences inside an album and needs album data
/// (tags, or every track of the album measured); it falls back to the track gain otherwise.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NormalizationModeSetting {
    Off,
    #[default]
    Track,
    Album,
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
    /// UI accent color key (`teal`, `blue`, `violet`, `rose`, `amber`, `green`).
    #[serde(default = "default_accent")]
    pub accent: String,
    /// Point size of the active lyric line.
    #[serde(default = "default_lyric_font_size")]
    pub lyric_font_size: f32,
    /// Karaoke (left-to-right) fill for lyrics with word/char timings.
    #[serde(default = "default_true")]
    pub karaoke: bool,
    /// Estimate a karaoke fill for line-based lyrics (spread over the line, weighted by
    /// syllables). Display only; never written to the `.lrc`.
    #[serde(default = "default_true")]
    pub estimate_karaoke: bool,
    /// Slim live spectrum strip on the Now Playing page.
    #[serde(default)]
    pub show_spectrum: bool,
    /// External command that adds per-character timing to an `.lrc` (e.g. `lrc-align`); the
    /// audio and `.lrc` paths are appended. Empty = feature off.
    #[serde(default)]
    pub align_command: String,
    /// Look up lyrics on LRCLIB (lrclib.net) for tracks without any (sends title, artist,
    /// album and duration). Off by default.
    #[serde(default)]
    pub fetch_lyrics_online: bool,
    /// Loudness normalization (`"off"` | `"track"` | `"album"`).
    #[serde(default)]
    pub normalization_mode: NormalizationModeSetting,
    /// Gain added on top of the normalization gain (dB). 0 = the ReplayGain 2.0 reference
    /// (-18 LUFS).
    #[serde(default)]
    pub normalization_preamp_db: f32,
    /// Limit the normalization gain so that the track (or album) peak stays at or below
    /// full scale.
    #[serde(default = "default_true")]
    pub prevent_clipping: bool,
}

fn default_accent() -> String {
    "teal".to_string()
}

/// Default size of the active lyric line (points).
pub const DEFAULT_LYRIC_FONT_SIZE: f32 = 30.0;

fn default_lyric_font_size() -> f32 {
    DEFAULT_LYRIC_FONT_SIZE
}

fn default_true() -> bool {
    true
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
            accent: default_accent(),
            lyric_font_size: DEFAULT_LYRIC_FONT_SIZE,
            karaoke: true,
            estimate_karaoke: true,
            show_spectrum: false,
            align_command: String::new(),
            fetch_lyrics_online: false,
            normalization_mode: NormalizationModeSetting::default(),
            normalization_preamp_db: 0.0,
            prevent_clipping: true,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_json_gets_ui_defaults() {
        let cfg: Config = serde_json::from_str(r#"{"resampler_taps": 64, "preferred_output_device": null}"#).unwrap();
        assert_eq!(cfg.language, Language::Ja);
        assert_eq!(cfg.accent, "teal");
        assert_eq!(cfg.lyric_font_size, DEFAULT_LYRIC_FONT_SIZE);
        assert!(cfg.karaoke);
        assert!(cfg.estimate_karaoke);
        assert!(!cfg.fetch_lyrics_online, "online lookups are opt-in");
        assert!(!cfg.show_spectrum);
        assert_eq!(cfg.normalization_mode, NormalizationModeSetting::Track);
        assert_eq!(cfg.normalization_preamp_db, 0.0);
        assert!(cfg.prevent_clipping);
        let back: Config = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.resampler_taps, 64);
    }

    #[test]
    fn normalization_settings_round_trip() {
        let cfg: Config = serde_json::from_str(
            r#"{"resampler_taps": 32, "preferred_output_device": null,
                "normalization_mode": "album", "normalization_preamp_db": -3.5, "prevent_clipping": false}"#,
        )
        .unwrap();
        assert_eq!(cfg.normalization_mode, NormalizationModeSetting::Album);
        assert_eq!(cfg.normalization_preamp_db, -3.5);
        assert!(!cfg.prevent_clipping);
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["normalization_mode"], "album");
        let back: Config = serde_json::from_value(json).unwrap();
        assert_eq!(back.normalization_mode, NormalizationModeSetting::Album);
        assert_eq!(back.normalization_preamp_db, -3.5);
        for (text, mode) in [("off", NormalizationModeSetting::Off), ("track", NormalizationModeSetting::Track)] {
            let v: NormalizationModeSetting = serde_json::from_str(&format!("\"{text}\"")).unwrap();
            assert_eq!(v, mode);
        }
        assert_eq!(Config::default().normalization_mode, NormalizationModeSetting::Track);
    }
}
