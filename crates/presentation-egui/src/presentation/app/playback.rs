//! Playback control on top of the persistent [`AudioEngine`].
//!
//! The GUI is a thin client: commands go to the engine, the engine reports progress through
//! [`EngineEvent`]s (which wake the UI via the waker) and a cheap per-frame
//! [`AudioEngine::snapshot`] (sample-accurate, latency-compensated position).
//!
//! # Control API for the UI
//!
//! All of these are cheap (they only send engine commands / update in-memory state):
//!
//! | method | effect |
//! |---|---|
//! | [`GuiApp::toggle_play_pause`] | play / pause; starts the queue if nothing is loaded |
//! | [`GuiApp::play`], [`GuiApp::pause`], [`GuiApp::stop`] | transport |
//! | [`GuiApp::next`], [`GuiApp::previous`] | queue navigation (Previous restarts after 3 s) |
//! | [`GuiApp::seek_to`], [`GuiApp::seek_relative`] | debounced (~30 ms) seek |
//! | [`GuiApp::set_volume`], [`GuiApp::toggle_mute`], [`GuiApp::set_muted`] | 0..=1 perceptual volume |
//! | [`GuiApp::set_eq`] | 10-band EQ |
//! | [`GuiApp::set_shuffle`], [`GuiApp::cycle_repeat`], [`GuiApp::set_repeat`] | play order |
//! | [`GuiApp::play_queue_index`], [`GuiApp::enqueue`], [`GuiApp::play_now`] | queue |
//! | [`GuiApp::set_output_device`] | `None` = follow the system default |
//! | [`GuiApp::playback_view`] | read-only state for drawing a transport bar |
//! | [`GuiApp::spectrum_tap`] | live post-gain samples for visualizers |

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;
use lrc_player_application::config::{Config, ResamplerSetting};
use lrc_player_application::dto::{EqDto, RepeatMode, volume_to_linear};
use lrc_player_application::infrastructure::audio::engine::{
    AudioEngine, EngineConfig, EngineEvent, EngineState, EqSettings, ResamplerKind, SpectrumTap,
    EQ_BANDS,
};
use lrc_player_application::usecases::queue_navigation::PreviousAction;
use lrc_player_domain::model::PlaybackState;

use crate::presentation::app::GuiApp;

/// How long the target of a seek is displayed while the engine catches up.
const SEEK_DISPLAY_HOLD: Duration = Duration::from_millis(600);

/// Read-only playback state for drawing transport controls.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackView {
    pub state: PlaybackState,
    pub path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    /// 0..=1 perceptual volume.
    pub volume: f32,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub queue_index: Option<usize>,
    pub queue_len: usize,
    pub output_device: Option<String>,
    pub underruns: u64,
}

pub(crate) fn eq_from_dto(dto: &EqDto) -> EqSettings {
    let mut bands_db = [0.0; EQ_BANDS];
    for (dst, src) in bands_db.iter_mut().zip(dto.bands_db.iter()) {
        *dst = *src;
    }
    EqSettings {
        enabled: dto.enabled,
        preamp_db: dto.preamp_db,
        bands_db,
    }
}

pub(crate) fn eq_to_dto(eq: &EqSettings) -> EqDto {
    let mut dto = EqDto {
        enabled: eq.enabled,
        preamp_db: eq.preamp_db,
        ..Default::default()
    };
    for (dst, src) in dto.bands_db.iter_mut().zip(eq.bands_db.iter()) {
        *dst = *src;
    }
    dto
}

/// The engine's "could not open any output device" error (`EngineEvent::Error` without a
/// path, emitted once per failure streak together with `DeviceChanged { device: None }`).
pub(crate) fn is_output_open_error(message: &str) -> bool {
    message.starts_with("cannot open audio output")
}

pub(crate) fn map_engine_state(state: EngineState) -> PlaybackState {
    match state {
        EngineState::Stopped => PlaybackState::Stopped,
        EngineState::Loading => PlaybackState::Buffering,
        EngineState::Playing => PlaybackState::Playing,
        EngineState::Paused => PlaybackState::Paused,
    }
}

