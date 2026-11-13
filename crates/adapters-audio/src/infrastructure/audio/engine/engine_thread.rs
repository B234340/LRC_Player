//! The engine/control thread: command handling, decoding into the ring, gapless splicing,
//! state/event bookkeeping and device management. Never runs on the realtime thread.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};

use super::convert::Converter;
use super::crossfeed::CrossfeedCoeffs;
use super::device_watch::{DeviceWatch, PresentPolicy};
use super::dither::{DitherControl, DitherMode};
use super::dsp::{auto_headroom_db, db_to_linear, EqCoeffs, EqSettings};
use super::normalization::{resolve_gain, TrackGain};
use super::output;
use super::output_wasapi::{self, ExclusivePolicy, Plan};
use super::peq::PeqCoeffs;
use super::priority;
use super::renderer::{new_pair, Feeder, Marker, RenderConfig, Renderer, RtCommand, RtShared};
use super::source::{TrackMeta, TrackSource};
use super::{Crossfeed, LoudnessHint, Normalization, ParametricEq};
use super::{DeviceInfo, EngineConfig, EngineEvent, EngineState, EventSink, OutputMode};

pub(crate) enum Command {
    Load {
        path: PathBuf,
        start_ms: u64,
        autoplay: bool,
    },
    SetNext(Option<PathBuf>),
    Play,
    Pause,
    Stop,
    Seek(u64),
    SetEq(EqSettings),
    SetNormalization(Normalization),
    SetLoudnessHint(PathBuf, Option<LoudnessHint>),
    SetParametricEq(ParametricEq),
    SetCrossfeed(Crossfeed),
    SetAutoHeadroom(bool),
    SetDevice(Option<String>),
    SetExclusive(bool),
    SetDither(DitherMode),
    Shutdown,
    /// Test hook: answered once every earlier command was handled and published.
    #[cfg(test)]
    Sync(Sender<()>),
}

/// Snapshot data published for [`super::AudioEngine::snapshot`].
#[derive(Clone, Debug, Default)]
pub(crate) struct SharedInfo {
    pub(crate) state: EngineState,
    pub(crate) entries: Vec<EntryInfo>,
    pub(crate) last: Option<EntryInfo>,
    pub(crate) device: Option<DeviceInfo>,
    pub(crate) dsp: DspInfo,
}

/// DSP state published for the snapshot's [`super::SignalPath`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DspInfo {
    pub(crate) eq: bool,
    pub(crate) peq: bool,
    pub(crate) crossfeed: Option<super::CrossfeedLevel>,
    pub(crate) headroom_db: f32,
    pub(crate) resampler: super::ResamplerKind,
}

#[derive(Clone, Debug)]
pub(crate) struct EntryInfo {
    pub(crate) seq: u64,
    pub(crate) gen: u64,
    pub(crate) path: PathBuf,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) base_ms: u64,
    pub(crate) meta: TrackMeta,
    pub(crate) gain: TrackGain,
}

/// A track that has been (or is being) written into the ring, in ring order.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub(crate) seq: u64,
    /// Generation of the segment this entry was started in.
    pub(crate) gen: u64,
    pub(crate) path: PathBuf,
    pub(crate) duration_ms: Option<u64>,
    /// Track position where the current segment starts (load/seek target).
    pub(crate) base_ms: u64,
    pub(crate) meta: TrackMeta,
    /// Normalization gain (attached to the entry so it switches at its gapless boundary).
    pub(crate) gain: TrackGain,
}

struct Decoding {
    src: TrackSource,
    conv: Option<Converter>,
    seq: u64,
    /// Ring sample index where `conv`'s first output sample landed.
    origin: u64,
}

enum Backend {
    Cpal(#[allow(dead_code)] cpal::Stream),
    /// WASAPI exclusive stream (see `output_wasapi`).
    #[cfg(windows)]
    Wasapi(output_wasapi::ExclusiveStream),
    Manual,
}

struct Output {
    feeder: Feeder,
    info: DeviceInfo,
    backend: Backend,
}

pub(crate) struct EngineThread {
    cfg: EngineConfig,
    cmd_rx: Receiver<Command>,
    events: EventSink,
    rt: Arc<RtShared>,
    info: Arc<Mutex<SharedInfo>>,
    manual_slot: Arc<Mutex<Option<Renderer>>>,
    err_tx: Sender<String>,
    err_rx: Receiver<String>,

