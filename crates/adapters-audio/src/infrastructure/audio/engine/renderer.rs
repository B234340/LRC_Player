//! Realtime side of the engine.
//!
//! The [`Renderer`] runs inside the audio callback (or a test/manual driver). It is fed by the
//! engine thread through three lock-free SPSC queues ([`Feeder`]):
//! - `samples`: interleaved f32 at the device rate/channel count;
//! - `markers`: positional events keyed by absolute sample index (`TrackStart`, `End`);
//! - `cmds`: immediate commands (`Flush`, `SetPaused`, DSP settings, per-track gains).
//!
//! The renderer never allocates, never locks, and never makes syscalls (apart from reading the
//! monotonic clock, which is vDSO / QueryPerformanceCounter). It owns the transport envelope
//! (fade in/out for pause/resume/seek/flush/underrun), smoothed volume and mute, the
//! per-track normalization gain, the filter chain and limiter, sample-accurate position
//! bookkeeping, and the spectrum tap.
//!
//! Per-sample chain: `x * norm` -> graphic EQ (+preamp) -> parametric EQ (+preamp) ->
//! crossfeed -> auto headroom -> `* fade envelope * volume` -> limiter (soft knee whenever
//! anything can boost, else a hard clamp that is the identity inside ±1). With every stage
//! off/unity the samples pass bit-exactly.
//!
//! Click protection: every transport change is a gain ramp on the real signal (pause, resume,
//! flush, starvation, priming). Where audio stops before a ramp can complete (the ring ran dry
//! exactly at a callback boundary, a flush with too little stale audio left, the end of the
//! last track on a non-zero sample) a short *declick tail* - a linear ramp from the last output
//! value to zero - is mixed in, so the output never steps. Filter-chain changes are
//! crossfaded; normalization changes on the audible track are ramped (~300 ms) and switch
//! sample-exactly at gapless boundaries (the gain is keyed by track sequence number).

use std::sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use rtrb::{Consumer, Producer, RingBuffer};

use super::crossfeed::CrossfeedCoeffs;
use super::dsp::{hard_limit, soft_limit, soft_limit_engaged, ChainCoeffs, ChainState, EqCoeffs};
use super::peq::PeqCoeffs;

/// Positional marker. `at` is an absolute index into the sample stream (in samples, i.e.
/// frames * channels) counted from the creation of the ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Marker {
    /// From `at` on, the stream plays track `seq`, starting at track frame `base_frames`
    /// (device-rate frames from the start of the track).
    TrackStart {
        gen: u64,
        at: u64,
        seq: u64,
        base_frames: u64,
    },
    /// No more audio after `at` (unless a `TrackStart` at the same index follows).
    End { gen: u64, at: u64 },
}

impl Marker {
    fn gen(&self) -> u64 {
        match *self {
            Marker::TrackStart { gen, .. } | Marker::End { gen, .. } => gen,
        }
    }
    fn at(&self) -> u64 {
        match *self {
            Marker::TrackStart { at, .. } | Marker::End { at, .. } => at,
        }
    }
}

/// Immediate command for the renderer.
// The filter coefficients travel by value (~0.5 KB) on purpose: boxing them would allocate on
// the engine thread and free in the audio callback.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum RtCommand {
    /// Fade out, then discard everything before sample index `until` and switch to `gen`.
    Flush {
        gen: u64,
        until: u64,
    },
    SetPaused(bool),
    SetEq(EqCoeffs),
    SetPeq(PeqCoeffs),
    SetCrossfeed(CrossfeedCoeffs),
    /// Linear auto-headroom gain.
    SetHeadroom(f32),
    /// Normalization gain (linear) and resampling flag of track `seq`. Applied exactly at the
    /// track's `TrackStart`; for the audible track the gain ramps to the new value.
    SetTrackParams {
        seq: u64,
        gain: f32,
        resampled: bool,
    },
}

/// Seqlock-protected playback clock published by the renderer.
#[derive(Default)]
pub(crate) struct Clock {
    seq: AtomicU64,
    track_seq: AtomicU64,
    gen: AtomicU64,
    base: AtomicU64,
    pos_end: AtomicU64,
    anchor_ns: AtomicU64,
    rate: AtomicU64,
}

/// A consistent copy of the clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClockReading {
    pub(crate) track_seq: u64,
    pub(crate) gen: u64,
    /// Track frame where the current segment (track start or seek target) begins.
    pub(crate) base: u64,
    /// Track frame just after the last frame handed to the device.
    pub(crate) pos_end: u64,
    /// Time (ns since engine epoch) at which frame `pos_end` would be heard.
    pub(crate) anchor_ns: u64,
    pub(crate) rate: u64,
}

impl ClockReading {
    /// Track frame audible at `now_ns` (output latency compensated, interpolated between
    /// callbacks, never before the segment start nor beyond what was rendered).
    pub(crate) fn audible_frames(&self, now_ns: u64) -> u64 {
        if self.anchor_ns <= now_ns || self.rate == 0 {
            return self.pos_end;
        }
        let lag =
            ((self.anchor_ns - now_ns) as u128 * self.rate as u128 / 1_000_000_000u128) as u64;
        self.pos_end
            .saturating_sub(lag)
            .max(self.base.min(self.pos_end))
    }

    pub(crate) fn audible_ms(&self, now_ns: u64) -> u64 {
        if self.rate == 0 {
            return 0;
        }
        self.audible_frames(now_ns) * 1000 / self.rate
    }
}

/// Makes the reported position monotonic within one segment.
///
/// The latency-compensated reading can step backwards: on resume after a pause (the first
/// callback's frames are only heard after the output latency, while the paused reading already
/// showed every rendered frame) and when a callback arrives late or the reported latency
/// jitters. A UI slider / lyrics view must never move backwards within a segment.
#[derive(Debug, Default)]
pub(crate) struct PositionGuard {
    key: Option<(u64, u64)>,
    pos: u64,
}

impl PositionGuard {
    /// `key` identifies the segment (generation, track); `None` resets the guard.
    pub(crate) fn apply(&mut self, key: Option<(u64, u64)>, pos: u64) -> u64 {
        match key {
            Some(k) if self.key == Some(k) => {
                self.pos = self.pos.max(pos);
                self.pos
            }
            _ => {
                self.key = key;
                self.pos = pos;
                pos
            }
        }
    }
}

