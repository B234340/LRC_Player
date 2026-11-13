//! WASAPI exclusive-mode output (Windows only; cpal only does shared mode).
//!
//! # Design
//! - **Thread**: one dedicated thread per stream (`lrc-wasapi-exclusive`), promoted like the
//!   cpal callback thread (MMCSS "Pro Audio" + time-critical, see `priority.rs`). It owns every
//!   COM object (MTA) and runs the event-driven exclusive loop: wait for the device event, let
//!   the same [`Renderer`] the cpal callback uses fill one period of f32 frames, encode them
//!   (with TPDF dither where it applies, see `dither.rs`) into a preallocated byte buffer and
//!   hand them to the device. Position, gapless markers, fades and events therefore work
//!   exactly as on the shared path. Nothing in the loop allocates, locks or does I/O.
//! - **Format negotiation** ([`candidate_formats`]): the current track's source rate first
//!   (bit-perfect when nothing is processed), then the device mix-format rate; for each rate
//!   the sample encodings I32, I24-in-32, I24 packed, I16, F32, each checked with
//!   `IsFormatSupported` in exclusive mode (with wasapi-rs' channel-mask/WAVEFORMATEX quirks).
//!   Channels are the mix format's. The period is the device default, clamped to
//!   `[device minimum, buffer_ms]` ([`choose_period_hns`]) and aligned to 128 bytes (HDA).
//! - **Rate changes**: an exclusive stream runs at one rate. When a track at another rate
//!   that the device supports (probed once per open, [`needs_rate_reopen`]) is loaded, the
//!   output is reopened at that rate. Gapless playback across different rates is impossible in
//!   exclusive mode: the engine lets the current track play out, closes the stream, reopens at
//!   the new rate and continues with the next track (`AdvancedToNext` is still reported); the
//!   only artefact is a short silence (device drain + reopen + prebuffer). Tracks at the same
//!   rate stay gapless.
//! - **Fallback** ([`ExclusivePolicy`]): any failure (device in use, no supported format,
//!   `AUDCLNT_E_*`, device removed, a stalled stream) closes the exclusive stream and opens
//!   the shared cpal output instead; the reason goes to `tracing::warn` and to
//!   `EngineEvent::ExclusiveUnavailable` (deduplicated). Exclusive is attempted again on the
//!   next output reopen (device change, `set_exclusive(true)`, stream error) and, after a
//!   failure to open, on a track load at most every [`RETRY_AFTER`]. A stream that dies
//!   shortly after opening suppresses exclusive mode for [`SUPPRESS_FOR`] so a flaky device
//!   cannot cause a reopen loop.
//! - **Pause**: the device stays held while paused (the renderer outputs silence). Releasing
//!   it would let another application take the device, so resuming could silently end up in
//!   shared mode, and resume would need a full reopen. Other applications cannot play sound on
//!   this device while the player holds it, paused or not; turn exclusive mode off to share.

// The negotiation/fallback helpers are plain functions so that they are tested on every OS;
// only Windows builds use them.
#![cfg_attr(not(windows), allow(dead_code))]

use std::time::{Duration, Instant};

use super::dither::SampleEncoding;

/// Whether this build can open exclusive streams.
pub(crate) const SUPPORTED: bool = cfg!(windows);

/// Reason reported when exclusive mode is requested on another OS.
pub(crate) const UNSUPPORTED_REASON: &str = "exclusive mode is not supported on this OS";

/// Rates probed at open time to decide later whether a track at another rate is worth a
/// reopen (a rate the device rejects would only reopen at the same fallback rate).
pub(crate) const COMMON_RATES: [u32; 8] = [
    44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000,
];

/// A stream failing within this time after opening counts as "fails right away".
pub(crate) const QUICK_FAILURE: Duration = Duration::from_secs(10);
/// How long exclusive mode is skipped after a stream failed right away.
pub(crate) const SUPPRESS_FOR: Duration = Duration::from_secs(60);
/// Minimum time between exclusive retries triggered by track loads after a failed open.
pub(crate) const RETRY_AFTER: Duration = Duration::from_secs(30);