/// Starts the engine (device by name, `None` = system default). Repaints are requested from
/// the engine thread whenever an event is queued.
pub(crate) fn create_engine(
    ctx: &egui::Context,
    cfg: &Config,
    volume: f32,
    muted: bool,
    eq: EqSettings,
) -> Option<AudioEngine> {
    let resampler = match cfg.resampler {
        ResamplerSetting::Fft => ResamplerKind::Fft,
        ResamplerSetting::Sinc => ResamplerKind::Sinc {
            sinc_len: cfg.taps_for_device(cfg.preferred_output_device.as_ref()),
        },
    };
    let engine = match AudioEngine::new(EngineConfig {
        device: cfg.preferred_output_device.clone(),
        resampler,
        volume: volume_to_linear(volume),
        ..Default::default()
    }) {
        Ok(engine) => engine,
        Err(err) => {
            tracing::error!("failed to start the audio engine: {err:#}");
            return None;
        }
    };
    engine.set_muted(muted);
    engine.set_eq(eq);
    let waker_ctx = ctx.clone();
    engine.set_waker(Arc::new(move || waker_ctx.request_repaint()));
    Some(engine)
}

impl GuiApp {
    fn same_path(a: &Path, b: &str) -> bool {
        !b.is_empty() && a == Path::new(b)
    }

    /// True when the engine has `play_path` loaded (playing, paused or loading).
    pub(crate) fn engine_has_current_track(&self) -> bool {
        self.engine_snapshot.as_ref().is_some_and(|snap| {
            snap.state != EngineState::Stopped
                && snap
                    .path
                    .as_deref()
                    .is_some_and(|p| Self::same_path(p, &self.play_path))
        })
    }

    // ------------------------------------------------------------------------------------
    // Per-frame polling
    // ------------------------------------------------------------------------------------

    /// Drains engine events and refreshes state / position from the engine snapshot.
    pub(crate) fn poll_engine(&mut self) {
        let mut events = Vec::new();
        if let Some(engine) = &self.engine {
            while let Some(ev) = engine.try_recv_event() {
                events.push(ev);
            }
        }
        for ev in events {
            self.handle_engine_event(ev);
        }

        let Some(engine) = &self.engine else {
            self.engine_snapshot = None;
            self.flush_pending_seek();
            return;
        };
        let snap = engine.snapshot();
        let is_current = snap
            .path
            .as_deref()
            .is_some_and(|p| Self::same_path(p, &self.play_path));
        if snap.state != EngineState::Stopped || is_current {
            self.playback_state = map_engine_state(snap.state);
        } else {
            self.playback_state = PlaybackState::Stopped;
        }
        if snap.state == EngineState::Playing {
            self.consecutive_errors = 0;
            self.pending_error_skip = false;
        }
        let skip_after_error = self.pending_error_skip && snap.state == EngineState::Stopped;
        if is_current && snap.state != EngineState::Stopped {
            if let Some(d) = snap.duration_ms {
                self.play_duration_ms = Some(d);
            }
            let showing_seek_target = self.pending_seek_ms.is_some();
            if !showing_seek_target {
                match self.seek_display_hold {
                    Some((target, at))
                        if at.elapsed() < SEEK_DISPLAY_HOLD
                            && snap.position_ms.abs_diff(target) > 750 =>
                    {
                        self.play_position_ms = target;
                    }
                    _ => {
                        self.seek_display_hold = None;
                        self.play_position_ms = snap.position_ms;
                    }
                }
            }
        }
        if let Some(device) = &snap.device {
            self.no_output_device = false;
            self.active_output_device = Some(device.name.clone());
        }
        self.engine_snapshot = Some(snap);
        if skip_after_error {
            self.pending_error_skip = false;
            self.consecutive_errors += 1;
            let len = self.queue.len();
            if let Some(index) = self.navigator.next_manual(self.current_queue_index, len) {
                if Some(index) != self.current_queue_index {
                    let tr = self.tr();
                    self.toast_warning(tr.t("toast.playback_skip").to_string());
                    self.start_queue_index(index, true);
                }
            }
        }
        self.flush_pending_seek();
    }

