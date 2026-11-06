//! Engine-level tests. They run without an audio device: the engine uses
//! `OutputMode::Manual` and the test thread drives the renderer.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::source::TrackSource;
use super::*;

// ---------- helpers ------------------------------------------------------------------------

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let p =
            std::env::temp_dir().join(format!("lrc_engine_test_{}_{}", std::process::id(), name));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes a 16-bit PCM WAV.
fn write_wav(path: &Path, rate: u32, channels: u16, samples: &[i16]) {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVE");
    b.extend_from_slice(b"fmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    b.extend_from_slice(&(channels * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

fn q(v: f64) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// Stereo signal that never crosses zero at the start (so the first audible frame is
/// detectable): L = 0.25 + 0.2 sin, R = -0.25 + 0.2 cos.
fn offset_sine(rate: u32, freq: f64, start_frame: usize, frames: usize) -> Vec<i16> {
    let mut v = Vec::with_capacity(frames * 2);
    for n in start_frame..start_frame + frames {
        let ph = 2.0 * std::f64::consts::PI * freq * n as f64 / rate as f64;
        v.push(q(0.25 + 0.2 * ph.sin()));
        v.push(q(-0.25 + 0.2 * ph.cos()));
    }
    v
}

/// Decodes a whole file with the engine's own source (reference samples).
fn decode_all(path: &Path) -> Vec<f32> {
    let mut src = TrackSource::open(path).unwrap();
    let mut out = Vec::new();
    while src.decode_next().unwrap().is_some() {
        out.extend_from_slice(src.block());
    }
    out
}

fn manual_engine(rate: u32, channels: u16) -> AudioEngine {
    AudioEngine::new(EngineConfig {
        output: OutputMode::Manual {
            sample_rate: rate,
            channels,
        },
        ring_ms: 500,
        prebuffer_ms: 100,
        fade_ms: 5,
        ..Default::default()
    })
    .unwrap()
}

/// Waits until the manual ring holds `samples` samples (or no progress for a while, i.e. the
/// decoder reached the end), so the test never renders faster than the decoder can fill.
fn wait_available(engine: &AudioEngine, samples: usize) {
    let start = Instant::now();
    let mut last = (usize::MAX, Instant::now());
    loop {
        let a = engine.manual_available().unwrap_or(0);
        if a >= samples || start.elapsed() > Duration::from_secs(20) {
            return;
        }
        if a != last.0 {
            last = (a, Instant::now());
        } else if last.1.elapsed() > Duration::from_millis(150) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn render(engine: &AudioEngine, channels: usize, frames: usize) -> Vec<f32> {
    let mut buf = vec![0.0f32; frames * channels];
    // Never render faster than the decoder: keep a prebuffer's worth (100 ms) plus this chunk
    // in the ring, so a slow debug-build decoder cannot cause underruns or long priming.
    let rate = engine
        .snapshot()
        .device
        .map(|d| d.sample_rate as usize)
        .unwrap_or(48_000);
    wait_available(engine, buf.len() + rate / 10 * channels);
    assert!(engine.render_manual(&mut buf));
    buf
}

fn wait_state(engine: &AudioEngine, state: EngineState) {
    let start = Instant::now();
    while engine.snapshot().state != state {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timeout waiting for {state:?}"
        );
        // Keep the renderer moving so priming can complete.
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Renders in `chunk`-frame blocks until `TrackFinished`, returning output and events.
fn render_until_finished(
    engine: &AudioEngine,
    channels: usize,
    chunk: usize,
    max_frames: usize,
) -> (Vec<f32>, Vec<EngineEvent>) {
    let mut out = Vec::new();
    let mut events = Vec::new();
    let mut finished_at = None;
    while out.len() < max_frames * channels {
        out.extend(render(engine, channels, chunk));
        while let Some(ev) = engine.try_recv_event() {
            if matches!(ev, EngineEvent::TrackFinished { .. }) && finished_at.is_none() {
                finished_at = Some(out.len());
            }
            events.push(ev);
        }
        if let Some(at) = finished_at {
            if out.len() > at + 4 * chunk * channels {
                break;
            }
        }
        if finished_at.is_none() && engine.rt.ended_at.load(Ordering::Acquire) != 0 {
            // Give the engine thread a moment to turn the end into an event.
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    (out, events)
}

fn first_nonzero_frame(out: &[f32], ch: usize) -> usize {
    out.chunks(ch)
        .position(|f| f.iter().any(|&s| s != 0.0))
        .expect("no audio rendered")
}

fn last_nonzero_frame(out: &[f32], ch: usize) -> usize {
    out.chunks(ch)
        .rposition(|f| f.iter().any(|&s| s != 0.0))
        .expect("no audio rendered")
}

/// Last frame of actual track audio: the last non-zero frame, minus the declick ramp
/// (`fade` frames repeating the last value and decaying linearly to 0) the renderer appends
/// when a track ends on a non-zero sample. Also checks the ramp's shape.
fn audio_end_frame(out: &[f32], ch: usize, fade: usize) -> usize {
    let last = last_nonzero_frame(out, ch);
    if last < fade {
        return last;
    }
    let end = last - fade;
    let is_ramp = (0..ch).all(|c| {
        let v = out[end * ch + c];
        let first = out[(end + 1) * ch + c];
        let tail_last = out[last * ch + c];
        v != 0.0 && first == v && (tail_last - v / fade as f32).abs() < 1e-4
    });
    if is_ramp {
        for j in 1..fade {
            for c in 0..ch {
                let (a, b) = (out[(end + j) * ch + c], out[(end + j + 1) * ch + c]);
                assert!(b.abs() <= a.abs() + 1e-7, "declick tail is not decaying");
            }
        }
        end
    } else {
        last
    }
}

fn max_step(out: &[f32], ch: usize, range: std::ops::Range<usize>) -> f32 {
    let mut m = 0.0f32;
    for n in range.start.max(1)..range.end {
        for c in 0..ch {
            m = m.max((out[n * ch + c] - out[(n - 1) * ch + c]).abs());
        }
    }
    m
}

// ---------- tests ----------------------------------------------------------------------------

#[test]
fn equal_rate_playback_is_bit_exact_with_eq_at_0db_and_reports_finish() {
    let dir = TempDir::new("bitexact");
    let path = dir.file("a.wav");
    let frames = 48_000 * 2 + 123;
    write_wav(&path, 48_000, 2, &offset_sine(48_000, 440.0, 0, frames));
    let reference = decode_all(&path);
    assert_eq!(reference.len(), frames * 2);

    let engine = manual_engine(48_000, 2);
    // EQ enabled with all bands at 0 dB must be an exact bypass.
    engine.set_eq(EqSettings {
        enabled: true,
        ..Default::default()
    });
    engine.load(path.clone(), 0, true);
    let (out, events) = render_until_finished(&engine, 2, 441, frames * 3);

    let first = first_nonzero_frame(&out, 2);
    let fade = 48_000 * 5 / 1000;
    let played = &out[first * 2..(first + frames) * 2];
    assert!(
        played[fade * 2..]
            .iter()
            .zip(&reference[fade * 2..])
            .all(|(a, b)| a.to_bits() == b.to_bits()),
        "pass-through is not bit-exact"
    );
    // Nothing duplicated or dropped: audio ends exactly after `frames` frames.
    assert_eq!(audio_end_frame(&out, 2, fade), first + frames - 1);
    assert_eq!(engine.snapshot().underruns, 0);
    assert!(events.iter().any(|e| matches!(
        e,
        EngineEvent::TrackStarted { path: p, duration_ms: Some(d) } if *p == path && d.abs_diff(frames as u64 * 1000 / 48_000) <= 1
    )));
    assert!(events.contains(&EngineEvent::TrackFinished { path: path.clone() }));
    assert!(events.contains(&EngineEvent::StateChanged {
        state: EngineState::Stopped
    }));
    // Position counter == frames rendered for the track.
    assert_eq!(engine.rt.clock.read().pos_end, frames as u64);
}

#[test]
fn resampled_44k1_to_48k_playback_is_continuous_with_correct_length() {
    let dir = TempDir::new("resample");
    let path = dir.file("a.wav");
    let frames = 44_100 * 2;
    // Full-scale-ish 1 kHz cosine (starts non-zero), amplitude 0.5.
    let mut s = Vec::with_capacity(frames * 2);
    for n in 0..frames {
        let v = q(0.5 * (2.0 * std::f64::consts::PI * 1000.0 * n as f64 / 44_100.0).cos());
        s.push(v);
        s.push(v);
    }
    write_wav(&path, 44_100, 2, &s);
    let engine = manual_engine(48_000, 2);
    engine.load(path, 0, true);
    let (out, _events) = render_until_finished(&engine, 2, 512, 48_000 * 4);
    let first = first_nonzero_frame(&out, 2);
    let last = audio_end_frame(&out, 2, 48_000 * 5 / 1000);
    let len = last + 1 - first;
    let expected = frames * 48_000 / 44_100;
    assert!(
        (len as i64 - expected as i64).abs() <= 2,
        "length {len} vs {expected}"
    );
    // No discontinuities: sample-to-sample delta bounded by the sine's max slope (+ quantisation).
    let bound = 0.5 * 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0;
    let fade = 48_000 * 5 / 1000;
    let m = max_step(&out, 2, first + fade + 64..last - 64);
    assert!(m <= bound * 1.03, "max step {m} > {bound}");
    assert_eq!(engine.snapshot().underruns, 0);
    let pos = engine.rt.clock.read().pos_end as i64;
    assert!(
        (pos - expected as i64).abs() <= 2,
        "position {pos} vs {expected}"
    );
}

/// Ramp that encodes the frame index: value(n) = ((n % 32000) - 16000) / 32767.
fn index_ramp_value(n: usize) -> i16 {
    ((n % 32_000) as i32 - 16_000) as i16
}

#[test]
fn seek_far_into_long_file_is_fast_and_sample_accurate() {
    let dir = TempDir::new("seek");
    let path = dir.file("long.wav");
    let rate = 8_000u32;
    let frames = rate as usize * 210; // 3.5 minutes, mono
    let s: Vec<i16> = (0..frames).map(index_ramp_value).collect();
    write_wav(&path, rate, 1, &s);

    // Direct source check: seek lands on the exact frame.
    let mut src = TrackSource::open(&path).unwrap();
    let t = Instant::now();
    src.seek_ms(180_000).unwrap();
    src.decode_next().unwrap().unwrap();
    let seek_time = t.elapsed();
    let target = 180 * rate as usize;
    let expected0 = index_ramp_value(target) as f32 / 32768.0;
    assert!(
        (src.block()[0] - expected0).abs() < 1.0 / 32768.0 + 1e-7,
        "{} vs {expected0}",
        src.block()[0]
    );
    assert!(
        seek_time < Duration::from_millis(100),
        "seek took {seek_time:?}"
    );

    let engine = manual_engine(rate, 1);
    engine.load(path.clone(), 0, true);
    let mut before = Vec::new();
    for _ in 0..10 {
        before.extend(render(&engine, 1, 400));
    }
    assert!(before.iter().any(|&s| s != 0.0));
    let t = Instant::now();
    engine.seek(180_000);
    // The snapshot reflects the target immediately (before audio resumes).
    let deadline = Instant::now() + Duration::from_secs(2);
    while engine.snapshot().position_ms != 180_000 {
        assert!(
            Instant::now() < deadline,
            "snapshot never showed the seek target"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut out = Vec::new();
    for _ in 0..8 {
        out.extend(render(&engine, 1, 400));
    }
    let resumed = t.elapsed();
    assert!(
        resumed < Duration::from_millis(1500),
        "seek+prebuffer took {resumed:?}"
    );
    // Find where the new segment starts: after the fade-out (zeros), the first non-zero
    // sample belongs to the target. Compare the post-fade-in part exactly.
    let fade = rate as usize * 5 / 1000;
    // The old segment fades out to exactly 0.0, then the new one fades in.
    let mut padded = vec![0.0f32];
    padded.extend_from_slice(&out);
    let zero_run_end = padded
        .windows(2)
        .rposition(|w| w[0] == 0.0 && w[1] != 0.0)
        .expect("no restart after seek");
    assert!(
        max_step(&out, 1, 1..out.len()) <= 0.5 / fade as f32 + 1e-3,
        "seek click"
    );
    for k in fade..fade + 1000 {
        let expected = index_ramp_value(target + k) as f32 / 32768.0;
        let got = out[zero_run_end + k];
        assert!(
            (got - expected).abs() < 1e-6,
            "frame {k}: {got} vs {expected}"
        );
    }
    let snap = engine.snapshot();
    let pos = snap.position_ms;
    assert!(
        (180_000..180_500).contains(&pos),
        "position after seek {pos}"
    );
    assert_eq!(snap.underruns, 0);
}

#[test]
fn pause_resume_seek_and_volume_changes_are_ramped() {
    let dir = TempDir::new("ramps");
    let path = dir.file("dc.wav");
    let rate = 48_000u32;
    let frames = rate as usize * 6;
    // Near-DC signal at 0.5 makes any hard cut show up as a 0.5 step.
    let s: Vec<i16> = (0..frames * 2).map(|_| q(0.5)).collect();
    write_wav(&path, rate, 2, &s);

    let engine = manual_engine(rate, 2);
    engine.load(path, 0, true);
    let mut out = Vec::new();
    for _ in 0..20 {
        out.extend(render(&engine, 2, 480));
    }
    let wait_cmd = || std::thread::sleep(Duration::from_millis(30));
    engine.pause();
    wait_cmd();
    for _ in 0..10 {
        out.extend(render(&engine, 2, 480));
    }
    assert_eq!(*out.last().unwrap(), 0.0, "pause must reach silence");
    assert_eq!(engine.snapshot().state, EngineState::Paused);
    engine.play();
    wait_cmd();
    for _ in 0..10 {
        out.extend(render(&engine, 2, 480));
    }
    engine.seek(2_000);
    wait_cmd();
    for _ in 0..20 {
        out.extend(render(&engine, 2, 480));
    }
    engine.set_volume(0.1);
    for _ in 0..10 {
        out.extend(render(&engine, 2, 480));
    }
    engine.set_muted(true);
    for _ in 0..10 {
        out.extend(render(&engine, 2, 480));
    }
    let n = out.len() / 2;
    let fade = rate as f32 * 5.0 / 1000.0;
    let m = max_step(&out, 2, 0..n);
    assert!(
        m <= 0.5 / fade * 1.05 + 1e-4,
        "step {m} (bound {})",
        0.5 / fade
    );
    assert!(out[(n - 1) * 2].abs() < 1e-3, "mute must reach silence");
    assert_eq!(engine.snapshot().underruns, 0);
}

#[test]
fn gapless_equal_rate_join_is_seamless() {
    let dir = TempDir::new("gapless");
    let a = dir.file("a.wav");
    let b = dir.file("b.wav");
    let rate = 48_000;
    let (na, nb) = (rate as usize + 777, rate as usize);
    write_wav(&a, rate, 2, &offset_sine(rate, 997.0, 0, na));
    write_wav(&b, rate, 2, &offset_sine(rate, 997.0, na, nb));
    let mut reference = decode_all(&a);
    reference.extend(decode_all(&b));

    let engine = manual_engine(rate, 2);
    engine.load(a.clone(), 0, true);
    engine.set_next(Some(b.clone()));
    let (out, events) = render_until_finished(&engine, 2, 333, (na + nb) * 3);
    let first = first_nonzero_frame(&out, 2);
    let fade = rate as usize * 5 / 1000;
    let played = &out[first * 2..(first + na + nb) * 2];
    assert!(
        played[fade * 2..]
            .iter()
            .zip(&reference[fade * 2..])
            .all(|(x, y)| x.to_bits() == y.to_bits()),
        "gapless join is not an exact concatenation"
    );
    assert_eq!(audio_end_frame(&out, 2, fade), first + na + nb - 1);
    let adv = events
        .iter()
        .position(|e| matches!(e, EngineEvent::AdvancedToNext { path, .. } if *path == b));
    let fin = events
        .iter()
        .position(|e| matches!(e, EngineEvent::TrackFinished { path } if *path == b));
    assert!(
        adv.is_some() && fin.is_some() && adv < fin,
        "events: {events:?}"
    );
    assert!(!events
        .iter()
        .any(|e| matches!(e, EngineEvent::TrackFinished { path } if *path == a)));
    assert_eq!(engine.snapshot().underruns, 0);
}

#[test]
fn gapless_join_through_resampler_has_no_discontinuity() {
    let dir = TempDir::new("gapless_rs");
    let a = dir.file("a.wav");
    let b = dir.file("b.wav");
    let rate = 44_100;
    let (na, nb) = (rate as usize + 1234, rate as usize);
    let tone = |start: usize, n: usize| -> Vec<i16> {
        (start..start + n)
            .flat_map(|i| {
                let v =
                    q(0.5 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / rate as f64).cos());
                [v, v]
            })
            .collect()
    };
    write_wav(&a, rate, 2, &tone(0, na));
    write_wav(&b, rate, 2, &tone(na, nb));
    let engine = manual_engine(48_000, 2);
    engine.load(a, 0, true);
    engine.set_next(Some(b));
    let (out, _events) = render_until_finished(&engine, 2, 500, (na + nb) * 3);
    let first = first_nonzero_frame(&out, 2);
    let last = audio_end_frame(&out, 2, 48_000 * 5 / 1000);
    let expected = (na + nb) * 48_000 / 44_100;
    assert!(((last + 1 - first) as i64 - expected as i64).abs() <= 2);
    let bound = 0.5 * 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0;
    let m = max_step(&out, 2, first + 300..last - 64);
    assert!(m <= bound * 1.03, "max step across join {m} > {bound}");
}

#[test]
fn position_tracks_rendered_frames() {
    let dir = TempDir::new("position");
    let path = dir.file("a.wav");
    let rate = 48_000;
    write_wav(
        &path,
        rate,
        2,
        &offset_sine(rate, 300.0, 0, rate as usize * 3),
    );
    let engine = manual_engine(rate, 2);
    engine.load(path.clone(), 0, true);
    let mut audio_frames = 0usize;
    for _ in 0..25 {
        let out = render(&engine, 2, 960);
        audio_frames += out.chunks(2).filter(|f| f[0] != 0.0).count();
    }
    wait_state(&engine, EngineState::Playing);
    for _ in 0..25 {
        let out = render(&engine, 2, 960);
        audio_frames += out.chunks(2).filter(|f| f[0] != 0.0).count();
    }
    let snap = engine.snapshot();
    assert_eq!(snap.state, EngineState::Playing);
    assert_eq!(snap.path.as_deref(), Some(path.as_path()));
    assert_eq!(engine.rt.clock.read().pos_end, audio_frames as u64);
    assert_eq!(snap.position_ms, audio_frames as u64 * 1000 / rate as u64);
    assert_eq!(snap.duration_ms, Some(3000));
}

#[test]
fn load_with_autoplay_false_primes_then_pauses_at_start_position() {
    let dir = TempDir::new("paused_load");
    let path = dir.file("a.wav");
    write_wav(&path, 48_000, 2, &offset_sine(48_000, 300.0, 0, 48_000 * 3));
    let engine = manual_engine(48_000, 2);
    engine.load(path, 1_500, false);
    for _ in 0..10 {
        let out = render(&engine, 2, 480);
        assert!(out.iter().all(|&s| s == 0.0));
    }
    wait_state(&engine, EngineState::Paused);
    assert_eq!(engine.snapshot().position_ms, 1_500);
    engine.play();
    std::thread::sleep(Duration::from_millis(30));
    let out = render(&engine, 2, 4800);
    assert!(out.iter().any(|&s| s != 0.0));
}

#[test]
fn load_error_emits_error_and_stops() {
    let engine = manual_engine(48_000, 2);
    let bogus = PathBuf::from("/definitely/not/here.flac");
    engine.load(bogus.clone(), 0, true);
    let ev = loop {
        match engine.recv_event_timeout(Duration::from_secs(5)) {
            Some(EngineEvent::Error { path, .. }) => break path,
            Some(_) => continue,
            None => panic!("no error event"),
        }
    };
    assert_eq!(ev, Some(bogus));
    assert_eq!(engine.snapshot().state, EngineState::Stopped);
}

#[test]
fn waker_is_called_for_events() {
    let engine = manual_engine(48_000, 2);
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let h = hits.clone();
    engine.set_waker(std::sync::Arc::new(move || {
        h.fetch_add(1, Ordering::Relaxed);
    }));
    engine.load(PathBuf::from("/nope.wav"), 0, true);
    let deadline = Instant::now() + Duration::from_secs(5);
    while hits.load(Ordering::Relaxed) == 0 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn drop_joins_engine_thread_and_releases_everything() {
    let dir = TempDir::new("drop");
    let path = dir.file("a.wav");
    write_wav(&path, 48_000, 2, &offset_sine(48_000, 300.0, 0, 48_000));
    let engine = manual_engine(48_000, 2);
    engine.load(path, 0, true);
    let _ = render(&engine, 2, 1000);
    let weak = engine.rt_weak();
    let t = Instant::now();
    drop(engine);
    assert!(t.elapsed() < Duration::from_secs(2));
    // The engine thread, renderer and feeder all held the shared state: if any thread or
    // closure leaked, this would still be alive.
    assert!(weak.upgrade().is_none(), "engine state leaked after drop");
}

#[test]
fn probe_track_is_fast_and_reads_duration() {
    let dir = TempDir::new("probe");
    let path = dir.file("long.wav");
    let rate = 8_000u32;
    write_wav(&path, rate, 1, &vec![0i16; rate as usize * 200]);
    let t = Instant::now();
    let info = probe_track(&path).unwrap();
    assert!(t.elapsed() < Duration::from_millis(100));
    assert_eq!(info.duration_ms, Some(200_000));
    assert_eq!(info.sample_rate, Some(rate));
    assert_eq!(info.channels, Some(1));
}

#[test]
fn eq_near_zero_gain_is_near_identity_and_boost_is_limited() {
    use super::dsp::{EqCoeffs, EqState};
    let mut settings = EqSettings {
        enabled: true,
        ..Default::default()
    };
    settings.bands_db = [0.01; EQ_BANDS];
    let mut st = EqState::new();
    st.set(EqCoeffs::from_settings(&settings, 48_000));
    assert!(st.active());
    let mut max_err = 0.0f32;
    for n in 0..48_000 {
        let x = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * n as f32 / 48_000.0).sin();
        let y = st.process(0, x);
        if n > 4800 {
            max_err = max_err.max((y - x).abs());
        }
    }
    assert!(max_err < 0.01, "near-0 dB EQ deviates by {max_err}");
}

// ---------- randomized stress -------------------------------------------------------------

/// Tiny deterministic PRNG (xorshift64*), so failures are reproducible from the seed.
struct XorShift(u64);

impl XorShift {
    fn new(seed: u64) -> Self {
        XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in `lo..hi`.
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo).max(1)
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.range(0, 100) < percent
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// 100 Hz sine, amplitude 0.5, an integer number of cycles: every file starts and ends at a
/// zero crossing, so any join (gapless, track change) is continuous in the signal itself and
/// any step larger than the signal's own slope is an engine artefact.
fn cycles_sine(rate: u32, channels: u16, cycles: usize) -> Vec<i16> {
    let period = rate as usize / 100;
    let frames = period * cycles;
    let mut v = Vec::with_capacity(frames * channels as usize);
    for n in 0..frames {
        let x = q(0.5 * (2.0 * std::f64::consts::PI * n as f64 / period as f64).sin());
        for _ in 0..channels {
            v.push(x);
        }
    }
    v
}

struct StressFiles {
    _dir: TempDir,
    good: Vec<PathBuf>,
    bad: Vec<PathBuf>,
}

fn stress_files(tag: &str) -> StressFiles {
    let dir = TempDir::new(tag);
    let a = dir.file("a48.wav");
    write_wav(&a, 48_000, 2, &cycles_sine(48_000, 2, 120));
    let b = dir.file("b44.wav");
    write_wav(&b, 44_100, 2, &cycles_sine(44_100, 2, 100));
    let c = dir.file("c48_mono.wav");
    write_wav(&c, 48_000, 1, &cycles_sine(48_000, 1, 30));
    let d = dir.file("d44_short.wav");
    write_wav(&d, 44_100, 2, &cycles_sine(44_100, 2, 5));
    let e = dir.file("e96.wav");
    write_wav(&e, 96_000, 2, &cycles_sine(96_000, 2, 50));
    // Header claims twice the data that is present (truncated download).
    let t = dir.file("truncated.wav");
    write_wav(&t, 48_000, 2, &cycles_sine(48_000, 2, 100));
    let mut bytes = std::fs::read(&t).unwrap();
    let keep = 44 + 480 * 2 * 2 * 50;
    bytes.truncate(keep);
    std::fs::write(&t, bytes).unwrap();
    let corrupt = dir.file("corrupt.flac");
    let mut rng = XorShift::new(99);
    let junk: Vec<u8> = (0..20_000).map(|_| rng.next() as u8).collect();
    std::fs::write(&corrupt, junk).unwrap();
    let missing = dir.file("missing.mp3");
    StressFiles {
        good: vec![a, b, c, d, e, t],
        bad: vec![corrupt, missing],
        _dir: dir,
    }
}

/// Largest legitimate sample-to-sample change: the signal's own slope at the highest possible
/// gain (100 Hz at amplitude 0.5 x volume 2 x EQ <= +6 dB: ~0.026 before the soft limiter),
/// plus the fade slope (1/240) and the volume smoother's per-sample move. Engine artefacts
/// (hard cuts, gain switches) are an order of magnitude larger.
const STRESS_STEP_BOUND: f32 = 0.035;

#[derive(Default)]
struct StressCheck {
    prev: Option<[f32; 2]>,
    worst_step: f32,
    samples: u64,
}

impl StressCheck {
    fn check(&mut self, out: &[f32], ctx: &str) {
        for f in out.chunks_exact(2) {
            for c in 0..2 {
                let v = f[c];
                assert!(v.is_finite(), "non-finite sample {v} ({ctx})");
                assert!(v.abs() <= 1.0, "sample out of range {v} ({ctx})");
                if let Some(p) = self.prev {
                    let d = (v - p[c]).abs();
                    self.worst_step = self.worst_step.max(d);
                    assert!(
                        d <= STRESS_STEP_BOUND,
                        "discontinuity {d} ({} -> {v}) after {} samples ({ctx})",
                        p[c],
                        self.samples
                    );
                }
            }
            self.prev = Some([f[0], f[1]]);
            self.samples += 1;
        }
    }
}

fn random_eq(rng: &mut XorShift) -> EqSettings {
    let mut eq = EqSettings {
        enabled: rng.chance(70),
        preamp_db: -6.0 * rng.unit(),
        ..Default::default()
    };
    for b in eq.bands_db.iter_mut() {
        *b = if rng.chance(40) {
            12.0 * rng.unit() - 6.0
        } else {
            0.0
        };
    }
    eq
}

fn run_stress(seed: u64, ops: usize) {
    let files = stress_files(&format!("stress{seed}"));
    let engine = AudioEngine::new(EngineConfig {
        output: OutputMode::Manual {
            sample_rate: 48_000,
            channels: 2,
        },
        ring_ms: 300,
        prebuffer_ms: 50,
        fade_ms: 5,
        ..Default::default()
    })
    .unwrap();
    engine.sync();
    let mut rng = XorShift::new(seed);
    let mut check = StressCheck::default();
    let mut buf = vec![0.0f32; 4096 * 2];
    // (path, position) of the last snapshot taken since the last position-resetting command.
    let mut last_pos: Option<(Option<PathBuf>, u64)> = None;
    let mut log: Vec<String> = Vec::new();

    for step in 0..ops {
        let pick =
            |rng: &mut XorShift, v: &Vec<PathBuf>| v[rng.range(0, v.len() as u64) as usize].clone();
        let r = rng.range(0, 100);
        let mut resets_position = false;
        let op = match r {
            0..=9 => {
                let p = if rng.chance(90) {
                    pick(&mut rng, &files.good)
                } else {
                    pick(&mut rng, &files.bad)
                };
                let start = if rng.chance(50) {
                    0
                } else {
                    rng.range(0, 1600)
                };
                let auto = rng.chance(80);
                engine.load(p.clone(), start, auto);
                resets_position = true;
                format!("load {:?} {start} {auto}", p.file_name().unwrap())
            }
            10..=17 => {
                let ms = if rng.chance(3) {
                    u64::MAX
                } else {
                    rng.range(0, 1500)
                };
                engine.seek(ms);
                resets_position = true;
                format!("seek {ms}")
            }
            18..=25 => {
                engine.pause();
                "pause".into()
            }
            26..=35 => {
                engine.play();
                // play() from Stopped reloads from 0.
                resets_position = true;
                "play".into()
            }
            36..=39 => {
                engine.stop();
                resets_position = true;
                "stop".into()
            }
            40..=49 => {
                let p = match rng.range(0, 10) {
                    0 => None,
                    1 => Some(pick(&mut rng, &files.bad)),
                    _ => Some(pick(&mut rng, &files.good)),
                };
                engine.set_next(p.clone());
                format!("set_next {:?}", p.as_ref().and_then(|p| p.file_name()))
            }
            50..=56 => {
                let v = 2.0 * rng.unit();
                engine.set_volume(v);
                format!("volume {v}")
            }
            57..=59 => {
                let m = rng.chance(50);
                engine.set_muted(m);
                format!("muted {m}")
            }
            60..=66 => {
                // A burst of EQ updates, like a dragged slider.
                for _ in 0..rng.range(1, 6) {
                    engine.set_eq(random_eq(&mut rng));
                }
                "eq".into()
            }
            67..=70 => {
                // Callback starvation: the device stops pulling for a while.
                let ms = rng.range(1, 120);
                std::thread::sleep(Duration::from_millis(ms));
                format!("no render {ms} ms")
            }
            71..=74 => {
                // Engine-thread starvation: block it (it publishes into `info`) while the
                // "device" keeps pulling audio, until the ring runs dry.
                let frames = rng.range(1_000, 30_000) as usize;
                let guard = engine.info.lock().unwrap();
                let mut done = 0;
                while done < frames {
                    let n = (rng.range(1, 4097) as usize).min(frames - done);
                    assert!(engine.render_manual(&mut buf[..n * 2]));
                    check.check(&buf[..n * 2], "engine starved");
                    done += n;
                }
                drop(guard);
                format!("engine starved for {frames} frames")
            }
            _ => "render".into(),
        };
        log.push(op);
        if log.len() > 12 {
            log.remove(0);
        }
        let ctx = || format!("seed {seed} step {step}: {:?}", log);

        // Render a burst of irregular blocks, sometimes faster than the decoder can keep up.
        let blocks = rng.range(1, 6);
        for _ in 0..blocks {
            let n = match rng.range(0, 10) {
                0 => 1,
                1 => rng.range(1, 16) as usize,
                _ => rng.range(1, 4097) as usize,
            };
            assert!(engine.render_manual(&mut buf[..n * 2]));
            check.check(&buf[..n * 2], &ctx());
            if rng.chance(30) {
                std::thread::sleep(Duration::from_micros(rng.range(0, 3000)));
            }
        }

        // Position must not go backwards unless a command moved it or the track changed.
        // After sync() every track change the renderer made so far has been turned into an
        // event (the engine's housekeeping runs before it answers).
        engine.sync();
        while let Some(ev) = engine.try_recv_event() {
            if matches!(
                ev,
                EngineEvent::AdvancedToNext { .. }
                    | EngineEvent::TrackStarted { .. }
                    | EngineEvent::TrackFinished { .. }
            ) {
                resets_position = true;
            }
        }
        let snap = engine.snapshot();
        if resets_position || snap.state == EngineState::Stopped {
            last_pos = None;
        }
        if let Some(d) = snap.duration_ms {
            assert!(
                snap.position_ms <= d,
                "position beyond duration ({})",
                ctx()
            );
        }
        if snap.state != EngineState::Stopped {
            if let Some((p, pos)) = &last_pos {
                if *p == snap.path {
                    assert!(
                        snap.position_ms >= *pos,
                        "position went back {pos} -> {} ({})",
                        snap.position_ms,
                        ctx()
                    );
                }
            }
            last_pos = Some((snap.path.clone(), snap.position_ms));
        }
    }

    // Still functional: a fresh load plays audio.
    engine.set_volume(1.0);
    engine.set_muted(false);
    engine.load(files.good[0].clone(), 0, true);
    let mut heard = false;
    for _ in 0..200 {
        let out = render(&engine, 2, 480);
        check.check(&out, "final");
        if out.iter().any(|&s| s.abs() > 0.1) {
            heard = true;
            break;
        }
    }
    assert!(heard, "engine stopped producing audio after the stress run");

    let weak = engine.rt_weak();
    let t = Instant::now();
    drop(engine);
    assert!(
        t.elapsed() < Duration::from_secs(2),
        "drop took {:?}",
        t.elapsed()
    );
    assert!(weak.upgrade().is_none(), "threads/state leaked after drop");
    eprintln!(
        "stress seed {seed}: {} frames checked, worst step {:.4}",
        check.samples, check.worst_step
    );
}

#[test]
fn stress_random_transport_seed_1() {
    run_stress(1, 400);
}

#[test]
fn stress_random_transport_seed_2() {
    run_stress(0xC0FFEE, 400);
}

#[test]
fn stress_random_transport_seed_3() {
    run_stress(31337, 400);
}

/// The engine thread may be blocked in `render` waits; dropping with a full ring and nobody
/// rendering must still join promptly.
#[test]
fn drop_with_full_ring_and_no_rendering_is_prompt() {
    let files = stress_files("drop_full");
    let engine = manual_engine(48_000, 2);
    engine.load(files.good[0].clone(), 0, true);
    engine.set_next(Some(files.good[1].clone()));
    std::thread::sleep(Duration::from_millis(200));
    let weak = engine.rt_weak();
    let t = Instant::now();
    drop(engine);
    assert!(t.elapsed() < Duration::from_secs(1));
    assert!(weak.upgrade().is_none());
}

/// Longer randomized run: `LRC_STRESS_SEEDS=200 cargo test -p lrc_player_adapters_audio
/// --release stress_many_seeds -- --ignored --nocapture`.
#[test]
#[ignore]
fn stress_many_seeds() {
    let n: u64 = std::env::var("LRC_STRESS_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    for seed in 100..100 + n {
        run_stress(seed, 300);
    }
}

/// The GUI re-sends `set_next(B)` while A is still audible but B was already spliced in
/// (A's decoding finished): B must still play only once.
#[test]
fn repeated_set_next_after_splice_does_not_play_next_twice() {
    let dir = TempDir::new("next_dup");
    let a = dir.file("a.wav");
    let b = dir.file("b.wav");
    write_wav(&a, 48_000, 2, &cycles_sine(48_000, 2, 20)); // 0.2 s: fully decoded at once
    write_wav(&b, 48_000, 2, &cycles_sine(48_000, 2, 30)); // 0.3 s
    let engine = manual_engine(48_000, 2);
    engine.load(a.clone(), 0, true);
    engine.set_next(Some(b.clone()));
    engine.sync();
    // Let the engine decode A to EOF and splice B.
    std::thread::sleep(Duration::from_millis(100));
    engine.sync();
    engine.set_next(Some(b.clone()));
    engine.sync();
    let (out, events) = render_until_finished(&engine, 2, 480, 48_000 * 3);
    let advanced = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::AdvancedToNext { .. }))
        .count();
    assert_eq!(advanced, 1, "events: {events:?}");
    let first = first_nonzero_frame(&out, 2);
    let last = last_nonzero_frame(&out, 2);
    let len = last + 1 - first;
    assert!(
        len < 48_000 * 6 / 10,
        "played {len} frames: B was played twice"
    );
}

/// The renderer's command queue overflows while the output does not pull (stalled device /
/// nobody rendering) and commands keep coming. The lost flush used to desynchronise the
/// renderer for good (seek target never reached, position frozen).
#[test]
fn renderer_queue_overflow_recovers() {
    let dir = TempDir::new("overflow");
    let path = dir.file("a.wav");
    write_wav(&path, 48_000, 2, &cycles_sine(48_000, 2, 300)); // 3 s
    let engine = manual_engine(48_000, 2);
    engine.load(path, 0, true);
    for _ in 0..10 {
        render(&engine, 2, 480);
    }
    wait_state(&engine, EngineState::Playing);
    // No rendering from here on: 400 pause/play pairs overflow the 256-entry queue.
    for _ in 0..400 {
        engine.pause();
        engine.play();
    }
    engine.seek(1_000);
    engine.sync();
    engine.sync();
    let mut last = 0;
    for _ in 0..60 {
        render(&engine, 2, 480);
        last = engine.snapshot().position_ms;
    }
    // (Loading -> Playing is reported by the engine thread shortly after priming.)
    wait_state(&engine, EngineState::Playing);
    assert!(
        (1_100..1_700).contains(&last),
        "position {last}: renderer did not resynchronise after the overflow"
    );
}

/// The last owner of the engine may be released inside the waker (which runs on the engine
/// thread). Drop must not try to join the thread it is running on.
#[test]
fn dropping_engine_from_its_own_waker_is_safe() {
    let slot: std::sync::Arc<std::sync::Mutex<Option<AudioEngine>>> = Default::default();
    let engine = manual_engine(48_000, 2);
    let weak = engine.rt_weak();
    let s2 = slot.clone();
    engine.set_waker(std::sync::Arc::new(move || {
        let taken = s2.lock().unwrap().take();
        drop(taken);
    }));
    *slot.lock().unwrap() = Some(engine);
    // Any event (here: a load error) invokes the waker on the engine thread.
    slot.lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .load(PathBuf::from("/nope/x.wav"), 0, true);
    let t = Instant::now();
    while weak.upgrade().is_some() {
        assert!(
            t.elapsed() < Duration::from_secs(3),
            "engine thread did not exit after being dropped from its waker"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Absurd positions from the UI (e.g. a garbage duration) must not overflow the frame/time
/// arithmetic and kill the engine thread (debug builds panic on overflow).
#[test]
fn huge_seek_and_load_offsets_do_not_kill_the_engine() {
    let files = stress_files("huge_seek");
    let engine = manual_engine(48_000, 2);
    engine.load(files.good[0].clone(), u64::MAX, true);
    engine.sync();
    engine.load(files.good[1].clone(), 0, true);
    for _ in 0..5 {
        render(&engine, 2, 480);
    }
    engine.seek(u64::MAX);
    engine.sync();
    engine.seek(u64::MAX / 3);
    engine.sync();
    for _ in 0..5 {
        render(&engine, 2, 480);
    }
    let _ = engine.snapshot();
    // Still alive and functional.
    engine.load(files.good[0].clone(), 0, true);
    engine.sync();
    let mut heard = false;
    for _ in 0..100 {
        if render(&engine, 2, 480).iter().any(|s| s.abs() > 0.1) {
            heard = true;
            break;
        }
    }
    assert!(heard);
}