impl Clock {
    fn publish(&self, r: ClockReading) {
        let s = self.seq.load(Ordering::Relaxed);
        self.seq.store(s.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        self.track_seq.store(r.track_seq, Ordering::Relaxed);
        self.gen.store(r.gen, Ordering::Relaxed);
        self.base.store(r.base, Ordering::Relaxed);
        self.pos_end.store(r.pos_end, Ordering::Relaxed);
        self.anchor_ns.store(r.anchor_ns, Ordering::Relaxed);
        self.rate.store(r.rate, Ordering::Relaxed);
        self.seq.store(s.wrapping_add(2), Ordering::Release);
    }

    pub(crate) fn read(&self) -> ClockReading {
        let mut spins = 0u32;
        loop {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 & 1 == 1 {
                // The writer (audio callback) is mid-publish. It finishes within nanoseconds
                // unless it was preempted; then yield instead of burning the reader's core.
                spins += 1;
                if spins > 64 {
                    std::thread::yield_now();
                } else {
                    std::hint::spin_loop();
                }
                continue;
            }
            let r = ClockReading {
                track_seq: self.track_seq.load(Ordering::Relaxed),
                gen: self.gen.load(Ordering::Relaxed),
                base: self.base.load(Ordering::Relaxed),
                pos_end: self.pos_end.load(Ordering::Relaxed),
                anchor_ns: self.anchor_ns.load(Ordering::Relaxed),
                rate: self.rate.load(Ordering::Relaxed),
            };
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s1 {
                return r;
            }
        }
    }
}

pub(crate) struct TapInner {
    buf: Box<[AtomicU32]>,
    write: AtomicU64,
    rate: AtomicU32,
}

/// Lock-free tap of the most recent post-gain mono samples (for a spectrum visualizer).
/// Cloning is cheap; reads never block the audio thread.
#[derive(Clone)]
pub struct SpectrumTap {
    inner: Arc<TapInner>,
}

impl SpectrumTap {
    pub(crate) fn new(capacity_pow2: usize) -> Self {
        let cap = capacity_pow2.next_power_of_two().max(1024);
        let buf: Vec<AtomicU32> = (0..cap).map(|_| AtomicU32::new(0)).collect();
        Self {
            inner: Arc::new(TapInner {
                buf: buf.into_boxed_slice(),
                write: AtomicU64::new(0),
                rate: AtomicU32::new(0),
            }),
        }
    }

    /// Number of samples the tap retains.
    pub fn capacity(&self) -> usize {
        self.inner.buf.len()
    }

    /// Sample rate of the tapped signal (device rate); 0 before any output was opened.
    pub fn sample_rate(&self) -> u32 {
        self.inner.rate.load(Ordering::Relaxed)
    }

    /// Total samples written so far (lets a reader detect whether new audio arrived).
    pub fn total_written(&self) -> u64 {
        self.inner.write.load(Ordering::Acquire)
    }

    /// Copies the most recent `min(out.len(), capacity)` samples into `out` (oldest first,
    /// newest last). Returns the number of samples copied; if fewer samples than requested
    /// have ever been written, the copy is right-aligned and the front is zero-filled.
    pub fn read_latest(&self, out: &mut [f32]) -> usize {
        let cap = self.inner.buf.len();
        let n = out.len().min(cap);
        let w = self.inner.write.load(Ordering::Acquire);
        let avail = (w as usize).min(n);
        let start = out.len() - avail;
        out[..start].iter_mut().for_each(|s| *s = 0.0);
        let mask = cap - 1;
        let first = w - avail as u64;
        for (i, slot) in out[start..].iter_mut().enumerate() {
            let idx = (first as usize + i) & mask;
            *slot = f32::from_bits(self.inner.buf[idx].load(Ordering::Relaxed));
        }
        avail
    }
}

/// State shared between the engine thread, the public handle and the renderer.
pub(crate) struct RtShared {
    epoch: Instant,
    pub(crate) volume_bits: AtomicU32,
    pub(crate) muted: AtomicBool,
    pub(crate) underruns: AtomicU64,
    pub(crate) callbacks: AtomicU64,
    pub(crate) clock: Clock,
    pub(crate) active_gen: AtomicU64,
    pub(crate) primed_gen: AtomicU64,
    pub(crate) ended_gen: AtomicU64,
    /// Index of the processed `End` marker + 1 (0 = none).
    pub(crate) ended_at: AtomicU64,
    pub(crate) tap: SpectrumTap,
    /// The signal is being altered (gain != 1, EQ, resampling, fades...): integer output
    /// formats are dithered only while this is set, so untouched audio stays bit-perfect.
    /// Written by the renderer at the end of every callback: true when any sample rendered in
    /// that callback was changed (any gain != 1 incl. fades/volume/normalization/headroom, an
    /// active EQ/PEQ/crossfeed or crossfade, a resampled track, the declick tail, or the
    /// limiter); false for untouched audio and for pure silence.
    pub(crate) processing_active: AtomicBool,
    /// `now_ns()` of the last callback in which the limiter changed samples (0 = never).
    pub(crate) limiter_ns: AtomicU64,
}

impl RtShared {
    pub(crate) fn new(volume: f32) -> Self {
        Self {
            epoch: Instant::now(),
            volume_bits: AtomicU32::new(volume.to_bits()),
            muted: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            callbacks: AtomicU64::new(0),
            clock: Clock::default(),
            active_gen: AtomicU64::new(0),
            primed_gen: AtomicU64::new(0),
            ended_gen: AtomicU64::new(0),
            ended_at: AtomicU64::new(0),
            tap: SpectrumTap::new(8192),
            processing_active: AtomicBool::new(false),
            limiter_ns: AtomicU64::new(0),
        }
    }

    pub(crate) fn now_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    pub(crate) fn set_volume(&self, v: f32) {
        let v = if v.is_finite() {
            v.clamp(0.0, 2.0)
        } else {
            1.0
        };
        self.volume_bits.store(v.to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }
}

/// Renderer timing parameters.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RenderConfig {
    pub(crate) rate: u32,
    pub(crate) channels: usize,
    pub(crate) ring_frames: usize,
    pub(crate) prebuffer_frames: usize,
    pub(crate) fade_frames: usize,
}

const LOCAL_MARKERS: usize = 64;
/// Tracks whose parameters the renderer remembers (the audible one plus queued successors).
const TRACK_PARAMS: usize = 16;
/// Largest frame (channels) crossfaded sample-by-sample between two filter chains; wider
/// devices switch chains without a crossfade.
const XFADE_MAX_CHANNELS: usize = 32;

/// Per-track parameters keyed by sequence number (0 = empty slot).
#[derive(Clone, Copy, Debug, PartialEq)]
struct TrackParams {
    seq: u64,
    gain: f32,
    resampled: bool,
}
const CMD_QUEUE: usize = 256;
const MARKER_QUEUE: usize = 256;

/// Fixed-capacity FIFO of markers owned by the renderer (no allocation after creation).
struct MarkerQueue {
    items: [Marker; LOCAL_MARKERS],
    head: usize,
    len: usize,
}

impl MarkerQueue {
    fn new() -> Self {
        Self {
            items: [Marker::End { gen: 0, at: 0 }; LOCAL_MARKERS],
            head: 0,
            len: 0,
        }
    }
    fn is_full(&self) -> bool {
        self.len == LOCAL_MARKERS
    }
    fn push(&mut self, m: Marker) {
        debug_assert!(!self.is_full());
        self.items[(self.head + self.len) % LOCAL_MARKERS] = m;
        self.len += 1;
    }
    fn get(&self, i: usize) -> Option<Marker> {
        (i < self.len).then(|| self.items[(self.head + i) % LOCAL_MARKERS])
    }
    fn front(&self) -> Option<Marker> {
        self.get(0)
    }
    fn pop(&mut self) {
        if self.len > 0 {
            self.head = (self.head + 1) % LOCAL_MARKERS;
            self.len -= 1;
        }
    }
    /// Drops markers of generations older than `gen` (in place, order preserved).
    fn drop_older_than(&mut self, gen: u64) {
        let mut kept = 0;
        for i in 0..self.len {
            let m = self.items[(self.head + i) % LOCAL_MARKERS];
            if m.gen() >= gen {
                self.items[(self.head + kept) % LOCAL_MARKERS] = m;
                kept += 1;
            }
        }
        self.len = kept;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Waiting for enough audio (after load/seek/underrun); outputs silence.
    Priming,
    Playing,
    /// Reached an `End` marker; outputs silence until new data for this generation arrives.
    Ended,
}

/// Producer side of the renderer queues (owned by the engine thread).
pub(crate) struct Feeder {
    samples: Producer<f32>,
    markers: Producer<Marker>,
    cmds: Producer<RtCommand>,
    written: u64,
    pending: Vec<f32>,
    pending_off: usize,
    channels: usize,
    rate: u32,
    capacity: usize,
}

impl Feeder {
    pub(crate) fn channels(&self) -> usize {
        self.channels
    }