    fn handle_engine_event(&mut self, ev: EngineEvent) {
        match ev {
            EngineEvent::TrackStarted { path, duration_ms } => {
                if Self::same_path(&path, &self.play_path) {
                    if duration_ms.is_some() {
                        self.play_duration_ms = duration_ms;
                    }
                    self.pending_next = None;
                    self.update_gapless_next();
                }
            }
            EngineEvent::AdvancedToNext { path, duration_ms } => {
                let path_str = path.to_string_lossy().into_owned();
                let index = match self.pending_next.take() {
                    Some((index, next)) if next == path_str => Some(index),
                    _ => self.queue.iter().position(|t| Path::new(t) == path),
                };
                self.on_track_changed(index, path_str);
                self.play_position_ms = 0;
                self.play_duration_ms = duration_ms;
                self.update_gapless_next();
            }
            EngineEvent::TrackFinished { path } => {
                if !Self::same_path(&path, &self.play_path) {
                    return;
                }
                // Nothing was queued for gapless playback; follow the rules once more in case
                // the queue changed right at the boundary, otherwise stop at the end.
                let len = self.queue.len();
                let next = self.navigator.next_auto(self.current_queue_index, len);
                match next {
                    Some(index) if self.current_queue_index.is_some() => {
                        self.start_queue_index(index, true);
                    }
                    _ => {
                        self.playback_state = PlaybackState::Stopped;
                        self.play_position_ms = 0;
                        self.save_state();
                    }
                }
            }
            EngineEvent::Error { path: None, message } if is_output_open_error(&message) => {
                // No device (e.g. a machine without audio hardware): a quiet indicator in the
                // transport bar / Settings instead of an error toast at startup. A toast is
                // shown only when the user is trying to play.
                tracing::info!("no audio output: {message}");
                let was_missing = self.no_output_device;
                self.no_output_device = true;
                let trying_to_play = matches!(self.playback_state, PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking);
                if trying_to_play && !was_missing {
                    self.warn_if_no_output_device();
                }
            }
            EngineEvent::Error { path, message } => {
                tracing::warn!("audio engine error ({:?}): {}", path, message);
                let seek_failure = message.starts_with("seek failed");
                let tr = self.tr();
                self.toast_error(tr.f("toast.playback_error", &[&message]));
                let Some(path) = path else {
                    return;
                };
                let path_str = path.to_string_lossy().into_owned();
                let len = self.queue.len();
                if self.consecutive_errors >= len.max(1) {
                    return;
                }
                if self.pending_next.as_ref().is_some_and(|(_, p)| *p == path_str) {
                    // The gapless successor cannot be opened: preload the one after it.
                    let (failed_index, _) = self.pending_next.take().expect("checked above");
                    self.consecutive_errors += 1;
                    if let Some(index) = self.navigator.next_auto(Some(failed_index), len) {
                        if Some(index) != self.current_queue_index {
                            if let (Some(engine), Some(next)) =
                                (&self.engine, self.queue.get(index).cloned())
                            {
                                engine.set_next(Some(PathBuf::from(&next)));
                                self.pending_next = Some((index, next));
                            }
                        }
                    }
                } else if path_str == self.play_path && !seek_failure {
                    // Possibly a load failure of the current track: once the engine reports
                    // Stopped (see `poll_engine`), skip to the next queued track.
                    self.pending_error_skip = true;
                }
            }
            EngineEvent::DeviceChanged { device } => {
                self.no_output_device = device.is_none();
                self.active_output_device = device.map(|d| d.name);
            }
            EngineEvent::StateChanged { state } => {
                self.playback_state = map_engine_state(state);
            }
        }
    }

