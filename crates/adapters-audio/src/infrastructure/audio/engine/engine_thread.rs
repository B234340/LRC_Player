//! The engine/control thread: command handling, decoding into the ring, gapless splicing,
//! state/event bookkeeping and device management. Never runs on the realtime thread.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};

use super::convert::Converter;
use super::dsp::{EqCoeffs, EqSettings};
use super::output;
use super::priority;
use super::renderer::{new_pair, Feeder, Marker, RenderConfig, Renderer, RtCommand, RtShared};
use super::source::TrackSource;
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
    SetDevice(Option<String>),
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
}

#[derive(Clone, Debug)]
pub(crate) struct EntryInfo {
    pub(crate) seq: u64,
    pub(crate) gen: u64,
    pub(crate) path: PathBuf,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) base_ms: u64,
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
    last_poll: Instant,
    last_callbacks: (u64, Instant),
    /// A segment was started and Loading -> Playing/Paused waits for the renderer to prime it.
    awaiting_prime: bool,
    /// A renderer queue overflowed (callbacks stalled while commands kept coming): the
    /// renderer has lost a command/marker and must be rebuilt to resynchronise.
    needs_rebuild: bool,
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
            last_poll: Instant::now(),
            last_callbacks: (0, Instant::now()),
            awaiting_prime: false,
            needs_rebuild: false,
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
                // EQ updates (e.g. a dragged slider) commute with everything else: only the
                // last one of a batch is sent to the renderer.
                if matches!(batch[i], Command::SetEq(_))
                    && batch[i + 1..]
                        .iter()
                        .any(|c| matches!(c, Command::SetEq(_)))
                {
                    i += 1;
                    continue;
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
            return Duration::from_millis(self.cfg.device_poll_ms.clamp(50, 1000) as u64);
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
            } => self.load(path, start_ms.min(MAX_POSITION_MS), autoplay),
            Command::SetNext(p) => self.set_next(p),
            Command::Play => self.play(),
            Command::Pause => self.pause(),
            Command::Stop => self.stop(),
            Command::Seek(ms) => self.seek(ms.min(MAX_POSITION_MS)),
            Command::SetEq(eq) => {
                self.eq = eq;
                if let Some(rate) = self.output.as_ref().map(|o| o.feeder.rate()) {
                    self.rt_cmd(RtCommand::SetEq(EqCoeffs::from_settings(&eq, rate)));
                }
            }
            Command::SetDevice(name) => {
                self.device_pref = name;
                self.reopen_output(true);
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
                });
                return;
            }
        };
        self.seq += 1;
        let entry = Entry {
            seq: self.seq,
            gen: self.gen,
            path: path.clone(),
            duration_ms: src.info().duration_ms,
            base_ms: start_ms,
        };
        self.entries.clear();
        self.entries.push_back(entry.clone());
        self.last = Some(entry.clone());
        self.next_path = None;
        self.next_src = None;
        self.dec = None;
        self.end_index = None;
        self.paused = !autoplay;
        self.emit(EngineEvent::TrackStarted {
            path,
            duration_ms: entry.duration_ms,
        });
        self.set_state(EngineState::Loading);
        self.restart_front(Some(src));
    }

    /// (Re)starts the front entry at its `base_ms`: flush, seek, new segment marker.
    fn restart_front(&mut self, src: Option<TrackSource>) {
        if self.output.is_none() {
            // Resumed when a device becomes available.
            return;
        }
        let Some(mut front) = self.entries.front().cloned() else {
            return;
        };
        let mut src = match src {
            Some(s) => s,
            None => match TrackSource::open(&front.path) {
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
        let ok = out.feeder.flush(gen) && out.feeder.command(RtCommand::SetPaused(paused));
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
                self.splice(src, at, None, at);
                self.end_index = None;
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
        self.rt_marker(Marker::TrackStart {
            gen,
            at,
            seq,
            base_frames: 0,
        });
        self.entries.push_back(Entry {
            seq,
            gen,
            path: src.path().to_path_buf(),
            duration_ms: src.info().duration_ms,
            base_ms: 0,
        });
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
        let next = self.take_next_source();
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
            self.reopen_output(false);
        }

        let now = Instant::now();
        // Stall watchdog: a stream that stopped calling back while it should be playing.
        if matches!(
            self.output.as_ref().map(|o| &o.backend),
            Some(Backend::Cpal(_))
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
                self.reopen_output(false);
                self.last_callbacks = (self.rt.callbacks.load(Ordering::Relaxed), Instant::now());
            }
        } else {
            self.last_callbacks = (self.rt.callbacks.load(Ordering::Relaxed), now);
        }

        if self.cfg.output == OutputMode::Cpal
            && now.duration_since(self.last_poll)
                >= Duration::from_millis(self.cfg.device_poll_ms as u64)
        {
            self.last_poll = now;
            match self.output.as_ref() {
                None => {
                    if self.open_output() && !self.entries.is_empty() {
                        self.restart_front(None);
                    }
                }
                Some(out) => {
                    let name = out.info.name.clone();
                    // Fell back to the default because the preferred device was missing:
                    // switch back once it reappears.
                    let pref_back = self.device_pref.as_ref().is_some_and(|p| {
                        *p != name && output::output_device_names().iter().any(|n| n == p)
                    });
                    if pref_back {
                        tracing::info!("preferred output device {:?} is back", self.device_pref);
                        self.reopen_output(true);
                    } else if out.info.follows_default {
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
                    },
                    backend: Backend::Manual,
                })
            }
            OutputMode::Cpal => {
                let mut feeder_slot: Option<Feeder> = None;
                let rt = self.rt.clone();
                let cfgs = (self.cfg.ring_ms, self.cfg.prebuffer_ms, self.cfg.fade_ms);
                let result = {
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
                    output::open_cpal(
                        self.device_pref.as_deref(),
                        self.cfg.buffer_ms,
                        &mut make,
                        self.err_tx.clone(),
                    )
                };
                match result {
                    Ok((stream, info)) => Some(Output {
                        feeder: feeder_slot.expect("feeder"),
                        info,
                        backend: Backend::Cpal(stream),
                    }),
                    Err(e) => {
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
        let coeffs = EqCoeffs::from_settings(&self.eq, out.feeder.rate());
        out.feeder.command(RtCommand::SetEq(coeffs));
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
            Some(Output {
                backend: Backend::Cpal(_),
                info,
                ..
            }) => {
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