/// (rate, encoding) pairs in negotiation order: source rate first, then the mix rate; for each
/// rate the encodings of [`SampleEncoding::PREFERENCE`].
pub(crate) fn candidate_formats(
    source_rate: Option<u32>,
    mix_rate: u32,
) -> Vec<(u32, SampleEncoding)> {
    let mut rates: Vec<u32> = Vec::with_capacity(2);
    for r in [source_rate, Some(mix_rate)].into_iter().flatten() {
        if r > 0 && !rates.contains(&r) {
            rates.push(r);
        }
    }
    rates
        .into_iter()
        .flat_map(|r| SampleEncoding::PREFERENCE.into_iter().map(move |e| (r, e)))
        .collect()
}

/// Exclusive-mode period in 100 ns units: the device default period, not above the configured
/// buffer (`buffer_ms`, 0 = no limit) and never below the device minimum.
pub(crate) fn choose_period_hns(default_hns: i64, min_hns: i64, buffer_ms: u32) -> i64 {
    let min_hns = min_hns.max(1);
    let default_hns = if default_hns > 0 {
        default_hns
    } else {
        min_hns
    };
    let wanted = if buffer_ms > 0 {
        default_hns.min(buffer_ms as i64 * 10_000)
    } else {
        default_hns
    };
    wanted.max(min_hns)
}

/// Whether an exclusive output running at `current` Hz must be reopened for a track at `next`
/// Hz: only when the rate differs and the device accepted that rate when it was probed.
pub(crate) fn needs_rate_reopen(current: u32, next: Option<u32>, supported: &[u32]) -> bool {
    next.is_some_and(|r| r != current && supported.contains(&r))
}

/// User-presentable meaning of the WASAPI HRESULTs an exclusive stream typically fails with.
pub(crate) fn describe_hresult(code: u32) -> Option<&'static str> {
    Some(match code {
        0x8889_000A => "the device is in use by another application",
        0x8889_000E => "exclusive mode is disabled for this device (Sound settings)",
        0x8889_0008 => "the device does not support the audio format",
        0x8889_0004 => "the device was removed or disabled",
        0x8889_0026 => "the device's audio resources were invalidated",
        0x8889_000F => "the audio endpoint could not be created",
        0x8889_0010 => "the Windows audio service is not running",
        0x8889_0016 | 0x8889_0019 => "the device rejected the buffer size",
        0x8889_0017 => "the audio engine exceeded its CPU budget",
        0x8007_0005 => "access to the device was denied",
        _ => return None,
    })
}

/// How the next output open should go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Plan {
    /// Try exclusive first (falling back to shared on failure).
    Exclusive,
    /// Open the shared output directly. `Some(reason)`: exclusive was wanted and this reason
    /// has not been reported yet.
    Shared(Option<String>),
}

/// Fallback bookkeeping for exclusive mode (pure state machine; the engine thread feeds it).
#[derive(Debug, Default)]
pub(crate) struct ExclusivePolicy {
    last_reported: Option<String>,
    suppressed_until: Option<Instant>,
    suppress_reason: String,
    opened_at: Option<Instant>,
    last_failed_open: Option<Instant>,
}

impl ExclusivePolicy {
    /// What to do on the next open.
    pub(crate) fn plan(&mut self, want: bool, supported: bool, now: Instant) -> Plan {
        if !want {
            return Plan::Shared(None);
        }
        if !supported {
            return Plan::Shared(self.report(UNSUPPORTED_REASON));
        }
        if let Some(until) = self.suppressed_until {
            if now < until {
                let reason = self.suppress_reason.clone();
                return Plan::Shared(self.report(&reason));
            }
            self.suppressed_until = None;
        }
        Plan::Exclusive
    }

    /// An exclusive open failed (the caller falls back to shared). Returns the reason to
    /// report, unless it was the last one reported.
    pub(crate) fn open_failed(&mut self, reason: &str, now: Instant) -> Option<String> {
        self.opened_at = None;
        self.last_failed_open = Some(now);
        self.report(reason)
    }

    /// An exclusive stream is running.
    pub(crate) fn opened(&mut self, now: Instant) {
        self.opened_at = Some(now);
        self.last_failed_open = None;
        self.last_reported = None;
    }