    /// Tells the engine which track follows the current one (gapless), honouring
    /// shuffle/repeat. Cheap; only sends a command when the answer changed.
    pub(crate) fn update_gapless_next(&mut self) {
        let Some(engine) = &self.engine else {
            return;
        };
        let active = self
            .engine_snapshot
            .as_ref()
            .is_none_or(|s| s.state != EngineState::Stopped);
        let len = self.queue.len();
        let next = if active && self.current_queue_index.is_some() {
            self.navigator
                .next_auto(self.current_queue_index, len)
                .and_then(|i| self.queue.get(i).map(|p| (i, p.clone())))
        } else {
            None
        };
        if next != self.pending_next {
            engine.set_next(next.as_ref().map(|(_, p)| PathBuf::from(p)));
            self.pending_next = next;
        }
    }

    // ------------------------------------------------------------------------------------
    // Track changes
    // ------------------------------------------------------------------------------------

    /// Makes `path` the current track in the UI (lyrics, metadata, artwork, offset) without
    /// touching the engine.
    pub(crate) fn on_track_changed(&mut self, index: Option<usize>, path: String) {
        let changed = self.play_path != path;
        self.current_queue_index = index;
        self.play_path = path;
        if changed {
            self.play_duration_ms = None;
            self.lyrics_offset_ms = self.lyrics_offsets.get(&self.play_path).copied().unwrap_or(0);
            match self.library.row_index(std::path::Path::new(&self.play_path)) {
                Some(i) => {
                    let row = &self.library.rows[i];
                    self.play_title = Some(row.title.clone());
                    self.play_artist = Some(row.artist.clone()).filter(|a| !a.is_empty());
                    self.play_album = Some(row.album.clone()).filter(|a| !a.is_empty());
                    self.play_duration_ms = row.duration_ms;
                }
                None => {
                    self.play_title = None;
                    self.play_artist = None;
                    self.play_album = None;
                }
            }
            self.play_artwork_requested_for = None;
            self.play_cover_path = None;
            self.spectrum.reset();
        }
        // Probe (duration/tags) + lyrics discovery run off the UI thread.
        self.request_track_info();
        self.save_state();
    }

    /// Loads queue item `index` into the engine (playing when `autoplay`).
    pub(crate) fn start_queue_index(&mut self, index: usize, autoplay: bool) {
        let Some(path) = self.queue.get(index).cloned() else {
            return;
        };
        self.start_track(Some(index), path, 0, autoplay);
    }

    /// Loads `path` into the engine at `start_ms`.
    pub(crate) fn start_track(&mut self, index: Option<usize>, path: String, start_ms: u64, autoplay: bool) {
        if path.trim().is_empty() {
            return;
        }
        self.on_track_changed(index, path);
        self.play_position_ms = start_ms;
        self.pending_seek_ms = None;
        self.seek_display_hold = None;
        self.load_current_into_engine(autoplay);
    }

    fn load_current_into_engine(&mut self, autoplay: bool) {
        let Some(engine) = &self.engine else {
            let tr = self.tr();
            self.toast_error(tr.t("toast.no_engine").to_string());
            return;
        };
        engine.load(PathBuf::from(&self.play_path), self.play_position_ms, autoplay);
        if autoplay {
            self.warn_if_no_output_device();
        }
        // `load` clears the engine's queued next track.
        self.pending_next = None;
        self.playback_state = if autoplay {
            PlaybackState::Buffering
        } else {
            PlaybackState::Paused
        };
    }

    // ------------------------------------------------------------------------------------
    // Public control API
    // ------------------------------------------------------------------------------------

    /// The user asked to play while no output device is open: say so once per attempt
    /// (playback starts by itself when a device appears).
    fn warn_if_no_output_device(&mut self) {
        if self.no_output_device {
            let tr = self.tr();
            self.toast_warning(tr.t("toast.no_output_device").to_string());
        }
    }

