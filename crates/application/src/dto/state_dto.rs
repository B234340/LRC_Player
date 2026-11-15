use crate::config::ArtistViewMode;
use lrc_player_domain::model::PlaybackMode;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json;

fn deserialize_last_scan_status<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)
        .map_err(serde::de::Error::custom)?;

    match value {
        None => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s)),
        Some(other) => {
            let s = other.to_string();
            if s.chars().all(|c| c.is_ascii_digit()) {
                Ok(None)
            } else {
                Ok(Some(s))
            }
        }
    }
}

fn default_side_panel_expanded() -> bool {
    true
}

fn default_volume() -> f32 {
    DEFAULT_VOLUME
}

/// Default UI volume (perceptual 0..=1 slider position).
pub const DEFAULT_VOLUME: f32 = 0.8;

/// Number of equalizer bands persisted in [`EqDto`] (matches the engine's 10-band EQ).
pub const EQ_BAND_COUNT: usize = 10;

/// Repeat mode, independent of shuffle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    /// Stop after the last queue item.
    #[default]
    Off,
    /// Repeat the whole queue.
    All,
    /// Repeat the current track (auto-advance only; Next/Previous still move).
    One,
}

impl RepeatMode {
    /// Off -> All -> One -> Off.
    pub fn cycle(self) -> Self {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RepeatMode::Off => "Repeat off",
            RepeatMode::All => "Repeat all",
            RepeatMode::One => "Repeat one",
        }
    }
}

/// Split (shuffle, repeat) from the legacy single-valued [`PlaybackMode`].
pub fn split_playback_mode(mode: PlaybackMode) -> (bool, RepeatMode) {
    match mode {
        PlaybackMode::Sequential => (false, RepeatMode::Off),
        PlaybackMode::RepeatOne => (false, RepeatMode::One),
        PlaybackMode::RepeatAll => (false, RepeatMode::All),
        PlaybackMode::Shuffle => (true, RepeatMode::Off),
    }
}

/// Best-effort legacy [`PlaybackMode`] for (shuffle, repeat) so older builds still read a
/// sensible value from `queue_playback_mode`.
pub fn legacy_playback_mode(shuffle: bool, repeat: RepeatMode) -> PlaybackMode {
    match (shuffle, repeat) {
        (_, RepeatMode::One) => PlaybackMode::RepeatOne,
        (true, _) => PlaybackMode::Shuffle,
        (false, RepeatMode::All) => PlaybackMode::RepeatAll,
        (false, RepeatMode::Off) => PlaybackMode::Sequential,
    }
}

/// Lyric text shift for one track: the text slides over fixed timestamps by `lines` lines and
/// `chars` karaoke characters (positive = later).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsShiftDto {
    #[serde(default)]
    pub lines: i32,
    #[serde(default)]
    pub chars: i32,
}

impl LyricsShiftDto {
    pub fn is_zero(&self) -> bool {
        self.lines == 0 && self.chars == 0
    }
}

/// Last window geometry in logical points (position is `None` where the platform does
/// not report it, e.g. Wayland).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowStateDto {
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub x: Option<f32>,
    #[serde(default)]
    pub y: Option<f32>,
    #[serde(default)]
    pub maximized: bool,
}

