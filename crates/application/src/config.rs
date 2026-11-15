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

/// Headphone crossfeed strength, named by audible strength (`"weak"` | `"medium"` |
/// `"strong"`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CrossfeedLevelSetting {
    Weak,
    #[default]
    Medium,
    Strong,
}

/// Dither policy for integer output formats (`"auto"` | `"off"`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DitherSetting {
    /// TPDF dither on 16/24-bit integer output, only while the sound is processed.
    #[default]
    Auto,
    Off,
}

/// Parametric-EQ filter type (`"peak"` | `"low_shelf"` | `"high_shelf"` | `"low_pass"` |
/// `"high_pass"`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PeqFilterKindSetting {
    #[default]
    Peak,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
}

/// One parametric-EQ filter (`gain_db` is ignored by the pass filters).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct PeqFilterSetting {
    #[serde(default)]
    pub kind: PeqFilterKindSetting,
    #[serde(default = "default_peq_freq")]
    pub freq_hz: f32,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default = "default_peq_q")]
    pub q: f32,
}

impl Default for PeqFilterSetting {
    fn default() -> Self {
        Self {
            kind: PeqFilterKindSetting::Peak,
            freq_hz: default_peq_freq(),
            gain_db: 0.0,
            q: default_peq_q(),
        }
    }
}

fn default_peq_freq() -> f32 {
    1000.0
}

fn default_peq_q() -> f32 {
    std::f32::consts::FRAC_1_SQRT_2
}

/// Parametric EQ (in series after the 10-band EQ). Off and empty by default.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ParametricEqSetting {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub preamp_db: f32,
    #[serde(default)]
    pub filters: Vec<PeqFilterSetting>,
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
    /// Headphone crossfeed on/off (stereo output only).
    #[serde(default)]
    pub crossfeed_enabled: bool,
    #[serde(default)]
    pub crossfeed_level: CrossfeedLevelSetting,
    /// Lower the level by the EQs' largest boost so boosts do not clip.
    #[serde(default = "default_true")]
    pub auto_headroom: bool,
    #[serde(default)]
    pub dither: DitherSetting,
    /// WASAPI exclusive output (Windows only; ignored elsewhere).
    #[serde(default)]
    pub exclusive_mode: bool,
    #[serde(default)]
    pub parametric_eq: ParametricEqSetting,
    /// Desktop lyrics: a small always-on-top window with the current line.
    #[serde(default)]
    pub desktop_lyrics: DesktopLyricsSetting,
}

/// Desktop lyrics window settings (geometry lives in `state.json`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DesktopLyricsSetting {
    /// The window is shown.
    pub enabled: bool,
    /// Point size of the current line (the next line is smaller).
    pub font_size: f32,
    /// Background opacity `0..=1` (the text stays opaque).
    pub opacity: f32,
    pub show_next_line: bool,
    pub show_title: bool,
    /// Click-through: the window ignores the mouse (unlock from the main window).
    pub locked: bool,
    /// The current track's album art left of the text (hidden in a narrow window).
    pub show_artwork: bool,
}

/// Range of [`DesktopLyricsSetting::font_size`].
pub const DESKTOP_LYRICS_FONT_RANGE: std::ops::RangeInclusive<f32> = 16.0..=72.0;

impl Default for DesktopLyricsSetting {
    fn default() -> Self {
        Self { enabled: false, font_size: 32.0, opacity: 0.7, show_next_line: true, show_title: false, locked: false, show_artwork: true }
    }
}