    /// Play / pause. With nothing loaded, starts the current (or first) queue item.
    pub fn toggle_play_pause(&mut self) {
        match self.playback_state {
            PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => self.pause(),
            PlaybackState::Paused | PlaybackState::Stopped => self.play(),
        }
    }

    /// Starts or resumes playback of the current track (from the stored position after a
    /// restart), or the current/selected/first queue item when there is no current track.
    pub fn play(&mut self) {
        if self.play_path.trim().is_empty() {
            let len = self.queue.len();
            if len == 0 {
                let tr = self.tr();
                self.toast_info(tr.t("toast.nothing_to_play").to_string());
                return;
            }
            let index = self
                .current_queue_index
                .filter(|&i| i < len)
                .or_else(|| self.navigator.next_manual(None, len))
                .unwrap_or(0);
            self.start_queue_index(index, true);
            return;
        }
        if self.engine_has_current_track() {
            self.flush_seek_now();
            if let Some(engine) = &self.engine {
                engine.play();
            }
            self.playback_state = PlaybackState::Playing;
            self.warn_if_no_output_device();
            return;
        }
        if let Some(target) = self.pending_seek_ms.take() {
            self.play_position_ms = target;
        }
        if let Some(d) = self.play_duration_ms {
            if self.play_position_ms >= d {
                self.play_position_ms = 0;
            }
        }
        self.load_current_into_engine(true);
    }

    /// Pauses (the engine fades out and keeps its buffer; resume is instant).
    pub fn pause(&mut self) {
        self.flush_seek_now();
        if let Some(engine) = &self.engine {
            engine.pause();
        }
        if self.playback_state != PlaybackState::Stopped {
            self.playback_state = PlaybackState::Paused;
        }
        self.save_state();
    }

    /// Stops playback and rewinds to 0 (the track stays selected).
    pub fn stop(&mut self) {
        if let Some(engine) = &self.engine {
            engine.stop();
        }
        self.pending_seek_ms = None;
        self.seek_display_hold = None;
        self.pending_next = None;
        self.playback_state = PlaybackState::Stopped;
        self.play_position_ms = 0;
        self.save_state();
    }

    /// Next queue item (wraps when repeat is on; repeat-one does not pin the Next button).
    pub fn next(&mut self) {
        let len = self.queue.len();
        if let Some(index) = self.navigator.next_manual(self.current_queue_index, len) {
            let autoplay = self.playback_state != PlaybackState::Paused;
            self.start_queue_index(index, autoplay);
        }
    }

    /// Restarts the current track when more than 3 s in, otherwise goes to the previous item.
    pub fn previous(&mut self) {
        let len = self.queue.len();
        match self
            .navigator
            .previous(self.current_queue_index, len, self.play_position_ms)
        {
            PreviousAction::RestartCurrent => self.seek_to(0),
            PreviousAction::PlayIndex(index) => {
                let autoplay = self.playback_state != PlaybackState::Paused;
                self.start_queue_index(index, autoplay);
            }
        }
    }

    /// Plays queue item `index`.
    pub fn play_queue_index(&mut self, index: usize) {
        self.start_queue_index(index, true);
    }

    /// Appends tracks to the end of the queue.
    pub fn enqueue(&mut self, paths: Vec<String>) {
        let paths: Vec<String> = paths.into_iter().filter(|p| !p.trim().is_empty()).collect();
        if paths.is_empty() {
            return;
        }
        self.queue.extend(paths);
        self.on_queue_changed();
    }

    /// Replaces the queue with `paths` and plays the first one.
    pub fn play_now(&mut self, paths: Vec<String>) {
        self.play_tracks(paths, 0);
    }

    /// 0..=1 perceptual volume (cubic taper to linear gain).
    pub fn set_volume(&mut self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        if (volume - self.volume).abs() < f32::EPSILON {
            return;
        }
        self.volume = volume;
        if let Some(engine) = &self.engine {
            engine.set_volume(volume_to_linear(volume));
        }
        self.save_state();
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if let Some(engine) = &self.engine {
            engine.set_muted(muted);
        }
        self.save_state();
    }

