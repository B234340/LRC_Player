//! Persistent playback engine.
//!
//! One long-lived engine thread owns the decoder (symphonia), the converter (channel mapping +
//! rubato resampling) and the producer side of a lock-free SPSC ring; the audio callback owns a
//! [`renderer::Renderer`] that pops from the ring and applies fades, volume, EQ and limiting.
//! The output stream stays open across tracks, pauses and seeks; it is only rebuilt on device
//! changes/errors (the current track resumes at the audible position).
//!
//! ```no_run
//! use lrc_player_adapters_audio::audio::engine::{AudioEngine, EngineConfig, EngineEvent};
//! let engine = AudioEngine::new(EngineConfig::default()).unwrap();
//! engine.load("song.flac".into(), 0, true);
//! loop {
//!     while let Some(ev) = engine.try_recv_event() {
//!         if let EngineEvent::TrackFinished { .. } = ev { return; }
//!     }
//!     let snap = engine.snapshot();
//!     println!("{:?} {} ms", snap.state, snap.position_ms);
//!     std::thread::sleep(std::time::Duration::from_millis(100));
//! }
//! ```

pub(crate) mod channel_map;
mod convert;
mod dsp;
mod engine_thread;
mod output;
pub(crate) mod priority;
mod renderer;
pub(crate) mod source;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};

pub use convert::ResamplerKind;
pub use dsp::{EqSettings, EQ_BANDS, EQ_FREQUENCIES_HZ, EQ_MAX_GAIN_DB};
pub use renderer::SpectrumTap;
pub use source::{probe_track, TrackInfo};

use engine_thread::{Command, EngineThread, EntryInfo, SharedInfo};
use renderer::{PositionGuard, Renderer, RtShared};

/// Where the engine sends audio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputMode {
    /// A cpal output stream (WASAPI / CoreAudio / ALSA-Pulse).
    Cpal,
    /// No device: the caller drives rendering via [`AudioEngine::render_manual`]
    /// (tests, offline rendering, headless use).
    Manual { sample_rate: u32, channels: u16 },
}

/// Engine configuration. `Default` is tuned for robustness under CPU load.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub output: OutputMode,
    /// Initial output device by name; `None` follows the system default device.
    pub device: Option<String>,
    /// Requested device buffer (cpal `BufferSize::Fixed`), clamped to the device range.
    pub buffer_ms: u32,
    /// Size of the decoder -> callback ring.
    pub ring_ms: u32,
    /// Audio that must be buffered before (re)starting output after load/seek/underrun.
    pub prebuffer_ms: u32,
    /// Fade length for pause/resume/seek/stop/track switch/underrun.
    pub fade_ms: u32,
    pub resampler: ResamplerKind,
    /// Initial linear volume (0..=2; >1 is gain with soft limiting).
    pub volume: f32,
    /// How often the engine thread checks for default-device changes / device reappearance.
    pub device_poll_ms: u32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            output: OutputMode::Cpal,
            device: None,
            buffer_ms: 50,
            ring_ms: 1000,
            prebuffer_ms: 200,
            fade_ms: 10,
            resampler: ResamplerKind::Fft,
            volume: 1.0,
            device_poll_ms: 1000,
        }
    }
}

/// Transport state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EngineState {
    #[default]
    Stopped,
    /// A track was loaded/seeked-from-stop and audio is being prebuffered.
    Loading,
    Playing,
    Paused,
}

/// The output device currently in use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
    /// Fixed buffer size requested from the device (None = device default).
    pub buffer_frames: Option<u32>,
    /// True when the engine follows the system default device.
    pub follows_default: bool,
}

/// Cheap, consistent view of the engine for per-frame UI use.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineSnapshot {
    pub state: EngineState,
    /// Track that is audible now (switches exactly at gapless boundaries).
    pub path: Option<PathBuf>,
    /// Audible position (latency compensated, interpolated between callbacks).
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub device: Option<DeviceInfo>,
    pub underruns: u64,
    pub volume: f32,
    pub muted: bool,
}

/// Notifications from the engine.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineEvent {
    /// `load()` accepted the file (duration from the container, no decode).
    TrackStarted {
        path: PathBuf,
        duration_ms: Option<u64>,
    },
    /// Playback crossed a gapless boundary into the track given by `set_next()`.
    AdvancedToNext {
        path: PathBuf,
        duration_ms: Option<u64>,
    },
    /// The last sample of the track was played and no next track was queued.
    /// (Not sent for gapless advances.)
    TrackFinished {
        path: PathBuf,
    },
    Error {
        path: Option<PathBuf>,
        message: String,
    },
    /// The output device was (re)opened, or lost (`None`).
    DeviceChanged {
        device: Option<DeviceInfo>,
    },
    StateChanged {
        state: EngineState,
    },
}