/// Persisted equalizer settings (mirrors the engine's `EqSettings`; dB values).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct EqDto {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub preamp_db: f32,
    #[serde(default)]
    pub bands_db: [f32; EQ_BAND_COUNT],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDto {
    pub last_tab: Option<String>,

    #[serde(default)]
    pub last_play_path: Option<String>,

    #[serde(default)]
    pub last_play_position_ms: Option<u64>,

    #[serde(default, deserialize_with = "deserialize_last_scan_status")]
    pub last_scan_status: Option<String>,

    #[serde(default)]
    pub scan_scanned: Option<u64>,
    #[serde(default)]
    pub scan_parsed: Option<u64>,
    #[serde(default)]
    pub scan_registered: Option<u64>,

    #[serde(default)]
    pub scan_targets: Vec<String>,
    #[serde(default)]
    pub scan_lrc_targets: Vec<String>,
    #[serde(default)]
    pub scan_excludes: Vec<String>,
    #[serde(default)]
    pub queue_tracks: Vec<String>,
    #[serde(default)]
    pub selected_queue_index: Option<usize>,
    #[serde(default)]
    pub queue_playback_mode: PlaybackMode,
    #[serde(default)]
    pub artist_view_mode: ArtistViewMode,
    #[serde(default)]
    pub lrc_overlay_enabled: bool,
    #[serde(default)]
    pub fft_overlay_enabled: bool,

    #[serde(default = "default_side_panel_expanded")]
    pub side_panel_expanded: bool,

    #[serde(default)]
    pub favorite_directories: Vec<String>,

    /// UI volume as a perceptual 0..=1 slider position (see `volume_to_linear`).
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub eq: EqDto,
    /// `None` in files written before shuffle/repeat were split; derive from
    /// `queue_playback_mode` with [`StateDto::shuffle_repeat`].
    #[serde(default)]
    pub shuffle: Option<bool>,
    #[serde(default)]
    pub repeat: Option<RepeatMode>,
    /// Per-track lyrics offsets (track path -> ms, positive = lyrics earlier).
    #[serde(default)]
    pub lyrics_offsets: std::collections::HashMap<String, i64>,
    /// Per-track lyric text shifts (track path -> shift).
    #[serde(default)]
    pub lyrics_shifts: std::collections::HashMap<String, LyricsShiftDto>,
    /// `.lrc` files attached to tracks by drag & drop (track path -> lyrics path).
    #[serde(default)]
    pub lyrics_overrides: std::collections::HashMap<String, String>,
    /// Right-hand panel of the shell: `"queue"`, `"lyrics"` or `None` (hidden).
    #[serde(default)]
    pub right_panel: Option<String>,
    /// Sidebar collapsed to icons.
    #[serde(default)]
    pub sidebar_collapsed: bool,
    #[serde(default)]
    pub window: Option<WindowStateDto>,
    /// Optional library table columns that are shown (`"year"`, `"genre"`).
    #[serde(default)]
    pub library_columns: Vec<String>,
    /// Album grid card width in logical px (`None` = default).
    #[serde(default)]
    pub album_card_size: Option<f32>,
    /// Desktop lyrics window geometry (logical px; `maximized` unused).
    #[serde(default)]
    pub desktop_lyrics_window: Option<WindowStateDto>,
}

impl Default for StateDto {
    fn default() -> Self {
        Self {
            last_tab: None,
            last_play_path: None,
            last_play_position_ms: None,
            last_scan_status: None,
            scan_scanned: None,
            scan_parsed: None,
            scan_registered: None,
            scan_targets: Vec::new(),
            scan_lrc_targets: Vec::new(),
            scan_excludes: Vec::new(),
            queue_tracks: Vec::new(),
            selected_queue_index: None,
            queue_playback_mode: PlaybackMode::Sequential,
            artist_view_mode: ArtistViewMode::Artist,
            lrc_overlay_enabled: false,
            fft_overlay_enabled: false,
            side_panel_expanded: true,
            favorite_directories: Vec::new(),
            volume: DEFAULT_VOLUME,
            muted: false,
            eq: EqDto::default(),
            shuffle: None,
            repeat: None,
            lyrics_offsets: std::collections::HashMap::new(),
            lyrics_shifts: std::collections::HashMap::new(),
            lyrics_overrides: std::collections::HashMap::new(),
            right_panel: None,
            sidebar_collapsed: false,
            window: None,
            library_columns: Vec::new(),
            album_card_size: None,
            desktop_lyrics_window: None,
        }
    }
}

impl StateDto {
    /// Effective (shuffle, repeat): the new fields when present, else derived from the legacy
    /// `queue_playback_mode`.
    pub fn shuffle_repeat(&self) -> (bool, RepeatMode) {
        let (legacy_shuffle, legacy_repeat) = split_playback_mode(self.queue_playback_mode);
        (
            self.shuffle.unwrap_or(legacy_shuffle),
            self.repeat.unwrap_or(legacy_repeat),
        )
    }
}

/// Perceptual slider position (0..=1) -> linear gain for the engine (0..=1).
///
/// Cubic taper (close to a logarithmic fader over ~50 dB): 0 -> silence, 1 -> unity gain,
/// 0.5 -> 0.125 (about -18 dB).
pub fn volume_to_linear(perceptual: f32) -> f32 {
    let p = perceptual.clamp(0.0, 1.0);
    if p <= 0.0 {
        return 0.0;
    }
    // x^3 approximates a log taper well over ~50 dB and maps 0 to true silence.
    p * p * p
}