    /// The running exclusive stream failed (error or stall). If it had only just been opened,
    /// exclusive mode is skipped for [`SUPPRESS_FOR`].
    pub(crate) fn stream_failed(&mut self, reason: &str, now: Instant) {
        if let Some(t) = self.opened_at.take() {
            if now.saturating_duration_since(t) < QUICK_FAILURE {
                self.suppressed_until = Some(now + SUPPRESS_FOR);
                self.suppress_reason =
                    format!("the exclusive stream failed right after starting ({reason})");
            }
        }
    }

    /// After a failed open (we are on the shared fallback), whether a track load may try
    /// exclusive again.
    pub(crate) fn retry_due(&self, now: Instant) -> bool {
        let suppressed = self.suppressed_until.is_some_and(|u| now < u);
        !suppressed
            && self
                .last_failed_open
                .is_some_and(|t| now.saturating_duration_since(t) >= RETRY_AFTER)
    }

    /// The user toggled exclusive mode: forget history so that the outcome is reported again.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    fn report(&mut self, reason: &str) -> Option<String> {
        if self.last_reported.as_deref() == Some(reason) {
            return None;
        }
        self.last_reported = Some(reason.to_string());
        Some(reason.to_string())
    }
}

#[cfg(windows)]
pub(crate) use imp::{open_exclusive, ExclusiveStream};

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::Duration;

    use crossbeam_channel::{bounded, Receiver, Sender};
    use wasapi::{
        calculate_period_100ns, AudioClient, AudioRenderClient, DeviceEnumerator, Direction,
        Handle, SampleType, StreamMode, WasapiError, WaveFormat,
    };

    use super::super::dither::{encode, DitherControl, SampleEncoding, Tpdf};
    use super::super::priority;
    use super::super::renderer::Renderer;
    use super::super::DeviceInfo;
    use super::{candidate_formats, choose_period_hns, describe_hresult, COMMON_RATES};

    /// Longest wait for the stream thread to set up / start the device.
    const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
    /// Event wait slice (also bounds how long dropping the stream waits for the thread).
    const WAIT_SLICE_MS: u32 = 200;
    /// No device event for this long: the stream is considered dead.
    const STALL_MS: u32 = 2000;
    const AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED: u32 = 0x8889_0019;

    /// A running exclusive stream. Dropping it stops the device and joins the thread.
    pub(crate) struct ExclusiveStream {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
        /// Rates the device accepted (with the negotiated encoding) when it was opened.
        pub(crate) supported_rates: Vec<u32>,
    }

    impl ExclusiveStream {
        /// Gives up on a thread stuck in a driver call: it exits on its own later.
        fn detach(&mut self) {
            self.stop.store(true, Ordering::Release);
            self.thread.take();
        }
    }

    impl Drop for ExclusiveStream {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// What the stream thread negotiated with the device.
    struct Negotiated {
        name: String,
        follows_default: bool,
        rate: u32,
        channels: u16,
        enc: SampleEncoding,
        buffer_frames: u32,
        supported_rates: Vec<u32>,
    }

    struct Params {
        preferred: Option<String>,
        buffer_ms: u32,
        source_rate: Option<u32>,
    }

    fn describe(e: &WasapiError) -> String {
        if let WasapiError::Windows(w) = e {
            let code = w.code().0 as u32;
            if let Some(s) = describe_hresult(code) {
                return format!("{s} (0x{code:08X})");
            }
        }
        e.to_string()
    }

    /// Opens `preferred` (or the default device) in exclusive mode and starts a stream driven
    /// by a renderer from `make_renderer(rate, channels)`. On error the reason is returned
    /// (user-presentable) and nothing keeps the device.
    pub(crate) fn open_exclusive(
        preferred: Option<&str>,
        buffer_ms: u32,
        source_rate: Option<u32>,
        make_renderer: &mut dyn FnMut(u32, u16) -> Renderer,
        err_tx: Sender<String>,
        dither: &DitherControl,
    ) -> Result<(ExclusiveStream, DeviceInfo), String> {
        let stop = Arc::new(AtomicBool::new(false));
        let (setup_tx, setup_rx) = bounded::<Result<Negotiated, String>>(1);
        let (renderer_tx, renderer_rx) = bounded::<Renderer>(1);
        let (started_tx, started_rx) = bounded::<Result<(), String>>(1);
        let params = Params {
            preferred: preferred.map(str::to_string),
            buffer_ms,
            source_rate,
        };
        let thread_stop = stop.clone();
        let thread_dither = dither.clone();
        let thread = std::thread::Builder::new()
            .name("lrc-wasapi-exclusive".into())
            .spawn(move || {
                // COM is initialised for this thread only and every COM object lives and dies
                // in `stream_main`, before CoUninitialize.
                let com_ok = wasapi::initialize_mta().is_ok();
                stream_main(
                    params,
                    thread_stop,
                    setup_tx,
                    renderer_rx,
                    started_tx,
                    err_tx,
                    thread_dither,
                );
                if com_ok {
                    wasapi::deinitialize();
                }
            })
            .map_err(|e| format!("cannot start the exclusive audio thread: {e}"))?;
        let mut stream = ExclusiveStream {
            stop,
            thread: Some(thread),
            supported_rates: Vec::new(),
        };
        let neg = match setup_rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                stream.detach();
                return Err("the device did not answer in time".into());
            }
        };
        let renderer = make_renderer(neg.rate, neg.channels);
        if renderer_tx.send(renderer).is_err() {
            return Err("the exclusive audio thread exited".into());
        }
        match started_rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                stream.detach();
                return Err("the device did not start in time".into());
            }
        }
        stream.supported_rates = neg.supported_rates;
        let bits = neg.enc.effective_bits();
        tracing::info!(
            "audio output (exclusive): {} {} Hz {} ch {}, buffer {} frames",
            neg.name,
            neg.rate,
            neg.channels,
            neg.enc.name(),
            neg.buffer_frames
        );
        Ok((
            stream,
            DeviceInfo {
                name: neg.name,
                sample_rate: neg.rate,
                channels: neg.channels,
                sample_format: neg.enc.name().into(),
                buffer_frames: Some(neg.buffer_frames),
                follows_default: neg.follows_default,
                exclusive: true,
                dither: dither.armed(bits),
            },
        ))
    }

    fn wave_format(enc: SampleEncoding, rate: u32, channels: u16) -> WaveFormat {
        let sample_type = if enc.is_float() {
            SampleType::Float
        } else {
            SampleType::Int
        };
        WaveFormat::new(
            enc.container_bits() as usize,
            enc.valid_bits() as usize,
            sample_type,
            rate as usize,
            channels as usize,
            None,
        )
    }

    /// Everything the render loop needs; created and used on the stream thread only.
    struct Device {
        client: AudioClient,
        render: AudioRenderClient,
        event: Handle,
        neg: Negotiated,
    }

    fn setup(p: &Params) -> Result<Device, String> {
        let err = |e: WasapiError| describe(&e);
        let enumerator = DeviceEnumerator::new().map_err(err)?;
        let default = || enumerator.get_default_device(Direction::Render);
        let (device, follows_default) = match p.preferred.as_deref() {
            Some(name) => match enumerator
                .get_device_collection(Direction::Render)
                .and_then(|c| c.get_device_with_name(name))
            {
                Ok(d) => (d, false),
                Err(_) => {
                    tracing::warn!(
                        "preferred output device {name:?} not found; exclusive mode uses the \
                         default device"
                    );
                    (default().map_err(err)?, true)
                }
            },
            None => (default().map_err(err)?, true),
        };
        let name = device
            .get_friendlyname()
            .unwrap_or_else(|_| "<unknown device>".to_string());
        let in_device = |e: WasapiError| format!("{name}: {}", describe(&e));
        let mut client = device.get_iaudioclient().map_err(in_device)?;
        let mix = client.get_mixformat().map_err(in_device)?;
        let mix_rate = mix.get_samplespersec();
        let channels = mix.get_nchannels();

        let mut chosen = None;
        for (rate, enc) in candidate_formats(p.source_rate, mix_rate) {
            let want = wave_format(enc, rate, channels);
            if let Ok(fmt) = client.is_supported_exclusive_with_quirks(&want) {
                chosen = Some((rate, enc, fmt));
                break;
            }
        }
        let Some((rate, enc, fmt)) = chosen else {
            return Err(format!(
                "{name}: the device does not support {} Hz / {mix_rate} Hz with {channels} \
                 channels in exclusive mode",
                p.source_rate.unwrap_or(mix_rate)
            ));
        };
        let supported_rates: Vec<u32> = COMMON_RATES
            .iter()
            .copied()
            .filter(|r| {
                *r == rate
                    || client
                        .is_supported_exclusive_with_quirks(&wave_format(enc, *r, channels))
                        .is_ok()
            })
            .collect();

        let (default_period, min_period) = client.get_device_period().map_err(in_device)?;
        let period = choose_period_hns(default_period, min_period, p.buffer_ms);
        // 128-byte alignment keeps Intel HDA (and most other) drivers happy.
        let period = client
            .calculate_aligned_period_near(period, Some(128), &fmt)
            .map_err(in_device)?;
        let mode = StreamMode::EventsExclusive { period_hns: period };
        match client.initialize_client(&fmt, Direction::Render, mode, None) {
            Ok(()) => {}
            Err(WasapiError::Windows(w))
                if w.code().0 as u32 == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED =>
            {
                // The documented recovery: the client reports the next aligned buffer size;
                // retry with a fresh client and the matching period.
                let frames = client.get_buffer_size().map_err(in_device)?;
                let aligned = calculate_period_100ns(frames as i64, rate as i64);
                client = device.get_iaudioclient().map_err(in_device)?;
                let mode = StreamMode::EventsExclusive {
                    period_hns: aligned,
                };
                client
                    .initialize_client(&fmt, Direction::Render, mode, None)
                    .map_err(in_device)?;
            }
            Err(e) => return Err(in_device(e)),
        }
        let event = client.set_get_eventhandle().map_err(in_device)?;
        let buffer_frames = client.get_buffer_size().map_err(in_device)?;
        let render = client.get_audiorenderclient().map_err(in_device)?;
        Ok(Device {
            client,
            render,
            event,
            neg: Negotiated {
                name,
                follows_default,
                rate,
                channels,
                enc,
                buffer_frames,
                supported_rates,
            },
        })
    }

    /// Per-stream buffers and state of the render loop (allocated before the loop starts).
    struct Loop {
        renderer: Renderer,
        dither: DitherControl,
        tpdf: Tpdf,
        enc: SampleEncoding,
        floats: Vec<f32>,
        bytes: Vec<u8>,
        latency_ns: u64,
    }

    impl Loop {
        /// Renders one device period and hands it to the device.
        fn fill(&mut self, render: &AudioRenderClient) -> Result<(), WasapiError> {
            self.renderer.render(&mut self.floats, self.latency_ns);
            let on = self.dither.active(self.enc.effective_bits());
            encode(self.enc, &self.floats, &mut self.bytes, &mut self.tpdf, on);
            render.write_to_device(&self.bytes, None)
        }
    }

    fn stream_main(
        params: Params,
        stop: Arc<AtomicBool>,
        setup_tx: Sender<Result<Negotiated, String>>,
        renderer_rx: Receiver<Renderer>,
        started_tx: Sender<Result<(), String>>,
        err_tx: Sender<String>,
        dither: DitherControl,
    ) {
        let dev = match setup(&params) {
            Ok(d) => d,
            Err(e) => {
                let _ = setup_tx.send(Err(e));
                return;
            }
        };
        let Device {
            client,
            render,
            event,
            neg,
        } = dev;
        let frames = neg.buffer_frames as usize;
        let ch = neg.channels as usize;
        let rate = neg.rate.max(1) as u64;
        let enc = neg.enc;
        let _ = setup_tx.send(Ok(neg));
        // The engine gave up (timeout) or failed to build a renderer: release the device.
        let Ok(renderer) = renderer_rx.recv() else {
            return;
        };
        let _prio = priority::promote_audio_thread();
        let mut lp = Loop {
            renderer,
            dither,
            tpdf: Tpdf::seeded_from_clock(),
            enc,
            floats: vec![0.0; frames * ch],
            bytes: vec![0; frames * ch * enc.bytes()],
            // The period we fill is played after the one the device is playing now.
            latency_ns: frames as u64 * 1_000_000_000 / rate,
        };
        // Prefill before starting, as WASAPI requires for exclusive event-driven streams.
        if let Err(e) = lp.fill(&render) {
            let _ = started_tx.send(Err(describe(&e)));
            return;
        }
        if let Err(e) = client.start_stream() {
            let _ = started_tx.send(Err(describe(&e)));
            return;
        }
        let _ = started_tx.send(Ok(()));
        let mut waited_ms = 0u32;
        while !stop.load(Ordering::Acquire) {
            if event.wait_for_event(WAIT_SLICE_MS).is_err() {
                waited_ms = waited_ms.saturating_add(WAIT_SLICE_MS);
                if waited_ms >= STALL_MS {
                    let _ = err_tx.try_send("exclusive stream stopped responding".into());
                    break;
                }
                continue;
            }
            waited_ms = 0;
            if stop.load(Ordering::Acquire) {
                break;
            }
            if let Err(e) = lp.fill(&render) {
                let _ = err_tx.try_send(format!("exclusive output: {}", describe(&e)));
                break;
            }
        }
        let _ = client.stop_stream();
        // `lp` (the renderer) and `_prio` drop here, on this thread.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiation_prefers_source_rate_then_mix_rate_and_deep_integer_formats() {
        use SampleEncoding as E;
        let c = candidate_formats(Some(44_100), 48_000);
        let want: Vec<(u32, SampleEncoding)> = [44_100, 48_000]
            .into_iter()
            .flat_map(|r| {
                [E::I32, E::I24In32, E::I24, E::I16, E::F32]
                    .into_iter()
                    .map(move |e| (r, e))
            })
            .collect();
        assert_eq!(c, want);
        // Same rate twice, unknown source rate, or a zero rate: tried once.
        assert_eq!(candidate_formats(Some(48_000), 48_000).len(), 5);
        assert_eq!(candidate_formats(None, 48_000).len(), 5);
        assert_eq!(candidate_formats(Some(0), 48_000)[0], (48_000, E::I32));
    }

    #[test]
    fn period_is_default_clamped_to_buffer_and_device_minimum() {
        // 10 ms default, 3 ms minimum.
        assert_eq!(choose_period_hns(100_000, 30_000, 50), 100_000);
        assert_eq!(choose_period_hns(100_000, 30_000, 5), 50_000);
        assert_eq!(choose_period_hns(100_000, 30_000, 1), 30_000);
        assert_eq!(choose_period_hns(100_000, 30_000, 0), 100_000);
        // Broken driver answers.
        assert_eq!(choose_period_hns(0, 30_000, 50), 30_000);
        assert_eq!(choose_period_hns(0, 0, 0), 1);
    }

    #[test]
    fn rate_reopen_only_for_supported_different_rates() {
        let rates = [44_100, 48_000, 96_000];
        assert!(needs_rate_reopen(48_000, Some(44_100), &rates));
        assert!(!needs_rate_reopen(48_000, Some(48_000), &rates));
        // 88.2k was rejected by the device: reopening would land on the same rate again.
        assert!(!needs_rate_reopen(48_000, Some(88_200), &rates));
        assert!(!needs_rate_reopen(48_000, None, &rates));
    }

    #[test]
    fn hresults_are_described() {
        assert_eq!(
            describe_hresult(0x8889_000A),
            Some("the device is in use by another application")
        );
        assert!(describe_hresult(0x8889_0004).is_some());
        assert!(describe_hresult(0x8889_0008).is_some());
        assert!(describe_hresult(0x8889_000E).is_some());
        assert_eq!(describe_hresult(0x8000_4005), None);
    }

    #[test]
    fn policy_plans_shared_when_not_wanted_or_unsupported_and_reports_once() {
        let now = Instant::now();
        let mut p = ExclusivePolicy::default();
        assert_eq!(p.plan(false, true, now), Plan::Shared(None));
        assert_eq!(
            p.plan(true, false, now),
            Plan::Shared(Some(UNSUPPORTED_REASON.to_string()))
        );
        // Same reason again (e.g. the next device reopen): not reported twice.
        assert_eq!(p.plan(true, false, now), Plan::Shared(None));
        // Toggling the setting reports it again.
        p.reset();
        assert_eq!(
            p.plan(true, false, now),
            Plan::Shared(Some(UNSUPPORTED_REASON.to_string()))
        );
        assert_eq!(p.plan(true, true, now), Plan::Exclusive);
    }

    #[test]
    fn policy_falls_back_on_open_failure_and_retries_later() {
        let t0 = Instant::now();
        let mut p = ExclusivePolicy::default();
        assert_eq!(p.plan(true, true, t0), Plan::Exclusive);
        let busy = "the device is in use by another application";
        assert_eq!(p.open_failed(busy, t0), Some(busy.to_string()));
        // Still exclusive on the next reopen (cheap to try), but not reported again.
        assert_eq!(p.plan(true, true, t0), Plan::Exclusive);
        assert_eq!(p.open_failed(busy, t0), None);
        // A different reason is news.
        assert!(p.open_failed("format", t0).is_some());
        // Loads retry only after RETRY_AFTER.
        assert!(!p.retry_due(t0 + Duration::from_secs(1)));
        assert!(p.retry_due(t0 + RETRY_AFTER));
        // Success clears the failure history: a later failure is reported again.
        p.opened(t0 + RETRY_AFTER);
        assert!(!p.retry_due(t0 + RETRY_AFTER * 3));
        assert_eq!(
            p.open_failed(busy, t0 + RETRY_AFTER * 3),
            Some(busy.to_string())
        );
    }

    #[test]
    fn policy_suppresses_exclusive_after_a_stream_dies_right_away() {
        let t0 = Instant::now();
        let mut p = ExclusivePolicy::default();
        p.opened(t0);
        p.stream_failed("the device was removed", t0 + Duration::from_secs(1));
        let t1 = t0 + Duration::from_secs(2);
        match p.plan(true, true, t1) {
            Plan::Shared(Some(r)) => assert!(r.contains("the device was removed"), "{r}"),
            other => panic!("expected a reported shared fallback, got {other:?}"),
        }
        assert_eq!(p.plan(true, true, t1), Plan::Shared(None));
        assert!(!p.retry_due(t1 + RETRY_AFTER));
        // Suppression ends.
        assert_eq!(p.plan(true, true, t1 + SUPPRESS_FOR), Plan::Exclusive);

        // A stream that ran for a while and then failed (e.g. unplugged) is retried at once.
        let mut p = ExclusivePolicy::default();
        p.opened(t0);
        p.stream_failed("the device was removed", t0 + QUICK_FAILURE * 2);
        assert_eq!(p.plan(true, true, t0 + QUICK_FAILURE * 2), Plan::Exclusive);
        // A failure without a recorded open (stream already replaced) changes nothing.
        let mut p = ExclusivePolicy::default();
        p.stream_failed("x", t0);
        assert_eq!(p.plan(true, true, t0), Plan::Exclusive);
    }

    /// Engine level (no device needed): on this OS exclusive mode is reported unavailable,
    /// every time the user asks for it, and playback state is untouched.
    #[cfg(not(windows))]
    #[test]
    fn set_exclusive_reports_unsupported_os() {
        use super::super::{AudioEngine, EngineConfig, EngineEvent, OutputMode};
        let engine = AudioEngine::new(EngineConfig {
            output: OutputMode::Manual {
                sample_rate: 48_000,
                channels: 2,
            },
            ..Default::default()
        })
        .unwrap();
        let unavailable = |engine: &AudioEngine| {
            engine.sync();
            let mut reasons = Vec::new();
            while let Some(ev) = engine.try_recv_event() {
                if let EngineEvent::ExclusiveUnavailable { reason } = ev {
                    reasons.push(reason);
                }
            }
            reasons
        };
        let _ = unavailable(&engine);
        engine.set_exclusive(true);
        assert_eq!(unavailable(&engine), vec![UNSUPPORTED_REASON.to_string()]);
        engine.set_exclusive(true);
        assert_eq!(unavailable(&engine), vec![UNSUPPORTED_REASON.to_string()]);
        engine.set_exclusive(false);
        assert!(unavailable(&engine).is_empty());
        let dev = engine.snapshot().device.expect("manual output");
        assert!(!dev.exclusive && !dev.dither);
    }
}