/// Callback invoked (on the engine thread) whenever an event is queued.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

pub(crate) struct EventSink {
    tx: Sender<EngineEvent>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl EventSink {
    pub(crate) fn send(&self, ev: EngineEvent) {
        let _ = self.tx.send(ev);
        let waker = self.waker.lock().ok().and_then(|w| w.clone());
        if let Some(w) = waker {
            w();
        }
    }
}

/// Handle to the playback engine. `Send + Sync`; dropping it stops audio (with a short fade)
/// and joins the engine thread.
pub struct AudioEngine {
    cmd_tx: Sender<Command>,
    events_rx: Receiver<EngineEvent>,
    rt: Arc<RtShared>,
    info: Arc<Mutex<SharedInfo>>,
    waker: Arc<Mutex<Option<Waker>>>,
    manual: Arc<Mutex<Option<Renderer>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    /// Keeps `snapshot().position_ms` monotonic within a segment.
    pos_guard: Mutex<PositionGuard>,
}

impl AudioEngine {
    /// Spawns the engine thread and opens the output. A missing device is not an error: an
    /// `Error`/`DeviceChanged{None}` event is emitted and the engine retries periodically.
    pub fn new(cfg: EngineConfig) -> Result<Self> {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let (ev_tx, events_rx) = crossbeam_channel::unbounded();
        let rt = Arc::new(RtShared::new(cfg.volume));
        rt.set_volume(cfg.volume);
        let info = Arc::new(Mutex::new(SharedInfo::default()));
        let waker: Arc<Mutex<Option<Waker>>> = Arc::new(Mutex::new(None));
        let manual = Arc::new(Mutex::new(None));
        let sink = EventSink {
            tx: ev_tx,
            waker: waker.clone(),
        };
        let (rt2, info2, manual2) = (rt.clone(), info.clone(), manual.clone());
        // The engine thread owns the (!Send) cpal stream, so it is constructed on that thread.
        let handle = std::thread::Builder::new()
            .name("lrc-audio-engine".into())
            .spawn(move || EngineThread::new(cfg, cmd_rx, sink, rt2, info2, manual2).run())?;
        Ok(Self {
            cmd_tx,
            events_rx,
            rt,
            info,
            waker,
            manual,
            thread: Mutex::new(Some(handle)),
            pos_guard: Mutex::new(PositionGuard::default()),
        })
    }