    pub fn toggle_mute(&mut self) {
        self.set_muted(!self.muted);
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn set_eq(&mut self, eq: EqSettings) {
        if eq == self.eq {
            return;
        }
        self.eq = eq;
        if let Some(engine) = &self.engine {
            engine.set_eq(eq);
        }
        self.save_state();
    }

    pub fn eq(&self) -> EqSettings {
        self.eq
    }

    pub fn set_shuffle(&mut self, on: bool) {
        let len = self.queue.len();
        self.navigator.set_shuffle(on, self.current_queue_index, len);
        self.update_gapless_next();
        self.save_state();
    }

    pub fn shuffle(&self) -> bool {
        self.navigator.shuffle()
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.navigator.set_repeat(repeat);
        self.update_gapless_next();
        self.save_state();
    }

    /// Off -> All -> One -> Off.
    pub fn cycle_repeat(&mut self) {
        let next = self.navigator.repeat().cycle();
        self.set_repeat(next);
    }

    pub fn repeat(&self) -> RepeatMode {
        self.navigator.repeat()
    }

    /// Switches the output device (`None` follows the system default) and remembers it.
    pub fn set_output_device(&mut self, name: Option<String>) {
        if name == self.preferred_output_device {
            return;
        }
        self.preferred_output_device = name.clone();
        if let Some(engine) = &self.engine {
            engine.set_output_device(name.clone());
        }
        self.config.preferred_output_device = name;
        self.save_config();
    }

    /// Re-enumerates output devices (cheap enough for a combo-box open, not per frame).
    pub fn refresh_output_devices(&mut self) {
        if self.engine.is_some() {
            self.device_names = AudioEngine::output_device_names();
        }
    }

    /// Live post-gain mono samples of what is being played (for visualizers).
    pub fn spectrum_tap(&self) -> Option<SpectrumTap> {
        self.engine.as_ref().map(|e| e.spectrum_tap())
    }

    /// Snapshot of transport state for drawing.
    pub fn playback_view(&self) -> PlaybackView {
        PlaybackView {
            state: self.playback_state,
            path: self.play_path.clone(),
            title: self.play_title.clone(),
            artist: self.play_artist.clone(),
            album: self.play_album.clone(),
            position_ms: self.play_position_ms,
            duration_ms: self.play_duration_ms,
            volume: self.volume,
            muted: self.muted,
            shuffle: self.navigator.shuffle(),
            repeat: self.navigator.repeat(),
            queue_index: self.current_queue_index,
            queue_len: self.queue.len(),
            output_device: self.active_output_device.clone(),
            underruns: self.engine_snapshot.as_ref().map(|s| s.underruns).unwrap_or(0),
        }
    }

    /// Clears the current track and stops the engine.
    pub(crate) fn clear_current_track(&mut self) {
        self.stop();
        self.play_path.clear();
        self.current_queue_index = None;
        self.play_duration_ms = None;
        self.play_title = None;
        self.play_artist = None;
        self.play_album = None;
        self.play_artwork_handle = None;
        self.play_backdrop_handle = None;
        self.play_artwork_requested_for = None;
        self.play_cover_path = None;
        self.lyrics_candidates.clear();
        self.loaded_timeline = None;
        self.lyrics_revision += 1;
        self.selected_lyrics_index = None;
        self.lyrics_offset_ms = 0;
        self.save_state();
    }

    /// Final state save + engine shutdown (idempotent).
    pub(crate) fn shutdown(&mut self) {
        if let Some(snap) = self.engine.as_ref().map(|e| e.snapshot()) {
            if snap.state != EngineState::Stopped
                && snap
                    .path
                    .as_deref()
                    .is_some_and(|p| Self::same_path(p, &self.play_path))
            {
                self.play_position_ms = snap.position_ms;
            }
        }
        self.flush_state();
        // Dropping the engine fades out and joins its thread.
        self.engine = None;
    }
}
