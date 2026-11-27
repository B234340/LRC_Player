//! Sound-quality settings on top of the engine: crossfeed, automatic headroom, dither,
//! WASAPI exclusive mode and the parametric EQ (with AutoEq import).
//!
//! | method | effect |
//! |---|---|
//! | [`GuiApp::set_parametric_eq`] / [`GuiApp::parametric_eq`] | PEQ (live; saved debounced) |
//! | [`GuiApp::set_peq_channel_mode`] / [`GuiApp::copy_peq_channel`] | L+R (linked) or L・R 別, copy L ↔ R |
//! | [`GuiApp::import_autoeq_text`] | parse an AutoEq / Equalizer APO profile into the edited channel |
//! | [`GuiApp::pick_autoeq_file`] | native `.txt` picker on a worker, then import |
//! | [`GuiApp::set_crossfeed`], [`GuiApp::set_auto_headroom`], [`GuiApp::set_dither`] | live + saved |
//! | [`GuiApp::set_exclusive`] | Windows only; failures arrive as `ExclusiveUnavailable` |
//!
//! Everything is persisted in `config.json` (see `application::config`) and applied when the
//! engine is created ([`apply_to_engine`]).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use lrc_player_application::config::{
    Config, CrossfeedLevelSetting, DitherSetting, ParametricEqSetting, PeqChannelModeSetting, PeqChannelSetting,
    PeqFilterKindSetting, PeqFilterSetting,
};
use lrc_player_application::infrastructure::audio::engine::{
    parse_autoeq, AudioEngine, Crossfeed, CrossfeedLevel, DitherMode, FilterKind, ParametricEq, PeqChannel,
    PeqChannelMode, PeqFilter, PeqSide, PEQ_MAX_FILTERS, PEQ_MAX_GAIN_DB,
};

use crate::presentation::app::GuiApp;

/// Exclusive output exists only on Windows (WASAPI).
pub(crate) const EXCLUSIVE_SUPPORTED: bool = cfg!(windows);

/// Config writes for continuous edits (PEQ drags) are coalesced this long.
pub(crate) const CONFIG_SAVE_DEBOUNCE: Duration = Duration::from_millis(600);

/// Frequency range offered by the PEQ editor.
pub(crate) const PEQ_FREQ_RANGE: std::ops::RangeInclusive<f32> = 20.0..=20_000.0;
/// Q range offered by the PEQ editor.
pub(crate) const PEQ_Q_RANGE: std::ops::RangeInclusive<f32> = 0.1..=10.0;

/// Result of the AutoEq file picker: `None` = cancelled, else the file's text or an error.
pub(crate) type AutoEqPick = Option<(PathBuf, Result<String, String>)>;

fn kind_from_config(k: PeqFilterKindSetting) -> FilterKind {
    match k {
        PeqFilterKindSetting::Peak => FilterKind::Peak,
        PeqFilterKindSetting::LowShelf => FilterKind::LowShelf,
        PeqFilterKindSetting::HighShelf => FilterKind::HighShelf,
        PeqFilterKindSetting::LowPass => FilterKind::LowPass,
        PeqFilterKindSetting::HighPass => FilterKind::HighPass,
    }
}