impl DesktopLyricsSetting {
    /// Values from a hand-edited file brought into range.
    pub fn sanitized(self) -> Self {
        let font_size = if self.font_size.is_finite() { self.font_size } else { Self::default().font_size };
        let opacity = if self.opacity.is_finite() { self.opacity } else { Self::default().opacity };
        Self {
            font_size: font_size.clamp(*DESKTOP_LYRICS_FONT_RANGE.start(), *DESKTOP_LYRICS_FONT_RANGE.end()),
            opacity: opacity.clamp(0.0, 1.0),
            ..self
        }
    }
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
            crossfeed_enabled: false,
            crossfeed_level: CrossfeedLevelSetting::default(),
            auto_headroom: true,
            dither: DitherSetting::default(),
            exclusive_mode: false,
            parametric_eq: ParametricEqSetting::default(),
            desktop_lyrics: DesktopLyricsSetting::default(),
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
        assert!(!cfg.crossfeed_enabled);
        assert_eq!(cfg.crossfeed_level, CrossfeedLevelSetting::Medium);
        assert!(cfg.auto_headroom);
        assert_eq!(cfg.dither, DitherSetting::Auto);
        assert!(!cfg.exclusive_mode);
        assert_eq!(cfg.parametric_eq, ParametricEqSetting::default());
        assert!(!cfg.parametric_eq.enabled && cfg.parametric_eq.filters.is_empty());
        let back: Config = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.resampler_taps, 64);
    }

    #[test]
    fn desktop_lyrics_defaults_and_partial_objects() {
        let old: Config = serde_json::from_str(r#"{"resampler_taps": 32, "preferred_output_device": null}"#).unwrap();
        let d = old.desktop_lyrics;
        assert_eq!(d, DesktopLyricsSetting::default());
        assert!(!d.enabled && !d.locked, "off and unlocked by default");
        assert!(d.show_next_line && !d.show_title);
        assert!(d.show_artwork, "album art on by default");
        assert!(DESKTOP_LYRICS_FONT_RANGE.contains(&d.font_size));
        assert!((0.0..=1.0).contains(&d.opacity));
        let partial: Config = serde_json::from_str(
            r#"{"resampler_taps": 32, "preferred_output_device": null, "desktop_lyrics": {"enabled": true, "opacity": 3.0, "font_size": 4}}"#,
        )
        .unwrap();
        assert!(partial.desktop_lyrics.enabled);
        assert!(partial.desktop_lyrics.show_next_line, "missing fields take their defaults");
        assert!(partial.desktop_lyrics.show_artwork, "configs written before the setting existed show the art");
        let s = partial.desktop_lyrics.sanitized();
        assert_eq!((s.opacity, s.font_size), (1.0, 16.0));
        let back: Config = serde_json::from_str(&serde_json::to_string(&partial).unwrap()).unwrap();
        assert_eq!(back.desktop_lyrics, partial.desktop_lyrics);
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

    #[test]
    fn sound_quality_defaults_match_old_configs() {
        let d = Config::default();
        let old: Config = serde_json::from_str(r#"{"resampler_taps": 32, "preferred_output_device": null}"#).unwrap();
        assert_eq!(old.crossfeed_enabled, d.crossfeed_enabled);
        assert_eq!(old.crossfeed_level, d.crossfeed_level);
        assert_eq!(old.auto_headroom, d.auto_headroom);
        assert_eq!(old.dither, d.dither);
        assert_eq!(old.exclusive_mode, d.exclusive_mode);
        assert_eq!(old.parametric_eq, d.parametric_eq);
        // A partial parametric EQ object (e.g. hand-edited) fills in the rest.
        let partial: Config = serde_json::from_str(
            r#"{"resampler_taps": 32, "preferred_output_device": null,
                "parametric_eq": {"enabled": true, "filters": [{"kind": "low_shelf", "gain_db": 3.0}, {}]}}"#,
        )
        .unwrap();
        assert!(partial.parametric_eq.enabled);
        assert_eq!(partial.parametric_eq.preamp_db, 0.0);
        let f = partial.parametric_eq.filters[0];
        assert_eq!(f.kind, PeqFilterKindSetting::LowShelf);
        assert_eq!((f.freq_hz, f.gain_db), (1000.0, 3.0));
        assert!((f.q - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(partial.parametric_eq.filters[1], PeqFilterSetting::default());
    }

    #[test]
    fn sound_quality_settings_round_trip() {
        let cfg = Config {
            crossfeed_enabled: true,
            crossfeed_level: CrossfeedLevelSetting::Strong,
            auto_headroom: false,
            dither: DitherSetting::Off,
            exclusive_mode: true,
            parametric_eq: ParametricEqSetting {
                enabled: true,
                preamp_db: -6.2,
                filters: vec![
                    PeqFilterSetting { kind: PeqFilterKindSetting::Peak, freq_hz: 105.0, gain_db: 4.2, q: 0.7 },
                    PeqFilterSetting { kind: PeqFilterKindSetting::HighShelf, freq_hz: 10_000.0, gain_db: -3.0, q: 0.71 },
                    PeqFilterSetting { kind: PeqFilterKindSetting::HighPass, freq_hz: 20.0, gain_db: 0.0, q: 0.5 },
                ],
            },
            ..Config::default()
        };
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["crossfeed_level"], "strong");
        assert_eq!(json["dither"], "off");
        assert_eq!(json["parametric_eq"]["filters"][1]["kind"], "high_shelf");
        let back: Config = serde_json::from_value(json).unwrap();
        assert!(back.crossfeed_enabled);
        assert_eq!(back.crossfeed_level, CrossfeedLevelSetting::Strong);
        assert!(!back.auto_headroom);
        assert_eq!(back.dither, DitherSetting::Off);
        assert!(back.exclusive_mode);
        assert_eq!(back.parametric_eq, cfg.parametric_eq);
        for (text, level) in [("weak", CrossfeedLevelSetting::Weak), ("medium", CrossfeedLevelSetting::Medium)] {
            let v: CrossfeedLevelSetting = serde_json::from_str(&format!("\"{text}\"")).unwrap();
            assert_eq!(v, level);
        }
    }
}