    pub(crate) fn rate(&self) -> u32 {
        self.rate
    }

    /// Sample index where the next pushed sample will land.
    pub(crate) fn logical_written(&self) -> u64 {
        self.written + (self.pending.len() - self.pending_off) as u64
    }

    pub(crate) fn has_pending(&self) -> bool {
        self.pending_off < self.pending.len()
    }

    /// Free space in the ring (samples).
    pub(crate) fn free(&self) -> usize {
        self.samples.slots()
    }

    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    /// Queues samples; whatever does not fit is kept and written by [`Feeder::drain_pending`].
    pub(crate) fn push(&mut self, data: &[f32]) {
        if data.is_empty() {
            return;
        }
        if self.has_pending() {
            self.pending.extend_from_slice(data);
            self.drain_pending();
            return;
        }
        let (done, rest) = self.samples.push_partial_slice(data);
        self.written += done.len() as u64;
        if !rest.is_empty() {
            self.pending.clear();
            self.pending_off = 0;
            self.pending.extend_from_slice(rest);
        }
    }

    /// Moves pending samples into the ring. Returns true when nothing is pending anymore.
    pub(crate) fn drain_pending(&mut self) -> bool {
        if !self.has_pending() {
            return true;
        }
        let (done, _rest) = self
            .samples
            .push_partial_slice(&self.pending[self.pending_off..]);
        let n = done.len();
        self.written += n as u64;
        self.pending_off += n;
        if self.pending_off >= self.pending.len() {
            self.pending.clear();
            self.pending_off = 0;
            true
        } else {
            false
        }
    }

    /// Discards pending samples and tells the renderer to drop everything written so far.
    pub(crate) fn flush(&mut self, gen: u64) -> bool {
        self.pending.clear();
        self.pending_off = 0;
        self.command(RtCommand::Flush {
            gen,
            until: self.written,
        })
    }

    pub(crate) fn marker(&mut self, m: Marker) -> bool {
        self.markers.push(m).is_ok()
    }

    pub(crate) fn command(&mut self, c: RtCommand) -> bool {
        self.cmds.push(c).is_ok()
    }
}

/// Creates a connected feeder/renderer pair.
pub(crate) fn new_pair(cfg: RenderConfig, shared: Arc<RtShared>) -> (Feeder, Renderer) {
    let ch = cfg.channels.max(1);
    let cap = cfg.ring_frames.max(256) * ch;
    let (sp, sc) = RingBuffer::<f32>::new(cap);
    let (mp, mc) = RingBuffer::<Marker>::new(MARKER_QUEUE);
    let (cp, cc) = RingBuffer::<RtCommand>::new(CMD_QUEUE);
    shared.tap.inner.rate.store(cfg.rate, Ordering::Relaxed);
    let tap_w = shared.tap.inner.write.load(Ordering::Relaxed);
    let fade = cfg.fade_frames.max(1);
    let feeder = Feeder {
        samples: sp,
        markers: mp,
        cmds: cp,
        written: 0,
        pending: Vec::with_capacity(16 * 1024),
        pending_off: 0,
        channels: ch,
        rate: cfg.rate,
        capacity: cap,
    };
    let renderer = Renderer {
        rate: cfg.rate,
        ch,
        samples: sc,
        markers_rx: mc,
        cmds: cc,
        local: MarkerQueue::new(),
        shared,
        read_total: 0,
        gen: 0,
        mode: Mode::Priming,
        paused: false,
        pending_flush: None,
        skip_until: 0,
        env: 0.0,
        env_step: 1.0 / fade as f32,
        fade_frames: fade,
        vol: f32::NAN,
        // One-pole smoother, ~10 ms time constant.
        vol_coef: 1.0 - (-1.0 / (cfg.rate.max(1) as f32 * 0.010)).exp(),
        fx: ChainState::new(),
        fx_old: ChainState::new(),
        fx_target: ChainCoeffs::BYPASS,
        fx_xfade_left: 0,
        // ~20 ms crossfade between the old and new chain (both run meanwhile).
        fx_xfade_len: (cfg.rate as usize / 50).max(1),
        params: [TrackParams {
            seq: 0,
            gain: 1.0,
            resampled: false,
        }; TRACK_PARAMS],
        norm: 1.0,
        norm_target: 1.0,
        norm_step: 0.0,
        // ~300 ms normalization ramp.
        norm_ramp: (cfg.rate as usize * 3 / 10).max(1),
        resampled: false,
        touched: false,
        limited: false,
        track_seq: 0,
        base: 0,
        track_pos: 0,
        // The engine thread stops refilling at 15/16 of the ring, so a prebuffer close to the
        // ring size could never be reached (stuck priming). Keep it at most half the ring.
        prebuffer_samples: cfg.prebuffer_frames.min(cap / ch / 2) * ch,
        tap_w,
        realtime_clock: true,
        last_raw: vec![0.0; ch].into_boxed_slice(),
        cut_from: vec![0.0; ch].into_boxed_slice(),
        cut_at: None,
        tail_val: vec![0.0; ch].into_boxed_slice(),
        tail_dec: vec![0.0; ch].into_boxed_slice(),
        tail_left: 0,
    };
    (feeder, renderer)
}

/// The realtime renderer. See module docs.
pub struct Renderer {
    rate: u32,
    ch: usize,
    samples: Consumer<f32>,
    markers_rx: Consumer<Marker>,
    cmds: Consumer<RtCommand>,
    local: MarkerQueue,
    shared: Arc<RtShared>,
    read_total: u64,
    gen: u64,
    mode: Mode,
    paused: bool,
    pending_flush: Option<(u64, u64)>,
    skip_until: u64,
    env: f32,
    env_step: f32,
    fade_frames: usize,
    vol: f32,
    vol_coef: f32,
    /// Filter chain (graphic EQ, parametric EQ, crossfeed, headroom).
    fx: ChainState,
    /// Previous chain (with its filter state) while crossfading to `fx`.
    fx_old: ChainState,
    /// Chain requested by the engine thread; `fx` follows it (crossfaded) when they differ.
    fx_target: ChainCoeffs,
    fx_xfade_left: usize,
    fx_xfade_len: usize,
    params: [TrackParams; TRACK_PARAMS],
    /// Normalization gain (linear) of the audible track, ramping towards `norm_target`.
    norm: f32,
    norm_target: f32,
    norm_step: f32,
    norm_ramp: usize,
    /// The audible track is resampled (for `processing_active`).
    resampled: bool,
    /// Some sample of this callback was changed (published as `processing_active`).
    touched: bool,
    /// The limiter changed some sample of this callback.
    limited: bool,
    track_seq: u64,
    base: u64,
    track_pos: u64,
    prebuffer_samples: usize,
    tap_w: u64,
    realtime_clock: bool,
    /// Last frame written by the previous `render` call (before the declick tail was mixed in).
    last_raw: Box<[f32]>,
    /// Declick: frame index (in the current call) where audio stopped abruptly, and the
    /// output value just before it.
    cut_from: Box<[f32]>,
    cut_at: Option<usize>,
    /// Declick tail in progress: a linear ramp from the last audible value to 0, mixed into
    /// the output so a hard stop (underrun, flush without enough audio to fade on) never
    /// produces a step.
    tail_val: Box<[f32]>,
    tail_dec: Box<[f32]>,
    tail_left: usize,
}

impl Renderer {
    /// With `false`, rendered frames count as audible immediately (manual/offline drivers).
    pub(crate) fn set_realtime_clock(&mut self, v: bool) {
        self.realtime_clock = v;
    }