fn kind_to_config(k: FilterKind) -> PeqFilterKindSetting {
    match k {
        FilterKind::Peak => PeqFilterKindSetting::Peak,
        FilterKind::LowShelf => PeqFilterKindSetting::LowShelf,
        FilterKind::HighShelf => PeqFilterKindSetting::HighShelf,
        FilterKind::LowPass => PeqFilterKindSetting::LowPass,
        FilterKind::HighPass => PeqFilterKindSetting::HighPass,
    }
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

/// A filter within the editor's ranges (frequency 20 Hz–20 kHz, gain ±30 dB, Q 0.1–10).
pub(crate) fn clamp_filter(f: PeqFilter) -> PeqFilter {
    PeqFilter {
        kind: f.kind,
        freq_hz: finite_or(f.freq_hz, 1000.0).clamp(*PEQ_FREQ_RANGE.start(), *PEQ_FREQ_RANGE.end()),
        gain_db: finite_or(f.gain_db, 0.0).clamp(-PEQ_MAX_GAIN_DB, PEQ_MAX_GAIN_DB),
        q: finite_or(f.q, std::f32::consts::FRAC_1_SQRT_2).clamp(*PEQ_Q_RANGE.start(), *PEQ_Q_RANGE.end()),
    }
}

fn filters_from_config(f: &[PeqFilterSetting]) -> Vec<PeqFilter> {
    f.iter()
        .take(PEQ_MAX_FILTERS)
        .map(|f| clamp_filter(PeqFilter { kind: kind_from_config(f.kind), freq_hz: f.freq_hz, gain_db: f.gain_db, q: f.q }))
        .collect()
}

fn filters_to_config(f: &[PeqFilter]) -> Vec<PeqFilterSetting> {
    f.iter().map(|f| PeqFilterSetting { kind: kind_to_config(f.kind), freq_hz: f.freq_hz, gain_db: f.gain_db, q: f.q }).collect()
}

fn clamp_preamp(db: f32) -> f32 {
    finite_or(db, 0.0).clamp(-PEQ_MAX_GAIN_DB, PEQ_MAX_GAIN_DB)
}

/// Engine PEQ from the config (at most [`PEQ_MAX_FILTERS`] filters per channel, values
/// clamped).
pub(crate) fn peq_from_config(s: &ParametricEqSetting) -> ParametricEq {
    ParametricEq {
        enabled: s.enabled,
        preamp_db: clamp_preamp(s.preamp_db),
        filters: filters_from_config(&s.filters),
        channel_mode: match s.channel_mode {
            PeqChannelModeSetting::Linked => PeqChannelMode::Linked,
            PeqChannelModeSetting::Separate => PeqChannelMode::Separate,
        },
        right: PeqChannel { preamp_db: clamp_preamp(s.right.preamp_db), filters: filters_from_config(&s.right.filters) },
    }
}

pub(crate) fn peq_to_config(p: &ParametricEq) -> ParametricEqSetting {
    ParametricEqSetting {
        enabled: p.enabled,
        preamp_db: p.preamp_db,
        filters: filters_to_config(&p.filters),
        channel_mode: match p.channel_mode {
            PeqChannelMode::Linked => PeqChannelModeSetting::Linked,
            PeqChannelMode::Separate => PeqChannelModeSetting::Separate,
        },
        right: PeqChannelSetting { preamp_db: p.right.preamp_db, filters: filters_to_config(&p.right.filters) },
    }
}

pub(crate) fn crossfeed_from_config(cfg: &Config) -> Crossfeed {
    Crossfeed {
        enabled: cfg.crossfeed_enabled,
        level: match cfg.crossfeed_level {
            CrossfeedLevelSetting::Weak => CrossfeedLevel::Weak,
            CrossfeedLevelSetting::Medium => CrossfeedLevel::Medium,
            CrossfeedLevelSetting::Strong => CrossfeedLevel::Strong,
        },
    }
}

pub(crate) fn dither_from_config(cfg: &Config) -> DitherMode {
    match cfg.dither {
        DitherSetting::Auto => DitherMode::Auto,
        DitherSetting::Off => DitherMode::Off,
    }
}

/// Exclusive mode as requested from the engine (never on where it is unsupported, so other
/// OSes do not get an "unavailable" notice at every start).
pub(crate) fn exclusive_from_config(cfg: &Config) -> bool {
    EXCLUSIVE_SUPPORTED && cfg.exclusive_mode
}

/// Applies the sound-quality settings of `cfg` to a new engine (exclusive mode is part of
/// its `EngineConfig`).
pub(crate) fn apply_to_engine(engine: &AudioEngine, cfg: &Config) {
    engine.set_parametric_eq(peq_from_config(&cfg.parametric_eq));
    engine.set_crossfeed(crossfeed_from_config(cfg));
    engine.set_auto_headroom(cfg.auto_headroom);
    engine.set_dither(dither_from_config(cfg));
}

impl GuiApp {
    // ---- config persistence (debounced) ----

    /// Saves `config.json` after [`CONFIG_SAVE_DEBOUNCE`] without further changes.
    pub(crate) fn save_config_soon(&mut self) {
        self.config_save_due = Some(Instant::now() + CONFIG_SAVE_DEBOUNCE);
    }

    /// Writes a pending debounced config save when due (or now with `force`).
    pub(crate) fn flush_config_if_due(&mut self, force: bool) {
        if let Some(due) = self.config_save_due {
            if force || Instant::now() >= due {
                self.config_save_due = None;
                self.save_config();
            }
        }
    }

    // ---- parametric EQ ----

    pub fn parametric_eq(&self) -> &ParametricEq {
        &self.peq
    }

    /// Replaces the PEQ (filters beyond [`PEQ_MAX_FILTERS`] are dropped, values clamped to
    /// the editor ranges), applies it live and saves it (debounced).
    pub fn set_parametric_eq(&mut self, mut peq: ParametricEq) {
        for filters in [&mut peq.filters, &mut peq.right.filters] {
            filters.truncate(PEQ_MAX_FILTERS);
            for f in filters.iter_mut() {
                *f = clamp_filter(*f);
            }
        }
        peq.preamp_db = clamp_preamp(peq.preamp_db);
        peq.right.preamp_db = clamp_preamp(peq.right.preamp_db);
        if peq.channel_mode == PeqChannelMode::Linked {
            self.ui.peq_side = PeqSide::Left;
        }
        if peq == self.peq {
            return;
        }
        if let Some(engine) = &self.engine {
            engine.set_parametric_eq(peq.clone());
        }
        self.config.parametric_eq = peq_to_config(&peq);
        self.peq = peq;
        self.save_config_soon();
    }

    /// The channel the PEQ editor shows and edits (always `Left` = the shared set when
    /// linked).
    pub fn peq_edit_side(&self) -> PeqSide {
        match self.peq.channel_mode {
            PeqChannelMode::Linked => PeqSide::Left,
            PeqChannelMode::Separate => self.ui.peq_side,
        }
    }

    pub fn set_peq_edit_side(&mut self, side: PeqSide) {
        self.ui.peq_side = side;
    }

    /// L+R (one set) or L・R 別. The first switch to separate channels starts the right
    /// channel as a copy of the shared set (so nothing changes audibly); later switches keep
    /// the right channel that was saved.
    pub fn set_peq_channel_mode(&mut self, mode: PeqChannelMode) {
        if mode == self.peq.channel_mode {
            return;
        }
        let mut peq = self.peq.clone();
        if mode == PeqChannelMode::Separate && peq.right == PeqChannel::default() {
            peq.right = PeqChannel { preamp_db: peq.preamp_db, filters: peq.filters.clone() };
        }
        peq.channel_mode = mode;
        self.set_parametric_eq(peq);
    }

    /// Copies the filters + preamp of `from` to the other channel (separate mode only).
    pub fn copy_peq_channel(&mut self, from: PeqSide) {
        if self.peq.channel_mode != PeqChannelMode::Separate {
            return;
        }
        let mut peq = self.peq.clone();
        peq.set_channel(from.other(), peq.channel(from));
        self.set_parametric_eq(peq);
    }

    /// Parses an AutoEq / Equalizer APO `ParametricEQ.txt` and replaces the edited channel's
    /// filters + preamp with it (PEQ enabled). Toasts the outcome; returns whether it was
    /// imported.
    pub fn import_autoeq_text(&mut self, text: &str) -> bool {
        let tr = self.tr();
        match parse_autoeq(text) {
            Ok(parsed) => {
                let mut peq = self.peq.clone();
                peq.enabled = true;
                let n = parsed.filters.len();
                peq.set_channel(self.peq_edit_side(), PeqChannel { preamp_db: parsed.preamp_db, filters: parsed.filters });
                self.set_parametric_eq(peq);
                self.toast_success(tr.f("toast.autoeq_loaded", &[&n]));
                true
            }
            Err(err) => {
                self.toast_error(tr.f("toast.autoeq_failed", &[&format!("{err:#}")]));
                false
            }
        }
    }

    /// Opens the native file dialog (`.txt`) on a worker thread; the file is read there too.
    pub(crate) fn pick_autoeq_file(&mut self, ctx: &egui::Context) {
        if self.ui.autoeq_picker.is_some() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded::<AutoEqPick>(1);
        let wake = ctx.clone();
        let title = self.tr().t("peq.import_title");
        let filter_name = self.tr().t("peq.file_filter");
        let spawned = std::thread::Builder::new().name("autoeq-picker".into()).spawn(move || {
            // Same arrangement as the folder picker (`scan.rs`).
            let picked = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()
                .and_then(|rt| {
                    rt.block_on(
                        rfd::AsyncFileDialog::new()
                            .set_title(title)
                            .add_filter(filter_name, &["txt"])
                            .add_filter("*", &["*"])
                            .pick_file(),
                    )
                })
                .map(|h| {
                    let path = h.path().to_path_buf();
                    let text = read_profile(&path);
                    (path, text)
                });
            let _ = tx.send(picked);
            wake.request_repaint();
        });
        if spawned.is_ok() {
            self.ui.autoeq_picker = Some(rx);
        }
    }

    pub(crate) fn poll_autoeq_picker(&mut self) {
        let Some(rx) = &self.ui.autoeq_picker else { return };
        let picked = match rx.try_recv() {
            Ok(p) => p,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => None,
        };
        self.ui.autoeq_picker = None;
        self.apply_autoeq_pick(picked);
    }

    pub(crate) fn apply_autoeq_pick(&mut self, picked: AutoEqPick) {
        let Some((path, text)) = picked else { return };
        match text {
            Ok(text) => {
                self.import_autoeq_text(&text);
            }
            Err(err) => {
                let tr = self.tr();
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.toast_error(tr.f("toast.autoeq_read_failed", &[&name, &err]));
            }
        }
    }

    // ---- crossfeed / headroom / dither ----

    pub fn crossfeed(&self) -> Crossfeed {
        crossfeed_from_config(&self.config)
    }

    pub fn set_crossfeed(&mut self, c: Crossfeed) {
        if c == self.crossfeed() {
            return;
        }
        self.config.crossfeed_enabled = c.enabled;
        self.config.crossfeed_level = match c.level {
            CrossfeedLevel::Weak => CrossfeedLevelSetting::Weak,
            CrossfeedLevel::Medium => CrossfeedLevelSetting::Medium,
            CrossfeedLevel::Strong => CrossfeedLevelSetting::Strong,
        };
        if let Some(engine) = &self.engine {
            engine.set_crossfeed(c);
        }
        self.save_config();
    }

    pub fn auto_headroom(&self) -> bool {
        self.config.auto_headroom
    }

    pub fn set_auto_headroom(&mut self, on: bool) {
        if on == self.config.auto_headroom {
            return;
        }
        self.config.auto_headroom = on;
        if let Some(engine) = &self.engine {
            engine.set_auto_headroom(on);
        }
        self.save_config();
    }

    pub fn dither(&self) -> DitherMode {
        dither_from_config(&self.config)
    }

    pub fn set_dither(&mut self, mode: DitherMode) {
        if mode == self.dither() {
            return;
        }
        self.config.dither = match mode {
            DitherMode::Auto => DitherSetting::Auto,
            DitherMode::Off => DitherSetting::Off,
        };
        if let Some(engine) = &self.engine {
            engine.set_dither(mode);
        }
        self.save_config();
    }

    // ---- exclusive mode ----

    /// State of the exclusive-mode switch: requested, and not reported unavailable since.
    pub fn exclusive_switch_state(&self) -> bool {
        exclusive_from_config(&self.config) && self.exclusive_unavailable.is_none()
    }

    /// Why exclusive mode could not be used (last `ExclusiveUnavailable` reason), if it is
    /// requested and not in effect.
    pub fn exclusive_unavailable_reason(&self) -> Option<&str> {
        self.exclusive_unavailable.as_deref()
    }

    /// Turns WASAPI exclusive mode on/off (Windows only; elsewhere this is a no-op). Turning
    /// it on again retries after an earlier failure.
    pub fn set_exclusive(&mut self, on: bool) {
        if !EXCLUSIVE_SUPPORTED {
            return;
        }
        self.exclusive_unavailable = None;
        self.config.exclusive_mode = on;
        if let Some(engine) = &self.engine {
            engine.set_exclusive(on);
        }
        self.save_config();
    }

    /// `EngineEvent::ExclusiveUnavailable`: playback continues on the shared output. Shows
    /// one warning per distinct reason and turns the switch off (the preference is kept, so
    /// the engine may still get exclusive access later; switching on again retries now).
    pub(crate) fn on_exclusive_unavailable(&mut self, reason: String) {
        tracing::warn!("exclusive output unavailable: {reason}");
        if self.exclusive_unavailable.as_deref() == Some(reason.as_str()) {
            return;
        }
        let tr = self.tr();
        self.toast_warning(tr.f("toast.exclusive_unavailable", &[&reason]));
        self.exclusive_unavailable = Some(reason);
    }

    /// `DeviceChanged`: an exclusive device clears an earlier "unavailable" notice.
    pub(crate) fn on_output_device_changed(&mut self, exclusive: bool) {
        if exclusive {
            self.exclusive_unavailable = None;
        }
    }
}

/// Reads a profile file (at most 1 MiB; UTF-8 with a lossy fallback, e.g. for UTF-16 BOMs).
fn read_profile(path: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > 1 << 20 {
        return Err("file too large".into());
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let le = bytes[0] == 0xFF;
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
            .collect();
        return Ok(String::from_utf16_lossy(&units));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peq_config_round_trip_and_clamping() {
        let peq = ParametricEq {
            enabled: true,
            preamp_db: -6.5,
            filters: vec![
                PeqFilter { kind: FilterKind::LowShelf, freq_hz: 105.0, gain_db: 4.0, q: 0.7 },
                PeqFilter { kind: FilterKind::HighPass, freq_hz: 30.0, gain_db: 0.0, q: 0.5 },
            ],
            ..Default::default()
        };
        assert_eq!(peq_from_config(&peq_to_config(&peq)), peq);
        let lr = ParametricEq {
            channel_mode: PeqChannelMode::Separate,
            right: PeqChannel { preamp_db: -2.0, filters: vec![PeqFilter { kind: FilterKind::Peak, freq_hz: 3000.0, gain_db: -3.0, q: 2.0 }] },
            ..peq.clone()
        };
        assert_eq!(peq_from_config(&peq_to_config(&lr)), lr);
        let wild = ParametricEqSetting {
            enabled: true,
            preamp_db: f32::NAN,
            filters: (0..30)
                .map(|_| PeqFilterSetting { kind: PeqFilterKindSetting::Peak, freq_hz: 5.0, gain_db: 99.0, q: 50.0 })
                .collect(),
            ..Default::default()
        };
        let p = peq_from_config(&wild);
        assert_eq!(p.preamp_db, 0.0);
        assert_eq!(p.filters.len(), PEQ_MAX_FILTERS);
        assert_eq!((p.filters[0].freq_hz, p.filters[0].gain_db, p.filters[0].q), (20.0, 30.0, 10.0));
    }

    #[test]
    fn utf16_profiles_are_decoded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.txt");
        let mut bytes = vec![0xFF, 0xFE];
        for u in "Preamp: -1 dB".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(read_profile(&path).unwrap(), "Preamp: -1 dB");
    }

    // ---- app level ----------------------------------------------------------------------

    use crate::presentation::app::tests::test_app;
    use lrc_player_application::infrastructure::audio::engine::{EngineConfig, OutputMode};

    const PROFILE: &str = "Preamp: -6.2 dB\n\
        Filter 1: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.70\n\
        Filter 2: ON PK Fc 2400 Hz Gain -3.1 dB Q 1.41\n\
        Filter 3: OFF PK Fc 5000 Hz Gain 2.0 dB Q 2.00\n\
        Filter 4: ON HSC Fc 10000 Hz Gain -2.0 dB Q 0.70\n";

    /// Stereo 16-bit 48 kHz WAV of a quiet sine.
    fn write_wav(path: &std::path::Path, seconds: f32) {
        let rate = 48_000u32;
        let frames = (rate as f32 * seconds) as u32;
        let mut data = Vec::new();
        for i in 0..frames {
            let v = ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 4_000.0) as i16;
            data.extend_from_slice(&v.to_le_bytes());
            data.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 4).to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        std::fs::write(path, out).unwrap();
    }

    /// A picked AutoEq file ends up in `app.peq`, the config and the engine (the engine's
    /// signal path reports the parametric EQ as active).
    #[test]
    fn autoeq_import_reaches_peq_config_and_engine() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("a.wav");
        write_wav(&wav, 3.0);
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx, dir.path(), false);
        app.engine = Some(
            AudioEngine::new(EngineConfig {
                output: OutputMode::Manual { sample_rate: 48_000, channels: 2 },
                ring_ms: 500,
                prebuffer_ms: 100,
                ..Default::default()
            })
            .unwrap(),
        );
        // Untouched audio first (auto headroom has nothing to do without EQ).
        app.set_dither(DitherMode::Off);
        let engine = app.engine.as_ref().unwrap();
        engine.load(wav.clone(), 0, true);
        let mut buf = vec![0.0f32; 960];
        let signal = |app: &GuiApp, want_peq: bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let engine = app.engine.as_ref().unwrap();
                let mut buf = vec![0.0f32; 960];
                engine.render_manual(&mut buf);
                let snap = engine.snapshot();
                if let Some(s) = snap.signal {
                    if s.parametric_eq == want_peq {
                        return s;
                    }
                }
                assert!(Instant::now() < deadline, "engine never reported parametric_eq = {want_peq}");
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        engine.render_manual(&mut buf);
        assert!(!signal(&app, false).parametric_eq);

        // The picker's result (file read on the worker) goes through the same path.
        let profile = dir.path().join("ParametricEQ.txt");
        std::fs::write(&profile, PROFILE).unwrap();
        app.apply_autoeq_pick(Some((profile.clone(), read_profile(&profile))));
        assert!(app.peq.enabled);
        assert_eq!(app.peq.filters.len(), 3, "OFF filters are skipped");
        assert!((app.peq.preamp_db + 6.2).abs() < 1e-4);
        assert_eq!(app.peq.filters[0].kind, FilterKind::LowShelf);
        assert_eq!(app.peq.filters[1].freq_hz, 2400.0);
        assert_eq!(app.config.parametric_eq, peq_to_config(&app.peq));
        assert!(app.config_save_due.is_some(), "saved debounced");
        let toast = app.ui.last_toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
        assert!(toast.contains('3'), "success toast: {toast}");
        let s = signal(&app, true);
        assert!(!s.bit_perfect);

        // A broken file keeps the filters and reports an error.
        let before = app.peq.clone();
        assert!(!app.import_autoeq_text("Preamp: -3 dB\n"));
        assert_eq!(app.peq, before);
        let toast = app.ui.last_toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
        assert!(toast.contains("AutoEq"), "error toast: {toast}");

        // Disabling it reaches the engine too.
        app.set_parametric_eq(ParametricEq { enabled: false, ..app.peq.clone() });
        signal(&app, false);
        app.flush_config_if_due(true);
        assert!(app.config_save_due.is_none());
    }

    #[test]
    fn settings_setters_update_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx, dir.path(), false);
        assert_eq!(app.crossfeed(), Crossfeed { enabled: false, level: CrossfeedLevel::Medium });
        assert!(app.auto_headroom());
        assert_eq!(app.dither(), DitherMode::Auto);
        app.set_crossfeed(Crossfeed { enabled: true, level: CrossfeedLevel::Strong });
        app.set_auto_headroom(false);
        app.set_dither(DitherMode::Off);
        assert!(app.config.crossfeed_enabled);
        assert_eq!(app.config.crossfeed_level, CrossfeedLevelSetting::Strong);
        assert!(!app.config.auto_headroom);
        assert_eq!(app.config.dither, DitherSetting::Off);
        // A new app from that config starts with the same settings.
        let cfg = app.config.clone();
        let app2 = GuiApp::with_options(
            &ctx,
            crate::presentation::app::AppOptions {
                persist: false,
                engine: false,
                meta_path: Some(dir.path().join("meta2.db")),
                config: Some(cfg),
                load_fonts: false,
                media_controls: false,
                ..Default::default()
            },
        );
        assert_eq!(app2.crossfeed(), Crossfeed { enabled: true, level: CrossfeedLevel::Strong });
        assert!(!app2.auto_headroom());
        assert_eq!(app2.dither(), DitherMode::Off);
    }

    /// L・R 別: the first switch copies the shared set to R, edits / imports act on the
    /// selected channel, copy L → R / R → L, and the right channel survives a round trip
    /// through linked mode and the config.
    #[test]
    fn peq_left_right_editing_and_copy() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx, dir.path(), false);
        let shared = ParametricEq {
            enabled: true,
            preamp_db: -3.0,
            filters: vec![PeqFilter { kind: FilterKind::Peak, freq_hz: 1000.0, gain_db: 3.0, q: 1.0 }],
            ..Default::default()
        };
        app.set_parametric_eq(shared.clone());
        assert_eq!(app.peq_edit_side(), PeqSide::Left);
        // Linked: selecting R does nothing (the shared set is edited).
        app.set_peq_edit_side(PeqSide::Right);
        assert_eq!(app.peq_edit_side(), PeqSide::Left);
        app.set_peq_channel_mode(PeqChannelMode::Separate);
        assert_eq!(app.peq.channel_mode, PeqChannelMode::Separate);
        assert_eq!(app.peq.channel(PeqSide::Right), app.peq.channel(PeqSide::Left), "R starts as a copy");
        assert_eq!(app.config.parametric_eq.channel_mode, PeqChannelModeSetting::Separate);
        // Import into R only.
        app.set_peq_edit_side(PeqSide::Right);
        assert_eq!(app.peq_edit_side(), PeqSide::Right);
        assert!(app.import_autoeq_text(PROFILE));
        assert_eq!(app.peq.right.filters.len(), 3);
        assert!((app.peq.right.preamp_db + 6.2).abs() < 1e-4);
        assert_eq!(app.peq.filters, shared.filters, "L untouched");
        assert_eq!(app.peq.preamp_db, -3.0);
        // Copy R -> L, then change L and copy L -> R.
        app.copy_peq_channel(PeqSide::Right);
        assert_eq!(app.peq.channel(PeqSide::Left), app.peq.channel(PeqSide::Right));
        let mut p = app.peq.clone();
        p.filters.truncate(1);
        p.preamp_db = -1.0;
        app.set_parametric_eq(p);
        app.copy_peq_channel(PeqSide::Left);
        assert_eq!(app.peq.right.filters.len(), 1);
        assert_eq!(app.peq.right.preamp_db, -1.0);
        // Back to linked: the main (L) set applies, the right set is kept for later.
        let mut p = app.peq.clone();
        p.right.filters.push(PeqFilter { kind: FilterKind::Peak, freq_hz: 4000.0, gain_db: -2.0, q: 2.0 });
        app.set_parametric_eq(p);
        app.set_peq_channel_mode(PeqChannelMode::Linked);
        assert_eq!(app.peq_edit_side(), PeqSide::Left);
        assert_eq!(app.peq.right.filters.len(), 2);
        app.copy_peq_channel(PeqSide::Left);
        assert_eq!(app.peq.right.filters.len(), 2, "copy is a no-op when linked");
        app.set_peq_channel_mode(PeqChannelMode::Separate);
        assert_eq!(app.peq.right.filters.len(), 2, "the saved right channel comes back");
        // A new app from the config gets the same PEQ.
        let app2 = GuiApp::with_options(
            &ctx,
            crate::presentation::app::AppOptions {
                persist: false,
                engine: false,
                meta_path: Some(dir.path().join("meta2.db")),
                config: Some(app.config.clone()),
                load_fonts: false,
                media_controls: false,
                ..Default::default()
            },
        );
        assert_eq!(app2.peq, app.peq);
    }

    /// A config written before L/R and auto-EQ existed loads unchanged (linked, defaults).
    #[test]
    fn old_config_without_new_fields_loads_linked() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let cfg: Config = serde_json::from_str(
            r#"{"resampler_taps": 32, "preferred_output_device": null,
                "parametric_eq": {"enabled": true, "preamp_db": -2.0, "filters": [{"kind": "peak", "freq_hz": 250.0, "gain_db": 1.5, "q": 1.0}]}}"#,
        )
        .unwrap();
        let app = GuiApp::with_options(
            &ctx,
            crate::presentation::app::AppOptions {
                persist: false,
                engine: false,
                meta_path: Some(dir.path().join("meta.db")),
                config: Some(cfg),
                load_fonts: false,
                media_controls: false,
                ..Default::default()
            },
        );
        assert_eq!(app.peq.channel_mode, PeqChannelMode::Linked);
        assert_eq!(app.peq.filters.len(), 1);
        assert_eq!(app.peq.right, PeqChannel::default());
        assert!(app.auto_eq.target.is_none() && app.auto_eq.source.is_none());
        assert_eq!(app.auto_eq_options(), lrc_player_application::infrastructure::audio::engine::AutoEqOptions::default());
    }

    #[test]
    fn exclusive_unavailable_toasts_once_per_reason() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx, dir.path(), false);
        app.config.exclusive_mode = true;
        app.on_exclusive_unavailable("device in use".into());
        let n = app.ui.toasts.len();
        app.on_exclusive_unavailable("device in use".into());
        assert_eq!(app.ui.toasts.len(), n, "same reason: no second toast");
        assert!(!app.exclusive_switch_state(), "switch shows off after a failure");
        assert_eq!(app.exclusive_unavailable_reason(), Some("device in use"));
        app.on_exclusive_unavailable("format not supported".into());
        assert_eq!(app.ui.toasts.len(), n + 1);
        app.on_output_device_changed(true);
        assert_eq!(app.exclusive_unavailable_reason(), None);
        assert_eq!(app.exclusive_switch_state(), EXCLUSIVE_SUPPORTED);
    }
}