/// Inverse of [`volume_to_linear`].
pub fn linear_to_volume(linear: f32) -> f32 {
    linear.clamp(0.0, 1.0).cbrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state.json written before volume / EQ / shuffle+repeat existed.
    const OLD_STATE_JSON: &str = r#"{
        "last_tab": "Play",
        "last_play_path": "/music/a.flac",
        "last_play_position_ms": 1234,
        "last_scan_status": "Scan completed",
        "scan_scanned": null,
        "scan_parsed": null,
        "scan_registered": null,
        "scan_targets": ["/music"],
        "scan_lrc_targets": [],
        "scan_excludes": [],
        "queue_tracks": ["/music/a.flac", "/music/b.flac"],
        "selected_queue_index": 1,
        "queue_playback_mode": "Shuffle",
        "artist_view_mode": "artist",
        "lrc_overlay_enabled": true,
        "fft_overlay_enabled": false,
        "side_panel_expanded": true,
        "favorite_directories": []
    }"#;

    #[test]
    fn old_state_json_still_loads_with_defaults() {
        let state: StateDto = serde_json::from_str(OLD_STATE_JSON).unwrap();
        assert_eq!(state.last_play_position_ms, Some(1234));
        assert_eq!(state.queue_tracks.len(), 2);
        assert_eq!(state.volume, DEFAULT_VOLUME);
        assert!(!state.muted);
        assert_eq!(state.eq, EqDto::default());
        assert_eq!(state.shuffle, None);
        assert_eq!(state.shuffle_repeat(), (true, RepeatMode::Off));
        assert!(state.lyrics_offsets.is_empty());
        assert!(state.lyrics_shifts.is_empty());
        assert!(state.lyrics_overrides.is_empty());
        assert_eq!(state.right_panel, None);
        assert!(!state.sidebar_collapsed);
        assert_eq!(state.window, None);
        assert_eq!(state.desktop_lyrics_window, None);
    }

    #[test]
    fn minimal_state_json_loads() {
        let state: StateDto = serde_json::from_str(r#"{"last_tab": null}"#).unwrap();
        assert!(state.side_panel_expanded);
        assert_eq!(state.shuffle_repeat(), (false, RepeatMode::Off));
    }

    #[test]
    fn new_fields_roundtrip() {
        let mut state = StateDto {
            volume: 0.5,
            muted: true,
            shuffle: Some(true),
            repeat: Some(RepeatMode::All),
            ..Default::default()
        };
        state.eq.enabled = true;
        state.eq.bands_db[3] = -4.5;
        state.lyrics_offsets.insert("/x.flac".into(), 250);
        state.right_panel = Some("lyrics".into());
        state.sidebar_collapsed = true;
        state.window = Some(WindowStateDto { width: 1000.0, height: 700.0, x: Some(10.0), y: None, maximized: false });
        let json = serde_json::to_string(&state).unwrap();
        let back: StateDto = serde_json::from_str(&json).unwrap();
        assert_eq!(back.volume, 0.5);
        assert!(back.muted);
        assert_eq!(back.eq, state.eq);
        assert_eq!(back.shuffle_repeat(), (true, RepeatMode::All));
        assert_eq!(back.lyrics_offsets.get("/x.flac"), Some(&250));
        assert_eq!(back.right_panel.as_deref(), Some("lyrics"));
        assert!(back.sidebar_collapsed);
        assert_eq!(back.window, state.window);
    }

    #[test]
    fn legacy_mode_mapping_roundtrips() {
        for mode in [
            PlaybackMode::Sequential,
            PlaybackMode::RepeatOne,
            PlaybackMode::RepeatAll,
            PlaybackMode::Shuffle,
        ] {
            let (s, r) = split_playback_mode(mode);
            assert_eq!(legacy_playback_mode(s, r), mode);
        }
    }

    #[test]
    fn volume_taper_is_monotonic_and_invertible() {
        assert_eq!(volume_to_linear(0.0), 0.0);
        assert_eq!(volume_to_linear(1.0), 1.0);
        let mut prev = 0.0;
        for i in 1..=100 {
            let v = volume_to_linear(i as f32 / 100.0);
            assert!(v > prev);
            prev = v;
            assert!((linear_to_volume(v) - i as f32 / 100.0).abs() < 1e-4);
        }
    }
}