    /// Samples available in the ring.
    #[allow(dead_code)]
    pub(crate) fn available(&self) -> usize {
        self.samples.slots()
    }

    pub(crate) fn channels(&self) -> usize {
        self.ch
    }

    #[allow(dead_code)]
    pub(crate) fn rate(&self) -> u32 {
        self.rate
    }

    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.cmds.pop() {
            match cmd {
                RtCommand::Flush { gen, until } => self.pending_flush = Some((gen, until)),
                RtCommand::SetPaused(p) => self.paused = p,
                RtCommand::SetEq(c) => self.fx_target.eq = c,
                RtCommand::SetPeq(c) => self.fx_target.peq = c,
                RtCommand::SetCrossfeed(c) => self.fx_target.xfeed = c,
                RtCommand::SetHeadroom(g) => {
                    self.fx_target.headroom = if g.is_finite() {
                        g.clamp(0.0, 1.0)
                    } else {
                        1.0
                    }
                }
                RtCommand::SetTrackParams {
                    seq,
                    gain,
                    resampled,
                } => self.set_track_params(seq, gain, resampled),
            }
        }
        self.update_chain();
    }

    /// Moves the filter chain towards `fx_target`. While audio is audible the old and new
    /// chains are crossfaded (a hard coefficient/gain switch is an audible click); while silent
    /// it switches at once. A target arriving during a crossfade is applied after it.
    fn update_chain(&mut self) {
        if self.fx_xfade_left > 0 || *self.fx.coeffs() == self.fx_target {
            return;
        }
        if self.mode != Mode::Playing || self.env <= 0.0 || self.ch > XFADE_MAX_CHANNELS {
            self.fx.set(self.fx_target);
            return;
        }
        self.fx_old = self.fx;
        self.fx.set(self.fx_target);
        self.fx_xfade_left = self.fx_xfade_len;
    }

    fn set_track_params(&mut self, seq: u64, gain: f32, resampled: bool) {
        let gain = if gain.is_finite() && gain >= 0.0 {
            gain
        } else {
            1.0
        };
        let p = TrackParams {
            seq,
            gain,
            resampled,
        };
        // Update in place, else replace the oldest (smallest seq: sequence numbers only grow).
        let slot = match self.params.iter().position(|t| t.seq == seq) {
            Some(i) => i,
            None => {
                let mut oldest = 0;
                for (i, t) in self.params.iter().enumerate() {
                    if t.seq < self.params[oldest].seq {
                        oldest = i;
                    }
                }
                oldest
            }
        };
        self.params[slot] = p;
        if seq == self.track_seq {
            self.resampled = resampled;
            self.ramp_norm(gain);
        }
    }

    /// Ramps the audible track's normalization gain to `gain` (at once while silent).
    fn ramp_norm(&mut self, gain: f32) {
        self.norm_target = gain;
        if self.mode != Mode::Playing || self.env <= 0.0 {
            self.norm = gain;
            self.norm_step = 0.0;
        } else {
            self.norm_step = (gain - self.norm) / self.norm_ramp as f32;
        }
    }

    /// A new track starts at this sample: its gain applies from here on, without a ramp.
    fn enter_track(&mut self, seq: u64) {
        let p = self
            .params
            .iter()
            .find(|t| t.seq == seq)
            .copied()
            .unwrap_or(TrackParams {
                seq,
                gain: 1.0,
                resampled: false,
            });
        self.norm = p.gain;
        self.norm_target = p.gain;
        self.norm_step = 0.0;
        self.resampled = p.resampled;
    }

    /// Records that audio stops abruptly at frame `k` of `out` (a declick tail starts there).
    fn mark_cut(&mut self, out: &[f32], k: usize) {
        let ch = self.ch;
        let mut audible = false;
        for c in 0..ch {
            let v = if k > 0 {
                out[(k - 1) * ch + c]
            } else {
                self.last_raw[c]
            };
            self.cut_from[c] = v;
            audible |= v.abs() > 1.0e-6;
        }
        if audible || self.tail_left > 0 {
            self.cut_at = Some(k);
        }
    }