    fn send(&self, cmd: Command) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Loads `path` and starts at `start_ms` (playing if `autoplay`, else paused once
    /// prebuffered). Clears any queued next track.
    pub fn load(&self, path: PathBuf, start_ms: u64, autoplay: bool) {
        self.send(Command::Load {
            path,
            start_ms,
            autoplay,
        });
    }

    /// Queues the track to splice in gaplessly when the current one ends (`None` clears).
    /// Call again after every `AdvancedToNext`/`TrackStarted` with the following track.
    ///
    /// The successor is spliced into the ring when the current track finishes *decoding*,
    /// up to `ring_ms` before it finishes playing. From then on it is committed: calling
    /// `set_next` again with the same path is a no-op (it will not play twice), and a
    /// different path (or `None`) only affects what follows the spliced track. A `set_next`
    /// arriving after the current track has already finished playing starts the next track
    /// after a short prebuffer instead of gaplessly.
    pub fn set_next(&self, path: Option<PathBuf>) {
        self.send(Command::SetNext(path));
    }

    pub fn play(&self) {
        self.send(Command::Play);
    }

    pub fn pause(&self) {
        self.send(Command::Pause);
    }

    pub fn stop(&self) {
        self.send(Command::Stop);
    }

    /// Sample-accurate seek within the audible track (container seek index, no decode from
    /// the start). Rapid successive seeks are coalesced. Seeking past the end finishes the
    /// track (`TrackFinished`). If the file cannot be repositioned at all (e.g. it vanished),
    /// an `Error` is emitted and the engine stops. From `Stopped`, loads the last track
    /// paused at `ms`.
    pub fn seek(&self, ms: u64) {
        self.send(Command::Seek(ms));
    }

    /// Linear volume 0..=2 (smoothed in the callback, applied immediately).
    pub fn set_volume(&self, linear: f32) {
        self.rt.set_volume(linear);
    }

    pub fn set_muted(&self, muted: bool) {
        self.rt.muted.store(muted, Ordering::Relaxed);
    }

    pub fn set_eq(&self, eq: EqSettings) {
        self.send(Command::SetEq(eq));
    }

    /// `None` follows the system default device (changes are picked up automatically).
    pub fn set_output_device(&self, name: Option<String>) {
        self.send(Command::SetDevice(name));
    }

    /// `position_ms` never decreases while the same track segment plays (it may hold still for
    /// the output latency right after a resume); it changes discontinuously only on
    /// load/seek/stop/track change.
    pub fn snapshot(&self) -> EngineSnapshot {
        let clock = self.rt.clock.read();
        let now = self.rt.now_ns();
        let underruns = self.rt.underruns.load(Ordering::Relaxed);
        let volume = self.rt.volume();
        let muted = self.rt.muted.load(Ordering::Relaxed);
        let info = match self.info.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let mut segment = None;
        let (path, position_ms, duration_ms) = if info.state == EngineState::Stopped {
            match &info.last {
                Some(e) => (Some(e.path.clone()), 0, e.duration_ms),
                None => (None, 0, None),
            }
        } else {
            let live = info
                .entries
                .iter()
                .find(|e| e.seq == clock.track_seq && clock.gen >= e.gen);
            match (live, info.entries.first()) {
                (Some(e), _) => {
                    segment = Some((clock.gen, clock.track_seq));
                    (Some(e.path.clone()), clock.audible_ms(now), e.duration_ms)
                }
                (None, Some(e)) => (Some(e.path.clone()), e.base_ms, e.duration_ms),
                (None, None) => (None, 0, None),
            }
        };
        // Lock order: info -> pos_guard (pos_guard is only ever locked here).
        let position_ms = match self.pos_guard.lock() {
            Ok(mut g) => g.apply(segment, position_ms),
            Err(p) => p.into_inner().apply(segment, position_ms),
        };
        let position_ms = match duration_ms {
            Some(d) => position_ms.min(d),
            None => position_ms,
        };
        EngineSnapshot {
            state: info.state,
            path,
            position_ms,
            duration_ms,
            device: info.device.clone(),
            underruns,
            volume,
            muted,
        }
    }

    pub fn try_recv_event(&self) -> Option<EngineEvent> {
        self.events_rx.try_recv().ok()
    }

    /// Blocks up to `timeout` for the next event.
    pub fn recv_event_timeout(&self, timeout: Duration) -> Option<EngineEvent> {
        self.events_rx.recv_timeout(timeout).ok()
    }

    /// Called (from the engine thread) after each queued event, e.g. `ctx.request_repaint()`.
    pub fn set_waker(&self, f: Waker) {
        if let Ok(mut w) = self.waker.lock() {
            *w = Some(f);
        }
    }

    /// Lock-free tap of the most recent post-gain mono output samples.
    pub fn spectrum_tap(&self) -> SpectrumTap {
        self.rt.tap.clone()
    }

    pub fn output_device_names() -> Vec<String> {
        output::output_device_names()
    }

    pub fn default_output_device_name() -> Option<String> {
        output::default_output_device_name()
    }

    /// For [`OutputMode::Manual`]: renders the next `out.len()` interleaved samples.
    /// Returns false if no manual output is active.
    pub fn render_manual(&self, out: &mut [f32]) -> bool {
        let mut slot = match self.manual.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        match slot.as_mut() {
            Some(r) => {
                r.render(out, 0);
                true
            }
            None => false,
        }
    }

    /// Samples currently buffered for the manual renderer (tests use it to pace rendering).
    #[cfg(test)]
    pub(crate) fn manual_available(&self) -> Option<usize> {
        self.manual.lock().ok()?.as_ref().map(|r| r.available())
    }

    /// Waits until the engine thread handled (and published) every command sent so far.
    #[cfg(test)]
    pub(crate) fn sync(&self) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.send(Command::Sync(tx));
        rx.recv_timeout(Duration::from_secs(10))
            .expect("engine thread did not answer");
    }

    #[cfg(test)]
    pub(crate) fn rt_weak(&self) -> std::sync::Weak<RtShared> {
        Arc::downgrade(&self.rt)
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(Command::Shutdown);
        let handle = match self.thread.lock() {
            Ok(mut g) => g.take(),
            Err(p) => p.into_inner().take(),
        };
        if let Some(h) = handle {
            // Dropped from the engine thread itself (e.g. the last owner released inside the
            // waker callback): joining would deadlock/panic. The thread exits on its own
            // after processing the Shutdown command.
            if h.thread().id() != std::thread::current().id() {
                let _ = h.join();
            }
        }
        if let Ok(mut m) = self.manual.lock() {
            m.take();
        }
    }
}

impl From<&engine_thread::Entry> for EntryInfo {
    fn from(e: &engine_thread::Entry) -> Self {
        EntryInfo {
            seq: e.seq,
            gen: e.gen,
            path: e.path.clone(),
            duration_ms: e.duration_ms,
            base_ms: e.base_ms,
        }
    }
}