    output: Option<Output>,
    device_pref: Option<String>,
    device_error_reported: bool,
    eq: EqSettings,
    norm: Normalization,
    /// Loudness hints from the application (bounded, see `MAX_HINTS`).
    hints: HashMap<PathBuf, LoudnessHint>,
    peq: ParametricEq,
    xfeed: Crossfeed,
    auto_headroom: bool,
    /// Auto-headroom gain currently applied (dB, <= 0).
    headroom_db: f32,
    /// DSP state for the snapshot, refreshed whenever DSP commands are built.
    dsp_info: DspInfo,
    state: EngineState,
    paused: bool,
    gen: u64,
    seq: u64,
    entries: VecDeque<Entry>,
    dec: Option<Decoding>,
    next_path: Option<PathBuf>,
    next_src: Option<TrackSource>,
    /// Ring index of the `End` marker of the last track (no next queued).
    end_index: Option<u64>,
    last: Option<Entry>,
    scratch: Vec<f32>,
    /// Schedule of device probes (retry backoff while no device, periodic checks).
    watch: DeviceWatch,
    last_callbacks: (u64, Instant),
    /// A segment was started and Loading -> Playing/Paused waits for the renderer to prime it.
    awaiting_prime: bool,
    /// A renderer queue overflowed (callbacks stalled while commands kept coming): the
    /// renderer has lost a command/marker and must be rebuilt to resynchronise.
    needs_rebuild: bool,
    /// Dither mode switch shared with the output streams.
    dither: DitherControl,
    /// Exclusive-mode fallback bookkeeping.
    excl: ExclusivePolicy,
    /// Source rate of the audible track: the rate an exclusive output is opened at.
    rate_hint: Option<u32>,
    /// Exclusive output only: the gapless successor runs at another rate, so instead of being
    /// spliced it starts after the current track ended and the output was reopened at its rate.
    rate_switch_next: Option<TrackSource>,
    #[cfg(test)]
    sync_waiters: Vec<Sender<()>>,
}

const STALL_TIMEOUT: Duration = Duration::from_secs(3);
/// Upper bound for positions coming from the API (~2 years). Keeps every ms <-> frame <->
/// container-timestamp conversion far from u64 overflow (a debug-build panic would kill the
/// engine thread; a release build would wrap to garbage). Larger values simply mean "past
/// the end".
const MAX_POSITION_MS: u64 = 1 << 36;
const MAX_PUMP_SLICE: Duration = Duration::from_millis(20);
/// Loudness hints kept (an arbitrary one is forgotten beyond this).
const MAX_HINTS: usize = 4096;

/// Which DSP settings changed (selects the renderer commands to send).
#[derive(Clone, Copy, PartialEq, Eq)]
enum DspPart {
    All,
    Eq,
    Filters,
    Crossfeed,
    Headroom,
}

impl EngineThread {
    pub(crate) fn new(
        cfg: EngineConfig,
        cmd_rx: Receiver<Command>,
        events: EventSink,
        rt: Arc<RtShared>,
        info: Arc<Mutex<SharedInfo>>,
        manual_slot: Arc<Mutex<Option<Renderer>>>,
    ) -> Self {
        let (err_tx, err_rx) = crossbeam_channel::bounded(16);
        let device_pref = cfg.device.clone();
        let watch = DeviceWatch::new(cfg.device_poll_ms);
        let dither = DitherControl::new(rt.clone(), DitherMode::Auto);
        Self {
            cfg,
            cmd_rx,
            events,
            rt,
            info,
            manual_slot,
            err_tx,
            err_rx,
            output: None,
            device_pref,
            device_error_reported: false,
            eq: EqSettings::default(),
            norm: Normalization::default(),
            hints: HashMap::new(),
            peq: ParametricEq::default(),
            xfeed: Crossfeed::default(),
            auto_headroom: true,
            headroom_db: 0.0,
            dsp_info: DspInfo::default(),
            state: EngineState::Stopped,
            paused: false,
            gen: 0,
            seq: 0,
            entries: VecDeque::new(),
            dec: None,
            next_path: None,
            next_src: None,
            end_index: None,
            last: None,
            scratch: Vec::with_capacity(32 * 1024),
            watch,
            last_callbacks: (0, Instant::now()),
            awaiting_prime: false,
            needs_rebuild: false,
            dither,
            excl: ExclusivePolicy::default(),
            rate_hint: None,
            rate_switch_next: None,
            #[cfg(test)]
            sync_waiters: Vec::new(),
        }
    }