    /// Mixes the declick tail into `out` (`frames` frames).
    fn mix_tail(&mut self, out: &mut [f32], frames: usize) {
        let cut = self.cut_at.take();
        if self.tail_left == 0 && cut.is_none() {
            return;
        }
        let ch = self.ch;
        let len = self.fade_frames.max(1);
        let start = match cut {
            Some(c) if self.tail_left == 0 => c,
            _ => 0,
        };
        for j in start..frames {
            if cut == Some(j) {
                let running = self.tail_left > 0;
                for c in 0..ch {
                    let v = self.cut_from[c] + if running { self.tail_val[c] } else { 0.0 };
                    self.tail_val[c] = v;
                    self.tail_dec[c] = v / len as f32;
                }
                self.tail_left = len;
            }
            if self.tail_left == 0 {
                match cut {
                    Some(c) if c > j => continue,
                    _ => break,
                }
            }
            for c in 0..ch {
                let i = j * ch + c;
                out[i] = (out[i] + self.tail_val[c]).clamp(-1.0, 1.0);
                self.tail_val[c] -= self.tail_dec[c];
            }
            self.tail_left -= 1;
        }
        if self.tail_left == 0 {
            self.tail_val.iter_mut().for_each(|v| *v = 0.0);
        }
    }

    fn pull_markers(&mut self) {
        while !self.local.is_full() {
            match self.markers_rx.pop() {
                Ok(m) => self.local.push(m),
                Err(_) => break,
            }
        }
    }

    fn skip_available(&mut self) {
        if self.skip_until > self.read_total {
            let n = ((self.skip_until - self.read_total) as usize).min(self.samples.slots());
            if n > 0 {
                if let Ok(chunk) = self.samples.read_chunk(n) {
                    chunk.commit_all();
                    self.read_total += n as u64;
                }
            }
        }
    }

    fn apply_flush(&mut self, gen: u64, until: u64) {
        self.pending_flush = None;
        self.gen = gen;
        self.skip_until = until.max(self.read_total);
        self.skip_available();
        self.local.drop_older_than(gen);
        self.mode = Mode::Priming;
        self.env = 0.0;
        self.shared.active_gen.store(gen, Ordering::Release);
    }

    /// Applies markers due at the current read position. Returns true if the clock changed.
    fn apply_due_markers(&mut self) -> bool {
        let mut changed = false;
        while let Some(m) = self.local.front() {
            if m.gen() < self.gen {
                self.local.pop();
                continue;
            }
            if m.gen() > self.gen || m.at() > self.read_total {
                break;
            }
            self.local.pop();
            match m {
                Marker::TrackStart {
                    seq, base_frames, ..
                } => {
                    self.track_seq = seq;
                    self.base = base_frames;
                    self.track_pos = base_frames;
                    self.enter_track(seq);
                    if self.mode == Mode::Ended {
                        self.mode = Mode::Priming;
                    }
                    changed = true;
                }
                Marker::End { gen, at } => {
                    let continued = matches!(
                        self.local.front(),
                        Some(Marker::TrackStart { gen: g2, at: a2, .. }) if g2 == gen && a2 == at
                    );
                    if continued {
                        continue;
                    }
                    self.mode = Mode::Ended;
                    self.shared.ended_gen.store(gen, Ordering::Relaxed);
                    self.shared.ended_at.store(at + 1, Ordering::Release);
                    changed = true;
                    break;
                }
            }
        }
        changed
    }

    /// Frames until the next marker of the current generation (if any).
    fn frames_to_next_marker(&self) -> Option<usize> {
        for i in 0..self.local.len {
            let m = self.local.get(i)?;
            if m.gen() == self.gen {
                let d = m.at().saturating_sub(self.read_total) as usize;
                return Some(d / self.ch);
            }
            if m.gen() > self.gen {
                return None;
            }
        }
        None
    }

    fn prime_ready(&self) -> bool {
        let avail = self.samples.slots();
        if avail >= self.prebuffer_samples.max(self.ch) {
            return true;
        }
        let reach = self.read_total + avail as u64;
        for i in 0..self.local.len {
            if let Some(Marker::End { gen, at }) = self.local.get(i) {
                if gen == self.gen && at <= reach {
                    return true;
                }
            }
        }
        false
    }

    #[inline]
    fn tap_write(&mut self, v: f32) {
        let buf = &self.shared.tap.inner.buf;
        let idx = (self.tap_w as usize) & (buf.len() - 1);
        buf[idx].store(v.to_bits(), Ordering::Relaxed);
        self.tap_w += 1;
    }

    /// Pops `n` frames into `dst` and runs the DSP chain on them.
    fn process(&mut self, dst: &mut [f32], n: usize, target: f32, vol_target: f32, starving: bool) {
        let ch = self.ch;
        let count = n * ch;
        match self.samples.read_chunk(count) {
            Ok(chunk) => {
                let (a, b) = chunk.as_slices();
                dst[..a.len()].copy_from_slice(a);
                dst[a.len()..count].copy_from_slice(b);
                chunk.commit_all();
            }
            Err(_) => {
                dst[..count].iter_mut().for_each(|s| *s = 0.0);
            }
        }
        self.read_total += count as u64;
        self.track_pos += n as u64;

        // Starving: fade out over the last available frames at the normal fade slope; if the
        // envelope cannot reach 0 in time, the declick tail in `render` covers the rest.
        let tail = if starving { n.min(self.fade_frames) } else { 0 };
        let tail_start = n - tail;
        let inv_ch = 1.0 / ch as f32;
        let mut touched = self.resampled;
        let mut limited = false;
        for i in 0..n {
            if i >= tail_start {
                self.env = (self.env - self.env_step).max(0.0);
            } else if self.env < target {
                self.env = (self.env + self.env_step).min(target);
            } else if self.env > target {
                self.env = (self.env - self.env_step).max(target);
            }
            if self.vol != vol_target {
                let next = self.vol + (vol_target - self.vol) * self.vol_coef;
                // Near the target the f32 increment rounds away (the smoother would stall a
                // few 1e-5 short forever, so volume 1.0 would never be exact again): snap.
                self.vol = if next == self.vol || (next - vol_target).abs() < 1.0e-5 {
                    vol_target
                } else {
                    next
                };
            }
            if self.norm != self.norm_target {
                self.norm += self.norm_step;
                let done = if self.norm_step > 0.0 {
                    self.norm >= self.norm_target
                } else {
                    self.norm <= self.norm_target
                };
                if done || self.norm_step == 0.0 {
                    self.norm = self.norm_target;
                }
            }
            let g = self.env * self.vol;
            let pre = self.norm;
            let xfade = self.fx_xfade_left > 0;
            let fx_active = self.fx.active();
            let soft = xfade
                || fx_active
                || vol_target > 1.0
                || self.vol > 1.0
                || pre > 1.0
                || self.norm_target > 1.0;
            touched |= xfade || fx_active || g != 1.0 || pre != 1.0;
            let frame = &mut dst[i * ch..(i + 1) * ch];
            let mut mono = 0.0f32;
            if xfade {
                let a = 1.0 - self.fx_xfade_left as f32 / self.fx_xfade_len as f32;
                let mut old = [0.0f32; XFADE_MAX_CHANNELS];
                let old = &mut old[..ch];
                for (o, s) in old.iter_mut().zip(frame.iter_mut()) {
                    *s *= pre;
                    *o = *s;
                }
                self.fx_old.process_frame(old);
                self.fx.process_frame(frame);
                for (s, yo) in frame.iter_mut().zip(old.iter()) {
                    let x = (yo + (*s - yo) * a) * g;
                    limited |= soft_limit_engaged(x);
                    let y = soft_limit(x);
                    *s = y;
                    mono += y;
                }
                self.fx_xfade_left -= 1;
                if self.fx_xfade_left == 0 {
                    self.update_chain();
                }
            } else if fx_active {
                if pre != 1.0 {
                    frame.iter_mut().for_each(|s| *s *= pre);
                }
                self.fx.process_frame(frame);
                for s in frame.iter_mut() {
                    let x = *s * g;
                    limited |= soft_limit_engaged(x);
                    let y = soft_limit(x);
                    *s = y;
                    mono += y;
                }
            } else {
                // `g * 1.0 == g` exactly: unity normalization keeps this path bit-exact.
                let g = g * pre;
                for s in frame.iter_mut() {
                    let x = *s * g;
                    let y = if soft {
                        limited |= soft_limit_engaged(x);
                        soft_limit(x)
                    } else {
                        limited |= x.abs() > 1.0;
                        hard_limit(x)
                    };
                    *s = y;
                    mono += y;
                }
            }
            self.tap_write(mono * inv_ch);
        }
        self.touched |= touched || limited;
        self.limited |= limited;
    }

    /// Renders interleaved device samples into `out`. `latency_ns` is the time from now until
    /// the first sample of `out` reaches the speaker (0 if unknown).
    pub fn render(&mut self, out: &mut [f32], latency_ns: u64) {
        let ch = self.ch;
        let frames = out.len() / ch;
        self.shared.callbacks.fetch_add(1, Ordering::Relaxed);
        let now_ns = self.shared.now_ns();
        // Markers first: every command the engine thread queued before a marker we hold (e.g.
        // a track's gain before its `TrackStart`) is then visible to `drain_commands`.
        self.pull_markers();
        self.drain_commands();
        self.touched = false;
        self.limited = false;

        let muted = self.shared.muted.load(Ordering::Relaxed);
        let vol_target = if muted { 0.0 } else { self.shared.volume() };
        if self.vol.is_nan() {
            self.vol = vol_target;
        }

        let mut k = 0usize;
        let mut track_k_end: Option<usize> = None;
        let mut clock_dirty = false;
        let mut guard = 0;
        while k < frames && guard < 256 {
            guard += 1;
            if self.skip_until > self.read_total {
                self.skip_available();
                if self.skip_until > self.read_total {
                    break;
                }
            }
            if let Some((g, until)) = self.pending_flush {
                if self.env <= 0.0 || self.mode != Mode::Playing {
                    self.apply_flush(g, until);
                    clock_dirty = true;
                    continue;
                }
            }
            let was_ended = self.mode == Mode::Ended;
            if self.apply_due_markers() {
                clock_dirty = true;
            }
            if !was_ended && self.mode == Mode::Ended {
                // End of the last track: a track that does not end at a zero crossing (or
                // whose EQ-filtered output does not) must not stop with a step.
                self.mark_cut(out, k);
            }
            match self.mode {
                Mode::Priming => {
                    if self.prime_ready() {
                        self.mode = Mode::Playing;
                        self.env = 0.0;
                        self.shared.primed_gen.store(self.gen, Ordering::Release);
                        continue;
                    }
                    break;
                }
                Mode::Ended => break,
                Mode::Playing => {
                    let flushing = self.pending_flush.is_some();
                    let target = if self.paused || flushing { 0.0 } else { 1.0 };
                    if target == 0.0 && self.env <= 0.0 {
                        // Paused and fully faded: hold the ring, output silence.
                        break;
                    }
                    let avail = self.samples.slots() / ch;
                    let to_marker = self.frames_to_next_marker();
                    let want = frames - k;
                    let mut n = want.min(avail);
                    if let Some(m) = to_marker {
                        n = n.min(m);
                    }
                    if let Some((_, until)) = self.pending_flush {
                        // Fade out on stale audio only: never consume past the flush point
                        // (the new segment's samples follow it), and stop once silent.
                        let stale = until.saturating_sub(self.read_total) as usize / ch;
                        n = n.min(stale).min(self.fade_frames);
                    }
                    if target == 0.0 {
                        // Fading out (pause/flush): consume only the frames the fade needs.
                        // Anything more would be played silently and lost on resume.
                        let need = (self.env / self.env_step).ceil() as usize;
                        n = n.min(need.max(1));
                    }
                    if n == 0 {
                        if flushing {
                            // Not enough stale audio left to finish the fade on.
                            if self.env > 0.0 {
                                self.mark_cut(out, k);
                            }
                            self.env = 0.0;
                        } else if to_marker != Some(0) {
                            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                            self.mode = Mode::Priming;
                            self.mark_cut(out, k);
                        }
                        continue;
                    }
                    let starving = n == avail && n < want && to_marker.is_none_or(|m| m > n);
                    let dst = &mut out[k * ch..(k + n) * ch];
                    self.process(dst, n, target, vol_target, starving);
                    k += n;
                    track_k_end = Some(k);
                }
            }
        }
        if k < frames {
            out[k * ch..].iter_mut().for_each(|s| *s = 0.0);
            for _ in k..frames {
                self.tap_write(0.0);
            }
            // While silent, let the volume smoother settle on its target.
            self.vol = vol_target;
        } else {
            out[frames * ch..].iter_mut().for_each(|s| *s = 0.0);
        }
        if frames > 0 {
            self.last_raw
                .copy_from_slice(&out[(frames - 1) * ch..frames * ch]);
        }
        if self.tail_left > 0 || self.cut_at.is_some() {
            self.touched = true;
        }
        self.mix_tail(out, frames);
        self.shared
            .processing_active
            .store(self.touched, Ordering::Relaxed);
        if self.limited {
            self.shared
                .limiter_ns
                .store(now_ns.max(1), Ordering::Relaxed);
        }
        self.shared
            .tap
            .inner
            .write
            .store(self.tap_w, Ordering::Release);

        if track_k_end.is_some() || clock_dirty {
            let anchor_ns = match track_k_end {
                Some(_) if !self.realtime_clock => now_ns,
                Some(kend) => {
                    now_ns + latency_ns + (kend as u64 * 1_000_000_000 / self.rate.max(1) as u64)
                }
                None => now_ns,
            };
            self.shared.clock.publish(ClockReading {
                track_seq: self.track_seq,
                gen: self.gen,
                base: self.base,
                pos_end: self.track_pos,
                anchor_ns,
                rate: self.rate as u64,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(prebuffer: usize, fade: usize) -> (Feeder, Renderer, Arc<RtShared>) {
        let shared = Arc::new(RtShared::new(1.0));
        let cfg = RenderConfig {
            rate: 48_000,
            channels: 2,
            ring_frames: 48_000,
            prebuffer_frames: prebuffer,
            fade_frames: fade,
        };
        let (f, r) = new_pair(cfg, shared.clone());
        (f, r, shared)
    }

    fn start(f: &mut Feeder, gen: u64, seq: u64, base: u64) {
        assert!(f.flush(gen));
        assert!(f.marker(Marker::TrackStart {
            gen,
            at: f.logical_written(),
            seq,
            base_frames: base,
        }));
    }

    fn ramp(frames: usize, offset: usize) -> Vec<f32> {
        // Slowly varying, distinct values within ±0.8 (identity range of the limiter).
        (0..frames)
            .flat_map(|i| {
                let v = (((i + offset) % 1000) as f32 / 1000.0) * 0.8 - 0.4;
                [v, -v]
            })
            .collect()
    }

    #[test]
    fn pass_through_is_bit_exact_after_fade_in_and_counts_position() {
        let (mut f, mut r, shared) = pair(256, 64);
        start(&mut f, 1, 1, 0);
        let input = ramp(20_000, 0);
        f.push(&input);
        let end = f.logical_written();
        f.marker(Marker::End { gen: 1, at: end });
        let mut out = Vec::new();
        let mut buf = vec![0.0f32; 2 * 333];
        while out.len() < input.len() {
            r.render(&mut buf, 0);
            out.extend_from_slice(&buf);
        }
        // After the 64-frame fade-in everything is bit-exact and nothing is duplicated/dropped.
        let fade = 64 * 2;
        assert!(out[fade..input.len()]
            .iter()
            .zip(&input[fade..])
            .all(|(a, b)| a.to_bits() == b.to_bits()));
        let c = shared.clock.read();
        assert_eq!(c.track_seq, 1);
        assert_eq!(c.pos_end, 20_000);
        assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn pause_and_resume_are_ramped_and_hold_position() {
        let (mut f, mut r, shared) = pair(64, 96);
        start(&mut f, 1, 1, 0);
        let input: Vec<f32> = vec![0.5; 2 * 20_000];
        f.push(&input);
        let mut buf = vec![0.0f32; 2 * 512];
        for _ in 0..4 {
            r.render(&mut buf, 0);
        }
        assert_eq!(buf[0], 0.5);
        f.command(RtCommand::SetPaused(true));
        let mut out = Vec::new();
        for _ in 0..3 {
            r.render(&mut buf, 0);
            out.extend_from_slice(&buf);
        }
        let max_step = out
            .windows(2)
            .step_by(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step <= 0.5 / 96.0 + 1e-6, "pause step {max_step}");
        assert_eq!(*out.last().unwrap(), 0.0);
        let pos_paused = shared.clock.read().pos_end;
        r.render(&mut buf, 0);
        assert_eq!(
            shared.clock.read().pos_end,
            pos_paused,
            "position must hold while paused"
        );
        f.command(RtCommand::SetPaused(false));
        r.render(&mut buf, 0);
        assert!(buf[0] <= 0.5 / 96.0 + 1e-6);
        let max_step = buf
            .chunks(2)
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step <= 0.5 / 96.0 + 1e-6, "resume step {max_step}");
        assert_eq!(buf[buf.len() - 2], 0.5);
    }

    #[test]
    fn flush_fades_out_skips_stale_audio_and_restarts_segment() {
        let (mut f, mut r, shared) = pair(128, 64);
        start(&mut f, 1, 1, 0);
        f.push(&vec![0.5; 2 * 10_000]);
        let mut buf = vec![0.0f32; 2 * 256];
        r.render(&mut buf, 0);
        r.render(&mut buf, 0);
        // Seek: flush and new segment with different content.
        start(&mut f, 2, 1, 96_000);
        f.push(&vec![-0.25; 2 * 10_000]);
        let mut out = Vec::new();
        for _ in 0..6 {
            r.render(&mut buf, 0);
            out.extend_from_slice(&buf);
        }
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let max_step = left
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step <= 0.5 / 64.0 + 1e-6, "seek step {max_step}");
        assert_eq!(*left.last().unwrap(), -0.25);
        let c = shared.clock.read();
        assert_eq!(c.base, 96_000);
        assert!(c.pos_end > 96_000);
    }

    #[test]
    fn underrun_fades_out_and_is_counted() {
        let (mut f, mut r, shared) = pair(64, 32);
        start(&mut f, 1, 1, 0);
        f.push(&vec![0.5; 2 * 300]);
        let mut buf = vec![0.0f32; 2 * 256];
        r.render(&mut buf, 0);
        r.render(&mut buf, 0);
        // 44 frames remained then starvation: tail must ramp to 0.
        let left: Vec<f32> = buf.iter().step_by(2).copied().collect();
        let max_step = left
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step <= 0.5 / 32.0 + 1e-3, "underrun step {max_step}");
        assert_eq!(shared.underruns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn end_marker_reports_finish_without_underrun() {
        let (mut f, mut r, shared) = pair(10_000, 32);
        start(&mut f, 1, 1, 0);
        f.push(&vec![0.1; 2 * 500]);
        let at = f.logical_written();
        f.marker(Marker::End { gen: 1, at });
        let mut buf = vec![0.0f32; 2 * 256];
        for _ in 0..4 {
            r.render(&mut buf, 0);
        }
        assert_eq!(shared.ended_at.load(Ordering::Relaxed), at + 1);
        assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
        assert_eq!(shared.clock.read().pos_end, 500);
    }

    #[test]
    fn spectrum_tap_returns_latest_samples() {
        let (mut f, mut r, shared) = pair(16, 1);
        start(&mut f, 1, 1, 0);
        let input: Vec<f32> = (0..2048).flat_map(|i| [i as f32 / 4096.0; 2]).collect();
        f.push(&input);
        let mut buf = vec![0.0f32; 2 * 2048];
        r.render(&mut buf, 0);
        let mut tap = vec![0.0f32; 16];
        assert_eq!(shared.tap.read_latest(&mut tap), 16);
        assert_eq!(tap[15], 2047.0 / 4096.0);
        assert_eq!(tap[0], 2032.0 / 4096.0);
    }

    fn left(out: &[f32]) -> Vec<f32> {
        out.iter().step_by(2).copied().collect()
    }

    fn max_abs_step(v: &[f32]) -> f32 {
        v.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    }

    /// The ring runs dry exactly at a callback boundary (no frames left to fade on inside the
    /// callback that consumed the last audio). The next callback must not start with a step
    /// from full level to silence.
    #[test]
    fn underrun_at_callback_boundary_is_declicked() {
        let (mut f, mut r, shared) = pair(64, 32);
        start(&mut f, 1, 1, 0);
        f.push(&vec![0.5; 2 * 256 * 3]);
        let mut out = Vec::new();
        let mut buf = vec![0.0f32; 2 * 256];
        for _ in 0..5 {
            r.render(&mut buf, 0);
            out.extend_from_slice(&buf);
        }
        let l = left(&out);
        let m = max_abs_step(&l);
        assert!(m <= 0.5 / 32.0 + 1e-4, "step at underrun {m}");
        assert_eq!(*l.last().unwrap(), 0.0);
        assert_eq!(shared.underruns.load(Ordering::Relaxed), 1);
    }

    /// A flush (seek/stop) arrives when only a few stale frames remain: the fade-out cannot
    /// complete on stale audio, and must not end in a hard cut.
    #[test]
    fn flush_with_little_stale_audio_is_declicked() {
        let (mut f, mut r, _shared) = pair(64, 32);
        start(&mut f, 1, 1, 0);
        f.push(&vec![0.5; 2 * (256 + 8)]);
        let mut buf = vec![0.0f32; 2 * 256];
        let mut out = Vec::new();
        r.render(&mut buf, 0);
        out.extend_from_slice(&buf);
        start(&mut f, 2, 1, 0);
        f.push(&vec![0.5; 2 * 4000]);
        for _ in 0..4 {
            r.render(&mut buf, 0);
            out.extend_from_slice(&buf);
        }
        let l = left(&out);
        let m = max_abs_step(&l);
        assert!(m <= 0.5 / 32.0 + 1e-4, "step at flush {m}");
        assert_eq!(*l.last().unwrap(), 0.5);
    }

    /// Pausing must consume only the frames the fade-out needs; with large callbacks the old
    /// code consumed (and silently discarded) the whole callback's worth of audio.
    #[test]
    fn pause_consumes_only_the_fade_and_resume_continues_seamlessly() {
        let (mut f, mut r, shared) = pair(64, 32);
        start(&mut f, 1, 1, 0);
        let input = ramp(30_000, 0);
        f.push(&input);
        let mut buf = vec![0.0f32; 2 * 512];
        r.render(&mut buf, 0);
        let before = shared.clock.read().pos_end;
        f.command(RtCommand::SetPaused(true));
        let mut big = vec![0.0f32; 2 * 4096];
        r.render(&mut big, 0);
        let after = shared.clock.read().pos_end;
        assert!(
            after - before <= 33,
            "pause consumed {} frames",
            after - before
        );
        f.command(RtCommand::SetPaused(false));
        r.render(&mut big, 0);
        // After the 32-frame fade-in, playback continues exactly at the next unplayed frame.
        let idx = after as usize;
        for k in 40..200 {
            assert_eq!(big[k * 2], input[(idx + k) * 2], "frame {k}");
        }
    }

    /// Changing the EQ preamp/bands while playing must not switch gains instantaneously.
    #[test]
    fn eq_change_while_playing_is_crossfaded() {
        use super::super::dsp::EqSettings;
        let (mut f, mut r, _shared) = pair(64, 32);
        let quiet = EqSettings {
            enabled: true,
            preamp_db: -12.0,
            ..Default::default()
        };
        f.command(RtCommand::SetEq(EqCoeffs::from_settings(&quiet, 48_000)));
        start(&mut f, 1, 1, 0);
        // 0.3 amplitude 200 Hz sine: max natural step 0.3 * 2π * 200 / 48000 ≈ 0.0079.
        let sine: Vec<f32> = (0..40_000)
            .flat_map(|n| {
                let v = 0.3 * (2.0 * std::f32::consts::PI * 200.0 * n as f32 / 48_000.0).sin();
                [v, v]
            })
            .collect();
        f.push(&sine);
        let mut buf = vec![0.0f32; 2 * 1000];
        let mut out = Vec::new();
        for _ in 0..3 {
            r.render(&mut buf, 0);
            out.extend_from_slice(&buf);
        }
        let mut loud = quiet;
        loud.preamp_db = 0.0;
        loud.bands_db[2] = 12.0;
        f.command(RtCommand::SetEq(EqCoeffs::from_settings(&loud, 48_000)));
        for _ in 0..3 {
            r.render(&mut buf, 0);
            out.extend_from_slice(&buf);
        }
        // Disabling the EQ entirely is crossfaded as well.
        f.command(RtCommand::SetEq(EqCoeffs::BYPASS));
        for _ in 0..3 {
            r.render(&mut buf, 0);
            out.extend_from_slice(&buf);
        }
        let l = left(&out);
        let m = max_abs_step(&l[200..]);
        assert!(m < 0.03, "EQ switch step {m}");
        assert!(l.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    }

    /// A prebuffer larger than what the engine keeps in the ring (it stops refilling at 15/16)
    /// used to leave the renderer priming forever.
    #[test]
    fn oversized_prebuffer_still_primes() {
        let shared = Arc::new(RtShared::new(1.0));
        let cfg = RenderConfig {
            rate: 48_000,
            channels: 2,
            ring_frames: 4096,
            prebuffer_frames: 100_000,
            fade_frames: 16,
        };
        let (mut f, mut r) = new_pair(cfg, shared);
        start(&mut f, 1, 1, 0);
        f.push(&vec![0.25; 2 * 4096 * 15 / 16]);
        let mut buf = vec![0.0f32; 2 * 256];
        r.render(&mut buf, 0);
        assert!(buf.iter().any(|&s| s != 0.0), "renderer never primed");
    }

    /// With real output latency, the raw reading drops back by the latency when playback
    /// resumes after a pause; the guard keeps the reported position monotonic.
    #[test]
    fn resume_after_pause_does_not_move_position_backwards() {
        let (mut f, mut r, shared) = pair(64, 32);
        start(&mut f, 1, 1, 0);
        f.push(&vec![0.25; 2 * 40_000]);
        let latency_ns = 50_000_000;
        let mut buf = vec![0.0f32; 2 * 480];
        for _ in 0..4 {
            r.render(&mut buf, latency_ns);
        }
        f.command(RtCommand::SetPaused(true));
        r.render(&mut buf, latency_ns);
        // Let the pause fade become "audible".
        std::thread::sleep(std::time::Duration::from_millis(80));
        let mut guard = PositionGuard::default();
        let c = shared.clock.read();
        let key = Some((c.gen, c.track_seq));
        let paused_pos = guard.apply(key, c.audible_frames(shared.now_ns()));
        assert_eq!(paused_pos, c.pos_end);
        f.command(RtCommand::SetPaused(false));
        r.render(&mut buf, latency_ns);
        let c = shared.clock.read();
        let raw = c.audible_frames(shared.now_ns());
        assert!(
            raw < paused_pos,
            "raw reading did not go back ({raw} vs {paused_pos})"
        );
        assert_eq!(guard.apply(key, raw), paused_pos);
        // A new segment resets the guard.
        assert_eq!(guard.apply(Some((c.gen + 1, c.track_seq)), 5), 5);
    }
}