    pub(crate) fn run(mut self) {
        priority::promote_decoder_thread();
        self.open_output();
        self.publish();
        let mut batch: Vec<Command> = Vec::with_capacity(16);
        'outer: loop {
            let timeout = self.wait_timeout();
            let first = if timeout.is_zero() {
                self.cmd_rx.try_recv().ok()
            } else {
                match self.cmd_rx.recv_timeout(timeout) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break 'outer,
                }
            };
            batch.clear();
            batch.extend(first);
            while let Ok(c) = self.cmd_rx.try_recv() {
                batch.push(c);
            }
            let mut i = 0;
            while i < batch.len() {
                // Coalesce runs of seeks: only the last one matters.
                if matches!(batch[i], Command::Seek(_))
                    && matches!(batch.get(i + 1), Some(Command::Seek(_)))
                {
                    i += 1;
                    continue;
                }
                // DSP setting updates (e.g. a dragged slider) commute with everything else:
                // only the last one of each kind in a batch is applied.
                if let Some(k) = dsp_setting_kind(&batch[i]) {
                    if batch[i + 1..]
                        .iter()
                        .any(|c| dsp_setting_kind(c) == Some(k))
                    {
                        i += 1;
                        continue;
                    }
                }
                let cmd = std::mem::replace(&mut batch[i], Command::Play);
                if !self.handle(cmd) {
                    break 'outer;
                }
                i += 1;
            }
            self.housekeeping();
            self.publish();
            #[cfg(test)]
            for tx in self.sync_waiters.drain(..) {
                let _ = tx.send(());
            }
            self.pump();
            self.publish();
        }
        self.shutdown();
    }

    fn wait_timeout(&self) -> Duration {
        let t = self.wait_timeout_inner();
        if self.state == EngineState::Loading && self.awaiting_prime && self.output.is_some() {
            // Report Loading -> Playing/Paused promptly.
            t.min(Duration::from_millis(5))
        } else {
            t
        }
    }

    fn wait_timeout_inner(&self) -> Duration {
        let Some(out) = self.output.as_ref() else {
            // Nothing to pump: sleep until the next device probe (or a command), but wake at
            // least once a second.
            return self
                .watch
                .until_due(Instant::now())
                .unwrap_or(Duration::from_secs(1))
                .clamp(Duration::from_millis(1), Duration::from_secs(1));
        };
        let rate = out.feeder.rate().max(1) as u64;
        let ch = out.feeder.channels().max(1) as u64;
        if self.dec.is_some() {
            let refill_at = out.feeder.capacity() / 8;
            let free = out.feeder.free();
            if !out.feeder.has_pending() && free >= refill_at {
                return Duration::ZERO;
            }
            let missing = (refill_at.saturating_sub(free)) as u64 / ch;
            let ms = missing * 1000 / rate;
            return Duration::from_millis(ms.clamp(2, 50));
        }
        if !self.entries.is_empty() {
            return Duration::from_millis(20);
        }
        Duration::from_millis(250)
    }

    fn emit(&self, ev: EngineEvent) {
        self.events.send(ev);
    }

    fn set_state(&mut self, s: EngineState) {
        if self.state != s {
            self.state = s;
            self.emit(EngineEvent::StateChanged { state: s });
        }
    }

    fn rt_cmd(&mut self, c: RtCommand) {
        if let Some(out) = self.output.as_mut() {
            if !out.feeder.command(c) {
                tracing::warn!("renderer command queue full; rebuilding output");
                self.needs_rebuild = true;
            }
        }
    }

    /// Pushes a marker; an overflow (renderer not running) schedules a rebuild.
    fn rt_marker(&mut self, m: Marker) {
        if let Some(out) = self.output.as_mut() {
            if !out.feeder.marker(m) {
                tracing::warn!("renderer marker queue full; rebuilding output");
                self.needs_rebuild = true;
            }
        }
    }

    fn handle(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::Load {
                path,
                start_ms,
                autoplay,
            } => {
                self.expedite_device_retry();
                self.load(path, start_ms.min(MAX_POSITION_MS), autoplay)
            }
            Command::SetNext(p) => self.set_next(p),
            Command::Play => {
                self.expedite_device_retry();
                self.play()
            }
            Command::Pause => self.pause(),
            Command::Stop => self.stop(),
            Command::Seek(ms) => self.seek(ms.min(MAX_POSITION_MS)),
            Command::SetEq(eq) => {
                self.eq = eq;
                self.send_dsp(DspPart::Eq);
            }
            Command::SetNormalization(n) => self.set_normalization(n),
            Command::SetLoudnessHint(path, hint) => self.set_loudness_hint(path, hint),
            Command::SetParametricEq(eq) => {
                self.peq = eq;
                self.send_dsp(DspPart::Filters);
            }
            Command::SetCrossfeed(c) => {
                self.xfeed = c;
                self.send_dsp(DspPart::Crossfeed);
            }
            Command::SetAutoHeadroom(on) => {
                self.auto_headroom = on;
                self.send_dsp(DspPart::Headroom);
            }
            Command::SetDevice(name) => {
                self.device_pref = name;
                // An explicit user choice: try right away and restart the retry backoff.
                self.watch.expedite(Instant::now());
                self.reopen_output(true);
            }
            Command::SetExclusive(on) => self.set_exclusive(on),
            Command::SetDither(mode) => {
                self.dither.set_mode(mode);
                if let Some(out) = self.output.as_mut() {
                    let bits = output::format_effective_bits(&out.info.sample_format);
                    out.info.dither = self.dither.armed(bits);
                }
            }
            Command::Shutdown => return false,
            #[cfg(test)]
            Command::Sync(tx) => self.sync_waiters.push(tx),
        }
        true
    }

    // ----- transport -------------------------------------------------------------------

    fn load(&mut self, path: PathBuf, start_ms: u64, autoplay: bool) {
        let src = match TrackSource::open(&path) {
            Ok(s) => s,
            Err(e) => {
                self.emit(EngineEvent::Error {
                    path: Some(path.clone()),
                    message: format!("{e:#}"),
                });
                self.stop();
                self.last = Some(Entry {
                    seq: 0,
                    gen: 0,
                    path,
                    duration_ms: None,
                    base_ms: 0,
                    meta: TrackMeta::default(),
                    gain: TrackGain::UNITY,
                });
                return;
            }
        };
        self.seq += 1;
        let meta = *src.meta();
        let entry = Entry {
            seq: self.seq,
            gen: self.gen,
            gain: self.track_gain(&path, &meta),
            path: path.clone(),
            duration_ms: src.info().duration_ms,
            base_ms: start_ms,
            meta,
        };
        self.entries.clear();
        self.entries.push_back(entry.clone());
        self.last = Some(entry.clone());
        self.next_path = None;
        self.next_src = None;
        self.dec = None;
        self.end_index = None;
        self.rate_switch_next = None;
        self.paused = !autoplay;
        self.emit(EngineEvent::TrackStarted {
            path,
            duration_ms: entry.duration_ms,
        });
        self.set_state(EngineState::Loading);
        self.prepare_exclusive_for(src.info().sample_rate);
        self.restart_front(Some(src));
    }

    /// (Re)starts the front entry at its `base_ms`: flush, seek, new segment marker.
    fn restart_front(&mut self, src: Option<TrackSource>) {
        // The front track restarts: a successor waiting for a rate switch goes back to being
        // the queued next track (its splice/switch is decided again at the next EOF).
        if let Some(next) = self.rate_switch_next.take() {
            if self.next_path.is_none() && self.next_src.is_none() {
                self.next_path = Some(next.path().to_path_buf());
                self.next_src = Some(next);
            }
        }
        if self.output.is_none() {
            // Resumed when a device becomes available.
            return;
        }
        let Some(mut front) = self.entries.front().cloned() else {
            return;
        };
        // A reused source (seek within the playing track) sits wherever decoding got to, so it
        // must be repositioned even for 0. Restarting from the beginning reopens the file
        // instead: exactly the state of a first play (including encoder-delay trimming).
        let mut src = match src {
            Some(s) if front.base_ms > 0 => s,
            _ => match TrackSource::open(&front.path) {
                Ok(s) => s,
                Err(e) => {
                    self.emit(EngineEvent::Error {
                        path: Some(front.path.clone()),
                        message: format!("{e:#}"),
                    });
                    self.stop();
                    return;
                }
            },
        };
        if front.base_ms > 0 {
            if let Err(e) = src.seek_ms(front.base_ms) {
                // The source position is now undefined (and the file could not be reopened):
                // playing on would put audio and position out of sync.
                self.emit(EngineEvent::Error {
                    path: Some(front.path.clone()),
                    message: format!("seek failed: {e:#}"),
                });
                self.stop();
                return;
            }
        }
        self.gen += 1;
        let gen = self.gen;
        let paused = self.paused;
        let out = self.output.as_mut().expect("output");
        let params = track_params(&front, out.feeder.rate());
        let ok = out.feeder.flush(gen)
            && out.feeder.command(RtCommand::SetPaused(paused))
            && out.feeder.command(params);
        let at = out.feeder.logical_written();
        let rate = out.feeder.rate() as u64;
        // Round up so that the reported position (frames -> ms, rounded down) never shows
        // less than the requested target.
        let base_frames = (front.base_ms * rate).div_ceil(1000);
        let ok = ok
            && out.feeder.marker(Marker::TrackStart {
                gen,
                at,
                seq: front.seq,
                base_frames,
            });
        if !ok {
            tracing::warn!("renderer queues full while restarting; rebuilding output");
            self.needs_rebuild = true;
        }
        front.gen = gen;
        self.awaiting_prime = true;
        self.entries.clear();
        self.entries.push_back(front.clone());
        self.dec = Some(Decoding {
            src,
            conv: None,
            seq: front.seq,
            origin: at,
        });
        self.end_index = None;
    }

    fn set_next(&mut self, path: Option<PathBuf>) {
        if let Some(pending) = self.rate_switch_next.as_ref() {
            // Already committed to start after the current track (exclusive rate switch).
            if path.as_deref() == Some(pending.path()) {
                return;
            }
            self.rate_switch_next = None;
        }
        if path.is_some() && path == self.next_path && self.next_src.is_some() {
            return;
        }
        // The successor was already spliced into the ring (the current track hit EOF) but is
        // not reported as current yet: queueing it again would play it twice.
        if path.is_some()
            && self.entries.len() > 1
            && self.entries.back().map(|e| &e.path) == path.as_ref()
        {
            self.next_src = None;
            self.next_path = None;
            return;
        }
        self.next_src = None;
        self.next_path = path;
        let Some(path) = self.next_path.clone() else {
            return;
        };
        // Preopen to surface errors early and make the splice instant.
        match TrackSource::open(&path) {
            Ok(src) => self.next_src = Some(src),
            Err(e) => {
                self.emit(EngineEvent::Error {
                    path: Some(path),
                    message: format!("{e:#}"),
                });
                self.next_path = None;
                return;
            }
        }
        // The current track already hit EOF (End marker queued) but is still playing: splice
        // the next track right at the End index.
        if self.dec.is_none() && self.state != EngineState::Stopped && !self.entries.is_empty() {
            if let (Some(at), Some(_)) = (self.end_index, self.output.as_ref()) {
                let src = self.next_src.take().expect("next src");
                self.next_path = None;
                if self.needs_rate_switch(&src) {
                    self.rate_switch_next = Some(src);
                } else {
                    self.splice(src, at, None, at);
                    self.end_index = None;
                }
            }
        }
    }

    fn take_next_source(&mut self) -> Option<TrackSource> {
        if let Some(src) = self.next_src.take() {
            self.next_path = None;
            return Some(src);
        }
        let path = self.next_path.take()?;
        match TrackSource::open(&path) {
            Ok(s) => Some(s),
            Err(e) => {
                self.emit(EngineEvent::Error {
                    path: Some(path),
                    message: format!("{e:#}"),
                });
                None
            }
        }
    }

    /// Appends `src` as a new entry starting at ring index `at`.
    fn splice(&mut self, src: TrackSource, at: u64, conv: Option<Converter>, origin: u64) {
        self.seq += 1;
        let seq = self.seq;
        let gen = self.gen;
        let meta = *src.meta();
        let entry = Entry {
            seq,
            gen,
            path: src.path().to_path_buf(),
            duration_ms: src.info().duration_ms,
            base_ms: 0,
            gain: self.track_gain(src.path(), &meta),
            meta,
        };
        // The gain must reach the renderer before the marker that starts the track.
        if let Some(rate) = self.output.as_ref().map(|o| o.feeder.rate()) {
            self.rt_cmd(track_params(&entry, rate));
        }
        self.rt_marker(Marker::TrackStart {
            gen,
            at,
            seq,
            base_frames: 0,
        });
        self.entries.push_back(entry);
        self.dec = Some(Decoding {
            src,
            conv,
            seq,
            origin,
        });
    }

    fn play(&mut self) {
        match self.state {
            EngineState::Paused => {
                self.paused = false;
                self.rt_cmd(RtCommand::SetPaused(false));
                self.set_state(EngineState::Playing);
            }
            EngineState::Loading => {
                self.paused = false;
                self.rt_cmd(RtCommand::SetPaused(false));
            }
            EngineState::Stopped => {
                if let Some(last) = self.last.clone() {
                    self.load(last.path, 0, true);
                }
            }
            EngineState::Playing => {}
        }
    }

    fn pause(&mut self) {
        match self.state {
            EngineState::Playing => {
                self.paused = true;
                self.rt_cmd(RtCommand::SetPaused(true));
                self.set_state(EngineState::Paused);
            }
            EngineState::Loading => {
                self.paused = true;
                self.rt_cmd(RtCommand::SetPaused(true));
            }
            _ => {}
        }
    }

    fn stop(&mut self) {
        self.gen += 1;
        let gen = self.gen;
        if let Some(out) = self.output.as_mut() {
            if !out.feeder.flush(gen) {
                self.needs_rebuild = true;
            }
        }
        if let Some(front) = self.entries.front() {
            let mut e = front.clone();
            e.base_ms = 0;
            self.last = Some(e);
        }
        self.entries.clear();
        self.dec = None;
        self.next_path = None;
        self.next_src = None;
        self.end_index = None;
        self.rate_switch_next = None;
        self.set_state(EngineState::Stopped);
    }

    fn seek(&mut self, ms: u64) {
        if self.entries.is_empty() {
            if let Some(last) = self.last.clone() {
                self.load(last.path, ms, false);
            }
            return;
        }
        let front_seq = self.entries[0].seq;
        let src = match self.dec.take() {
            Some(d) if d.seq == front_seq => Some(d.src),
            _ => None,
        };
        if self.entries.len() > 1 {
            // We were already decoding the gapless successor; queue it again.
            self.next_path = Some(self.entries[1].path.clone());
            self.next_src = None;
        }
        self.entries.truncate(1);
        self.entries[0].base_ms = ms;
        self.restart_front(src);
    }

    // ----- decoding ----------------------------------------------------------------------

    fn pump(&mut self) {
        let deadline = Instant::now() + MAX_PUMP_SLICE;
        let mut steps = 0u32;
        loop {
            let Some(out) = self.output.as_mut() else {
                return;
            };
            if !out.feeder.drain_pending() || self.dec.is_none() {
                return;
            }
            if out.feeder.free() < out.feeder.capacity() / 16 {
                return;
            }
            // Always make some progress, even when commands keep arriving (e.g. a dragged EQ
            // slider): otherwise a steady command stream could starve the ring.
            if steps > 0 && (!self.cmd_rx.is_empty() || Instant::now() > deadline) {
                return;
            }
            steps += 1;
            self.decode_step();
        }
    }

    fn decode_step(&mut self) {
        let (Some(dec), Some(out)) = (self.dec.as_mut(), self.output.as_mut()) else {
            return;
        };
        let result = dec.src.decode_next();
        let fmt = match result {
            Ok(Some(fmt)) => fmt,
            Ok(None) => {
                self.on_eof();
                return;
            }
            Err(e) => {
                let path = dec.src.path().to_path_buf();
                self.emit(EngineEvent::Error {
                    path: Some(path),
                    message: format!("{e:#}"),
                });
                self.on_eof();
                return;
            }
        };
        self.scratch.clear();
        if !dec.conv.as_ref().is_some_and(|c| c.accepts(&fmt)) {
            if let Some(c) = dec.conv.as_mut() {
                let _ = c.flush(&mut self.scratch);
                out.feeder.push(&self.scratch);
                self.scratch.clear();
            }
            match Converter::new(
                fmt,
                out.feeder.rate(),
                out.feeder.channels(),
                self.cfg.resampler,
            ) {
                Ok(c) => {
                    dec.conv = Some(c);
                    dec.origin = out.feeder.logical_written();
                }
                Err(e) => {
                    let path = dec.src.path().to_path_buf();
                    self.emit(EngineEvent::Error {
                        path: Some(path),
                        message: format!("{e:#}"),
                    });
                    self.on_eof();
                    return;
                }
            }
        }
        let conv = dec.conv.as_mut().expect("converter");
        if let Err(e) = conv.process(dec.src.block(), &mut self.scratch) {
            tracing::warn!("conversion failed: {e:#}");
        }
        out.feeder.push(&self.scratch);
    }

    fn on_eof(&mut self) {
        let Some(mut dec) = self.dec.take() else {
            return;
        };
        if self.output.is_none() {
            return;
        }
        let next = match self.take_next_source() {
            // Exclusive output at another rate: end this track; the switch happens once it
            // has played out (see `track_progress`).
            Some(src) if self.needs_rate_switch(&src) => {
                self.rate_switch_next = Some(src);
                None
            }
            other => other,
        };
        self.scratch.clear();
        match next {
            Some(src) => {
                let compatible = dec.conv.as_ref().is_some_and(|c| {
                    let f = c.source_format();
                    let i = src.info();
                    i.sample_rate == Some(f.rate)
                        && i.channels.map(|n| n as usize) == Some(f.channels)
                });
                let out = self.output.as_mut().expect("output");
                if compatible {
                    // Keep the converter running: seamless join even through the resampler.
                    let c = dec.conv.take().expect("converter");
                    let ch = out.feeder.channels() as u64;
                    let at = dec.origin + c.out_frames_for(c.total_in()) * ch;
                    let origin = dec.origin;
                    self.splice(src, at, Some(c), origin);
                } else {
                    if let Some(c) = dec.conv.as_mut() {
                        let _ = c.flush(&mut self.scratch);
                        out.feeder.push(&self.scratch);
                    }
                    let at = out.feeder.logical_written();
                    self.splice(src, at, None, at);
                }
            }
            None => {
                let out = self.output.as_mut().expect("output");
                if let Some(c) = dec.conv.as_mut() {
                    let _ = c.flush(&mut self.scratch);
                    out.feeder.push(&self.scratch);
                }
                let at = out.feeder.logical_written();
                let gen = self.gen;
                self.rt_marker(Marker::End { gen, at });
                self.end_index = Some(at);
            }
        }
    }

    // ----- housekeeping --------------------------------------------------------------------

    fn housekeeping(&mut self) {
        if std::mem::take(&mut self.needs_rebuild) && self.output.is_some() {
            self.reopen_output(false);
        }
        let mut err = None;
        while let Ok(e) = self.err_rx.try_recv() {
            err = Some(e);
        }
        if let Some(e) = err {
            self.emit(EngineEvent::Error {
                path: None,
                message: format!("audio device error: {e}"),
            });
            self.note_stream_failure(&e);
            self.reopen_output(false);
        }

        let now = Instant::now();
        // Stall watchdog: a stream that stopped calling back while it should be playing.
        if !matches!(
            self.output.as_ref().map(|o| &o.backend),
            None | Some(Backend::Manual)
        ) && !self.paused
            && matches!(self.state, EngineState::Playing | EngineState::Loading)
        {
            let cb = self.rt.callbacks.load(Ordering::Relaxed);
            if cb != self.last_callbacks.0 {
                self.last_callbacks = (cb, now);
            } else if now.duration_since(self.last_callbacks.1) > STALL_TIMEOUT {
                self.emit(EngineEvent::Error {
                    path: None,
                    message: "audio device stopped responding; reopening".into(),
                });
                self.note_stream_failure("the device stopped responding");
                self.reopen_output(false);
                self.last_callbacks = (self.rt.callbacks.load(Ordering::Relaxed), Instant::now());
            }
        } else {
            self.last_callbacks = (self.rt.callbacks.load(Ordering::Relaxed), now);
        }

        if self.cfg.output == OutputMode::Cpal && self.watch.is_due(now) {
            match self.output.as_ref() {
                None => {
                    // Backed-off retry; open_output() reschedules the next one on failure.
                    if self.open_output() && !self.entries.is_empty() {
                        self.restart_front(None);
                    }
                }
                Some(out) => {
                    let name = out.info.name.clone();
                    let policy = self.present_policy(&out.info);
                    // Reopening below reschedules again through open_output().
                    self.watch.rearm(now, policy);
                    // Fell back to the default because the preferred device was missing:
                    // switch back once it reappears.
                    let pref_back = policy.preferred_missing
                        && self
                            .device_pref
                            .as_deref()
                            .is_some_and(output::output_device_exists);
                    if pref_back {
                        tracing::info!("preferred output device {:?} is back", self.device_pref);
                        self.reopen_output(true);
                    } else if policy.follow_default && policy.cheap_probes {
                        let current = output::default_output_device_name();
                        if current.is_some() && current.as_deref() != Some(name.as_str()) {
                            tracing::info!("default output device changed to {:?}", current);
                            self.reopen_output(true);
                        }
                    }
                }
            }
        }

        self.track_progress();
    }

    fn track_progress(&mut self) {
        let clock = self.rt.clock.read();
        if clock.gen == self.gen && self.entries.iter().any(|e| e.seq == clock.track_seq) {
            while self.entries.len() > 1 && self.entries[0].seq != clock.track_seq {
                self.entries.pop_front();
                let e = &self.entries[0];
                let ev = EngineEvent::AdvancedToNext {
                    path: e.path.clone(),
                    duration_ms: e.duration_ms,
                };
                self.last = Some(e.clone());
                self.emit(ev);
            }
        }
        if self.awaiting_prime && self.rt.primed_gen.load(Ordering::Acquire) == self.gen {
            self.awaiting_prime = false;
        }
        if self.state == EngineState::Loading
            && !self.awaiting_prime
            && self.output.is_some()
            && !self.entries.is_empty()
        {
            let s = if self.paused {
                EngineState::Paused
            } else {
                EngineState::Playing
            };
            self.set_state(s);
        }
        if let Some(end) = self.end_index {
            let at = self.rt.ended_at.load(Ordering::Acquire);
            let gen = self.rt.ended_gen.load(Ordering::Relaxed);
            if at == end + 1 && gen == self.gen {
                self.end_index = None;
                if let Some(src) = self.rate_switch_next.take() {
                    self.advance_with_reopen(src);
                    return;
                }
                if let Some(e) = self.entries.back().cloned() {
                    let mut last = e.clone();
                    last.base_ms = 0;
                    self.last = Some(last);
                    self.emit(EngineEvent::TrackFinished { path: e.path });
                }
                self.entries.clear();
                self.dec = None;
                self.set_state(EngineState::Stopped);
            }
        }
    }

    // ----- output management -------------------------------------------------------------

    /// What an open cpal output must be checked for periodically (see `device_watch`).
    fn present_policy(&self, info: &DeviceInfo) -> PresentPolicy {
        PresentPolicy {
            follow_default: info.follows_default,
            preferred_missing: self.device_pref.as_ref().is_some_and(|p| *p != info.name),
            cheap_probes: !output::ALSA_HOST,
        }
    }

    /// The user wants audio while no device is open: retry soon instead of waiting out the
    /// backoff.
    fn expedite_device_retry(&mut self) {
        if self.output.is_none() && self.cfg.output == OutputMode::Cpal {
            self.watch.expedite(Instant::now());
        }
    }

    fn render_config(&self, rate: u32, channels: u16) -> RenderConfig {
        let r = rate as usize;
        RenderConfig {
            rate,
            channels: channels as usize,
            ring_frames: (r * self.cfg.ring_ms.max(100) as usize / 1000).max(1024),
            prebuffer_frames: r * self.cfg.prebuffer_ms as usize / 1000,
            fade_frames: (r * self.cfg.fade_ms as usize / 1000).max(1),
        }
    }

    fn open_output(&mut self) -> bool {
        let opened = match self.cfg.output.clone() {
            OutputMode::Manual {
                sample_rate,
                channels,
            } => {
                let (feeder, mut renderer) =
                    new_pair(self.render_config(sample_rate, channels), self.rt.clone());
                // Manual rendering is not paced by a device clock: count frames as played
                // the moment they are rendered.
                renderer.set_realtime_clock(false);
                if let Ok(mut slot) = self.manual_slot.lock() {
                    *slot = Some(renderer);
                }
                Some(Output {
                    feeder,
                    info: DeviceInfo {
                        name: "manual".into(),
                        sample_rate,
                        channels,
                        sample_format: "F32".into(),
                        buffer_frames: None,
                        follows_default: false,
                        exclusive: false,
                        dither: false,
                    },
                    backend: Backend::Manual,
                })
            }
            OutputMode::Cpal => {
                let mut feeder_slot: Option<Feeder> = None;
                let rt = self.rt.clone();
                let cfgs = (self.cfg.ring_ms, self.cfg.prebuffer_ms, self.cfg.fade_ms);
                let mut make = |rate: u32, ch: u16| {
                    let r = rate as usize;
                    let rc = RenderConfig {
                        rate,
                        channels: ch as usize,
                        ring_frames: (r * cfgs.0.max(100) as usize / 1000).max(1024),
                        prebuffer_frames: r * cfgs.1 as usize / 1000,
                        fade_frames: (r * cfgs.2 as usize / 1000).max(1),
                    };
                    let (f, renderer) = new_pair(rc, rt.clone());
                    feeder_slot = Some(f);
                    renderer
                };
                // Exclusive first when wanted; any failure falls back to the shared output.
                #[cfg_attr(not(windows), allow(unused_mut))]
                let mut exclusive: Option<(Backend, DeviceInfo)> = None;
                let now = Instant::now();
                match self
                    .excl
                    .plan(self.cfg.exclusive, output_wasapi::SUPPORTED, now)
                {
                    Plan::Shared(reason) => {
                        if let Some(reason) = reason {
                            self.report_exclusive_unavailable(reason);
                        }
                    }
                    Plan::Exclusive => {
                        #[cfg(windows)]
                        match output_wasapi::open_exclusive(
                            self.device_pref.as_deref(),
                            self.cfg.buffer_ms,
                            self.rate_hint,
                            &mut make,
                            self.err_tx.clone(),
                            &self.dither,
                        ) {
                            Ok((stream, info)) => {
                                self.excl.opened(now);
                                exclusive = Some((Backend::Wasapi(stream), info));
                            }
                            Err(reason) => {
                                tracing::warn!(
                                    "exclusive output failed, falling back to shared: {reason}"
                                );
                                if let Some(r) = self.excl.open_failed(&reason, now) {
                                    self.report_exclusive_unavailable(r);
                                }
                            }
                        }
                    }
                }
                let result = match exclusive {
                    Some(opened) => Ok(opened),
                    None => output::open_cpal(
                        self.device_pref.as_deref(),
                        self.cfg.buffer_ms,
                        &mut make,
                        self.err_tx.clone(),
                        &self.dither,
                    )
                    .map(|(stream, info)| (Backend::Cpal(stream), info)),
                };
                match result {
                    Ok((backend, info)) => {
                        let policy = self.present_policy(&info);
                        self.watch.opened(Instant::now(), policy);
                        Some(Output {
                            feeder: feeder_slot.expect("feeder"),
                            info,
                            backend,
                        })
                    }
                    Err(e) => {
                        self.watch.open_failed(Instant::now());
                        tracing::debug!(
                            "opening audio output failed ({} in a row); next attempt in {:?}",
                            self.watch.failures(),
                            self.watch.until_due(Instant::now()).unwrap_or_default()
                        );
                        if !self.device_error_reported {
                            self.device_error_reported = true;
                            self.emit(EngineEvent::Error {
                                path: None,
                                message: format!("cannot open audio output: {e:#}"),
                            });
                            self.emit(EngineEvent::DeviceChanged { device: None });
                        }
                        None
                    }
                }
            }
        };
        let Some(mut out) = opened else {
            return false;
        };
        self.device_error_reported = false;
        while self.err_rx.try_recv().is_ok() {}
        for c in self.dsp_commands(DspPart::All, out.feeder.rate(), out.feeder.channels()) {
            out.feeder.command(c);
        }
        out.feeder.command(RtCommand::SetPaused(self.paused));
        let info = out.info.clone();
        self.output = Some(out);
        self.emit(EngineEvent::DeviceChanged { device: Some(info) });
        true
    }

    /// Rebuilds the output (device change/error) and resumes the audible track where it was.
    /// `graceful`: the old device still works, so fade out before tearing it down.
    /// Time for a fade-out sent now to have left the device (fade + device buffer + margin);
    /// `None` when there is no cpal stream to wait for.
    fn drain_wait(&self) -> Option<Duration> {
        match self.output.as_ref() {
            Some(Output { backend, info, .. }) if !matches!(backend, Backend::Manual) => {
                // Unknown (default) buffer: assume a generous 50 ms.
                let buffer_ms = info
                    .buffer_frames
                    .map(|f| f as u64 * 1000 / info.sample_rate.max(1) as u64)
                    .unwrap_or(50);
                Some(Duration::from_millis(
                    self.cfg.fade_ms as u64 + buffer_ms + 20,
                ))
            }
            _ => None,
        }
    }

    fn reopen_output(&mut self, graceful: bool) {
        // Make `entries` reflect what is audible right now (a gapless advance may not have
        // been processed yet), so the resume position refers to the right track.
        self.track_progress();
        let audible = self.state == EngineState::Playing && !self.paused;
        if graceful && audible {
            if let Some(wait) = self.drain_wait() {
                self.rt_cmd(RtCommand::SetPaused(true));
                std::thread::sleep(wait);
            }
        }
        let resume_ms = self.entries.front().map(|front| {
            let clock = self.rt.clock.read();
            if self.output.is_some() && clock.track_seq == front.seq && clock.gen >= front.gen {
                clock.audible_ms(self.rt.now_ns())
            } else {
                front.base_ms
            }
        });
        // Drop the stream first (joins/stops the callback thread), then the manual renderer.
        self.output = None;
        if let Ok(mut slot) = self.manual_slot.lock() {
            slot.take();
        }
        let front_seq = self.entries.front().map(|e| e.seq);
        let src = match self.dec.take() {
            Some(d) if Some(d.seq) == front_seq => Some(d.src),
            _ => None,
        };
        if self.cfg.exclusive {
            // An exclusive output reopens at the audible track's rate.
            let probed = || {
                let front = self.entries.front()?;
                super::source::probe_track(&front.path).ok()?.sample_rate
            };
            self.rate_hint = src
                .as_ref()
                .and_then(|s| s.info().sample_rate)
                .or_else(probed)
                .or(self.rate_hint);
        }
        if self.entries.len() > 1 {
            self.next_path = Some(self.entries[1].path.clone());
            self.next_src = None;
        }
        self.entries.truncate(1);
        if let (Some(front), Some(ms)) = (self.entries.front_mut(), resume_ms) {
            front.base_ms = ms;
        }
        self.end_index = None;
        if self.open_output() && !self.entries.is_empty() {
            self.restart_front(src);
            if self.state == EngineState::Playing {
                // Brief re-prime; report Loading until audio flows again.
                self.set_state(EngineState::Loading);
            }
        }
        // On failure open_output() already reported Error + DeviceChanged{None}; the device
        // poll retries and resumes the front entry when an output becomes available.
    }

    // ----- DSP / normalization ------------------------------------------------------------

    fn track_gain(&self, path: &std::path::Path, meta: &TrackMeta) -> TrackGain {
        resolve_gain(&self.norm, &meta.replay_gain, self.hints.get(path))
    }

    /// Recomputes every queued entry's gain and sends the changed ones (the renderer ramps the
    /// audible track's gain; queued tracks switch at their boundary).
    fn refresh_gains(&mut self, only: Option<&std::path::Path>) {
        let rate = self.output.as_ref().map(|o| o.feeder.rate());
        let mut cmds: Vec<RtCommand> = Vec::new();
        for i in 0..self.entries.len() {
            let e = &self.entries[i];
            if only.is_some_and(|p| p != e.path) {
                continue;
            }
            let g = self.track_gain(&e.path, &e.meta);
            if g != e.gain {
                self.entries[i].gain = g;
                if let Some(rate) = rate {
                    cmds.push(track_params(&self.entries[i], rate));
                }
            }
        }
        if let Some(last) = self.last.as_ref() {
            if only.is_none_or(|p| p == last.path) {
                let g = self.track_gain(&last.path, &last.meta);
                if let Some(l) = self.last.as_mut() {
                    l.gain = g;
                }
            }
        }
        for c in cmds {
            self.rt_cmd(c);
        }
    }

    fn set_normalization(&mut self, n: Normalization) {
        self.norm = n;
        self.refresh_gains(None);
    }

    fn set_loudness_hint(&mut self, path: PathBuf, hint: Option<LoudnessHint>) {
        match hint {
            Some(h) => {
                if self.hints.len() >= MAX_HINTS && !self.hints.contains_key(&path) {
                    if let Some(k) = self.hints.keys().next().cloned() {
                        self.hints.remove(&k);
                    }
                }
                self.hints.insert(path.clone(), h);
            }
            None => {
                self.hints.remove(&path);
            }
        }
        self.refresh_gains(Some(&path));
    }

    /// Renderer commands carrying the DSP settings for an output of `rate`/`channels`.
    fn dsp_commands(&mut self, part: DspPart, rate: u32, channels: usize) -> Vec<RtCommand> {
        let mut v = Vec::with_capacity(4);
        if matches!(part, DspPart::All | DspPart::Eq) {
            v.push(RtCommand::SetEq(EqCoeffs::from_settings(&self.eq, rate)));
        }
        if matches!(part, DspPart::All | DspPart::Filters) {
            v.push(RtCommand::SetPeq(PeqCoeffs::from_settings(&self.peq, rate)));
        }
        if matches!(part, DspPart::All | DspPart::Crossfeed) {
            v.push(RtCommand::SetCrossfeed(CrossfeedCoeffs::new(
                &self.xfeed,
                rate,
                channels,
            )));
        }
        if part != DspPart::Crossfeed {
            self.headroom_db = if self.auto_headroom {
                auto_headroom_db(&self.eq, &self.peq, rate)
            } else {
                0.0
            };
            let g = if self.headroom_db == 0.0 {
                1.0
            } else {
                db_to_linear(self.headroom_db)
            };
            v.push(RtCommand::SetHeadroom(g));
        }
        self.dsp_info = DspInfo {
            eq: EqCoeffs::from_settings(&self.eq, rate).active,
            peq: PeqCoeffs::from_settings(&self.peq, rate).active,
            crossfeed: CrossfeedCoeffs::new(&self.xfeed, rate, channels)
                .active
                .then_some(self.xfeed.level),
            headroom_db: self.headroom_db,
            resampler: self.cfg.resampler,
        };
        v
    }

    /// Sends changed DSP settings to the renderer.
    fn send_dsp(&mut self, part: DspPart) {
        let Some((rate, ch)) = self
            .output
            .as_ref()
            .map(|o| (o.feeder.rate(), o.feeder.channels()))
        else {
            return;
        };
        for c in self.dsp_commands(part, rate, ch) {
            self.rt_cmd(c);
        }
    }

    fn dsp_info(&self) -> DspInfo {
        if self.output.is_some() {
            self.dsp_info
        } else {
            DspInfo {
                resampler: self.cfg.resampler,
                ..Default::default()
            }
        }
    }

    // ----- exclusive output ------------------------------------------------------------

    fn report_exclusive_unavailable(&self, reason: String) {
        tracing::warn!("exclusive output unavailable, using the shared output: {reason}");
        self.emit(EngineEvent::ExclusiveUnavailable { reason });
    }

    fn set_exclusive(&mut self, on: bool) {
        let changed = self.cfg.exclusive != on;
        self.cfg.exclusive = on;
        // An explicit user choice: report the outcome again even if it is the same as before.
        self.excl.reset();
        if on && !output_wasapi::SUPPORTED {
            if let Plan::Shared(Some(r)) = self.excl.plan(true, false, Instant::now()) {
                self.report_exclusive_unavailable(r);
            }
            return;
        }
        if self.cfg.output != OutputMode::Cpal {
            if on {
                self.report_exclusive_unavailable(
                    "exclusive mode is not available for manual output".into(),
                );
            }
            return;
        }
        let is_exclusive = self.output.as_ref().is_some_and(|o| o.info.exclusive);
        if changed || on != is_exclusive {
            self.watch.expedite(Instant::now());
            self.reopen_output(true);
        }
    }

    /// The running stream failed (error/stall); feeds the exclusive fallback policy.
    fn note_stream_failure(&mut self, reason: &str) {
        #[cfg(windows)]
        if matches!(
            self.output.as_ref().map(|o| &o.backend),
            Some(Backend::Wasapi(_))
        ) {
            self.excl.stream_failed(reason, Instant::now());
        }
        #[cfg(not(windows))]
        let _ = reason;
    }

    /// Whether `src` (the gapless successor) cannot be spliced because the exclusive output
    /// has to be reopened at its sample rate.
    fn needs_rate_switch(&self, src: &TrackSource) -> bool {
        #[cfg(windows)]
        if let Some(Output {
            backend: Backend::Wasapi(stream),
            info,
            ..
        }) = self.output.as_ref()
        {
            return self.cfg.exclusive
                && output_wasapi::needs_rate_reopen(
                    info.sample_rate,
                    src.info().sample_rate,
                    &stream.supported_rates,
                );
        }
        let _ = src;
        false
    }

    /// Before a track at `rate` starts (load): reopens an exclusive output at that rate, or
    /// retries exclusive mode after an earlier fallback (at most every `RETRY_AFTER`).
    fn prepare_exclusive_for(&mut self, rate: Option<u32>) {
        if rate.is_some() {
            self.rate_hint = rate;
        }
        if self.cfg.output != OutputMode::Cpal || !self.cfg.exclusive || !output_wasapi::SUPPORTED {
            return;
        }
        let now = Instant::now();
        let reopen = match self
            .output
            .as_ref()
            .map(|o| (&o.backend, o.info.sample_rate))
        {
            #[cfg(windows)]
            Some((Backend::Wasapi(stream), current)) => {
                output_wasapi::needs_rate_reopen(current, rate, &stream.supported_rates)
            }
            Some((Backend::Cpal(_), _)) => self.excl.retry_due(now),
            _ => false,
        };
        if !reopen {
            return;
        }
        tracing::info!("reopening the output for a {rate:?} Hz track (exclusive mode)");
        if let Some(wait) = self.drain_wait() {
            self.rt_cmd(RtCommand::SetPaused(true));
            std::thread::sleep(wait);
        }
        self.output = None;
        self.open_output();
    }

    /// Exclusive rate switch at a track boundary: the current track has played out (its End
    /// marker was rendered); drain the device, reopen at the successor's rate and start it.
    /// Reported like a gapless advance, but with a short silence in between.
    fn advance_with_reopen(&mut self, src: TrackSource) {
        if let Some(wait) = self.drain_wait() {
            std::thread::sleep(wait);
        }
        self.seq += 1;
        let meta = *src.meta();
        let entry = Entry {
            seq: self.seq,
            gen: self.gen,
            path: src.path().to_path_buf(),
            duration_ms: src.info().duration_ms,
            base_ms: 0,
            gain: self.track_gain(src.path(), &meta),
            meta,
        };
        tracing::info!(
            "exclusive mode: {:?} needs {:?} Hz; reopening the output between tracks",
            entry.path,
            src.info().sample_rate
        );
        self.entries.clear();
        self.entries.push_back(entry.clone());
        self.last = Some(entry.clone());
        self.dec = None;
        self.emit(EngineEvent::AdvancedToNext {
            path: entry.path,
            duration_ms: entry.duration_ms,
        });
        self.rate_hint = src.info().sample_rate;
        self.output = None;
        if self.open_output() {
            self.restart_front(Some(src));
            if self.state == EngineState::Playing {
                self.set_state(EngineState::Loading);
            }
        }
        // On failure the device retry resumes the new front entry (see housekeeping).
    }

    fn publish(&self) {
        let mut info = match self.info.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        info.state = self.state;
        info.entries.clear();
        info.entries
            .extend(self.entries.iter().map(EntryInfo::from));
        info.last = self.last.as_ref().map(EntryInfo::from);
        info.device = self.output.as_ref().map(|o| o.info.clone());
        info.dsp = self.dsp_info();
    }

    fn shutdown(&mut self) {
        if self.state == EngineState::Playing && self.output.is_some() {
            self.rt_cmd(RtCommand::SetPaused(true));
            // Let the fade-out reach the speaker (not just the device buffer) before the
            // stream is torn down; stopping a stream discards what is still buffered.
            if let Some(wait) = self.drain_wait() {
                std::thread::sleep(wait);
            }
        }
        self.dec = None;
        self.next_src = None;
        self.output = None;
        if let Ok(mut slot) = self.manual_slot.lock() {
            slot.take();
        }
    }
}

/// Coalescing key of DSP setting commands (only the last of each kind in a batch matters).
fn dsp_setting_kind(c: &Command) -> Option<u8> {
    match c {
        Command::SetEq(_) => Some(0),
        Command::SetParametricEq(_) => Some(1),
        Command::SetCrossfeed(_) => Some(2),
        Command::SetAutoHeadroom(_) => Some(3),
        Command::SetNormalization(_) => Some(4),
        _ => None,
    }
}

/// The renderer's per-track parameters for `e` on an output at `rate`.
fn track_params(e: &Entry, rate: u32) -> RtCommand {
    RtCommand::SetTrackParams {
        seq: e.seq,
        gain: e.gain.linear,
        resampled: e.meta.sample_rate != 0 && e.meta.sample_rate != rate,
    }
}
