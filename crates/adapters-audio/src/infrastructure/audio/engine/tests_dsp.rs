//! Engine-level tests of the DSP features: loudness normalization (gapless boundaries, late
//! hints), crossfeed, parametric EQ, auto headroom, the signal path and `processing_active`.
//! Like `tests.rs` they use `OutputMode::Manual` (no device).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::source::TrackSource;
use super::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "lrc_engine_dsp_test_{}_{}",
            std::process::id(),
            name
        ));
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

fn write_wav(path: &Path, rate: u32, channels: u16, samples: &[i16]) {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
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

/// Stereo, never zero: L = 0.25 + 0.2 sin, R = -0.25 + 0.2 cos.
fn offset_sine(rate: u32, freq: f64, start: usize, frames: usize) -> Vec<i16> {
    (start..start + frames)
        .flat_map(|n| {
            let ph = 2.0 * std::f64::consts::PI * freq * n as f64 / rate as f64;
            [q(0.25 + 0.2 * ph.sin()), q(-0.25 + 0.2 * ph.cos())]
        })
        .collect()
}

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
    let rate = engine
        .snapshot()
        .device
        .map(|d| d.sample_rate as usize)
        .unwrap_or(48_000);
    wait_available(engine, buf.len() + rate / 10 * channels);
    assert!(engine.render_manual(&mut buf));
    buf
}

/// Renders until the end marker was played (plus a little), in `chunk`-frame blocks.
fn render_to_end(
    engine: &AudioEngine,
    channels: usize,
    chunk: usize,
    max_frames: usize,
) -> Vec<f32> {
    let mut out = Vec::new();
    let mut extra = 0;
    while out.len() < max_frames * channels {
        out.extend(render(engine, channels, chunk));
        if engine.rt.ended_at.load(Ordering::Acquire) != 0 {
            extra += 1;
            if extra > 4 {
                break;
            }
        }
    }
    out
}

fn first_nonzero_frame(out: &[f32], ch: usize) -> usize {
    out.chunks(ch)
        .position(|f| f.iter().any(|&s| s != 0.0))
        .expect("no audio rendered")
}

fn hint(db: f32) -> LoudnessHint {
    LoudnessHint {
        track_gain_db: db,
        track_peak: 0.5,
        album_gain_db: None,
        album_peak: None,
    }
}

fn track_mode() -> Normalization {
    Normalization {
        mode: NormalizationMode::Track,
        ..Default::default()
    }
}

/// Two gapless tracks with different normalization gains: each sample is scaled by its own
/// track's gain, switching exactly at the boundary sample.
#[test]
fn gapless_boundary_switches_normalization_gain_exactly() {
    let dir = TempDir::new("norm_gapless");
    let (a, b) = (dir.file("a.wav"), dir.file("b.wav"));
    let rate = 48_000;
    let (na, nb) = (rate as usize / 2 + 321, rate as usize / 2);
    write_wav(&a, rate, 2, &offset_sine(rate, 440.0, 0, na));
    write_wav(&b, rate, 2, &offset_sine(rate, 440.0, na, nb));
    let (ra, rb) = (decode_all(&a), decode_all(&b));

    let engine = manual_engine(rate, 2);
    engine.set_normalization(track_mode());
    engine.set_loudness_hint(a.clone(), Some(hint(-6.0)));
    engine.set_loudness_hint(b.clone(), Some(hint(-12.0)));
    engine.load(a.clone(), 0, true);
    engine.set_next(Some(b.clone()));
    let out = render_to_end(&engine, 2, 333, (na + nb) * 3);
    let (ga, gb) = (10f32.powf(-6.0 / 20.0), 10f32.powf(-12.0 / 20.0));
    let first = first_nonzero_frame(&out, 2);
    let fade = rate as usize * 5 / 1000;
    for n in fade..na {
        for c in 0..2 {
            let got = out[(first + n) * 2 + c];
            assert_eq!(
                got.to_bits(),
                (ra[n * 2 + c] * ga).to_bits(),
                "track a frame {n}"
            );
        }
    }
    for n in 0..nb {
        for c in 0..2 {
            let got = out[(first + na + n) * 2 + c];
            assert_eq!(
                got.to_bits(),
                (rb[n * 2 + c] * gb).to_bits(),
                "track b frame {n}"
            );
        }
    }
    assert_eq!(engine.snapshot().underruns, 0);
}

/// A hint that arrives while its track is already playing does not change that track's gain
/// (no volume jump shortly after the start); a settings change applies it, ramped over ~300 ms.
#[test]
fn late_hint_keeps_the_playing_track_gain() {
    let dir = TempDir::new("norm_late");
    let path = dir.file("dc.wav");
    let rate = 48_000u32;
    let frames = rate as usize * 4;
    write_wav(&path, rate, 2, &vec![q(0.5); frames * 2]);
    let level = decode_all(&path)[0];
    let engine = manual_engine(rate, 2);
    engine.set_normalization(track_mode());
    engine.load(path.clone(), 0, true);
    let mut out = Vec::new();
    for _ in 0..20 {
        out.extend(render(&engine, 2, 480));
    }
    // No data yet: unity gain, untouched.
    let s = engine.snapshot().signal.unwrap();
    assert_eq!(s.normalization_source, GainSource::None);
    assert_eq!(*out.last().unwrap(), level);

    engine.set_loudness_hint(path.clone(), Some(hint(-12.0)));
    engine.sync();
    let start = out.len() / 2;
    for _ in 0..50 {
        out.extend(render(&engine, 2, 480));
    }
    assert!(
        out.chunks(2).skip(start).all(|f| f[0] == level),
        "a late hint changed the playing track's gain"
    );
    let s = engine.snapshot().signal.unwrap();
    assert_eq!(s.normalization_source, GainSource::None);

    // A settings change recomputes the playing track's gain (now with the hint), smoothly.
    engine.set_normalization(track_mode());
    engine.sync();
    let start = out.len() / 2;
    for _ in 0..50 {
        out.extend(render(&engine, 2, 480));
    }
    let gains: Vec<f32> = out.chunks(2).skip(start).map(|f| f[0] / level).collect();
    let target = 10f32.powf(-12.0 / 20.0);
    let ramp = rate as f32 * 0.3;
    let max_step = gains
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(
        max_step <= (1.0 - target) / ramp * 1.05,
        "gain step {max_step} (ramp must take ~300 ms)"
    );
    assert!(
        gains.windows(2).all(|w| w[1] <= w[0] + 1e-6),
        "not monotonic"
    );
    let reached = gains
        .iter()
        .position(|g| (g - target).abs() < 1e-5)
        .expect("never reached the target");
    assert!(
        (ramp as usize * 9 / 10..ramp as usize * 11 / 10).contains(&reached),
        "target reached after {reached} frames"
    );
    let s = engine.snapshot().signal.unwrap();
    assert_eq!(s.normalization_source, GainSource::Hint);
    assert!((s.normalization_db + 12.0).abs() < 1e-4);
    assert!(!s.bit_perfect);
    // Turning normalization off ramps back up, also smoothly.
    engine.set_normalization(Normalization::default());
    engine.sync();
    let start = out.len() / 2;
    for _ in 0..40 {
        out.extend(render(&engine, 2, 480));
    }
    let back: Vec<f32> = out.chunks(2).skip(start).map(|f| f[0] / level).collect();
    let max_step = back
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(max_step <= (1.0 - target) / ramp * 1.05, "{max_step}");
    assert_eq!(*back.last().unwrap(), 1.0);
    assert_eq!(engine.snapshot().underruns, 0);
}

/// With crossfeed on, a centre (L == R) signal passes bit-exactly at unity.
#[test]
fn crossfeed_keeps_centre_signal_at_unity() {
    let dir = TempDir::new("xfeed");
    let path = dir.file("mono_in_stereo.wav");
    let rate = 48_000;
    let frames = rate as usize;
    let s: Vec<i16> = (0..frames)
        .flat_map(|n| {
            let v = q(0.3 + 0.4 * (2.0 * std::f64::consts::PI * 300.0 * n as f64 / 48_000.0).sin());
            [v, v]
        })
        .collect();
    write_wav(&path, rate, 2, &s);
    let reference = decode_all(&path);
    let engine = manual_engine(rate, 2);
    engine.set_crossfeed(Crossfeed {
        enabled: true,
        level: CrossfeedLevel::Weak,
    });
    engine.load(path, 0, true);
    let out = render_to_end(&engine, 2, 512, frames * 3);
    let first = first_nonzero_frame(&out, 2);
    let fade = rate as usize * 5 / 1000;
    for n in fade..frames {
        assert_eq!(out[(first + n) * 2], reference[n * 2], "frame {n}");
        assert_eq!(out[(first + n) * 2 + 1], reference[n * 2 + 1], "frame {n}");
    }
}

/// Measures the steady-state peak of channel 0 over the last half of `out`.
fn tail_peak(out: &[f32]) -> f32 {
    out[out.len() / 2..]
        .iter()
        .step_by(2)
        .fold(0.0f32, |m, v| m.max(v.abs()))
}

/// EQ boosts are compensated by the auto headroom (no limiter), crossfaded smoothly, and
/// both EQs' boosts add up.
#[test]
fn auto_headroom_compensates_eq_boosts() {
    let dir = TempDir::new("headroom");
    let path = dir.file("sine1k.wav");
    let rate = 48_000u32;
    let frames = rate as usize * 6;
    let s: Vec<i16> = (0..frames)
        .flat_map(|n| {
            let v = q(0.8 * (2.0 * std::f64::consts::PI * 1000.0 * n as f64 / 48_000.0).sin());
            [v, v]
        })
        .collect();
    write_wav(&path, rate, 2, &s);
    let engine = manual_engine(rate, 2);
    engine.load(path, 0, true);
    let mut out = Vec::new();
    for _ in 0..20 {
        out.extend(render(&engine, 2, 480));
    }
    let base = tail_peak(&out);
    assert!((base - 0.8).abs() < 1e-3);

    // +6 dB at 1 kHz (graphic EQ) and +4 dB at 1 kHz (parametric): +10 dB total.
    let mut eq = EqSettings {
        enabled: true,
        ..Default::default()
    };
    eq.bands_db[5] = 6.0;
    engine.set_eq(eq);
    engine.set_parametric_eq(ParametricEq {
        enabled: true,
        preamp_db: 0.0,
        filters: vec![PeqFilter {
            kind: FilterKind::Peak,
            freq_hz: 1000.0,
            gain_db: 4.0,
            q: 1.0,
        }],
        ..Default::default()
    });
    engine.sync();
    let mut seg = Vec::new();
    for _ in 0..40 {
        seg.extend(render(&engine, 2, 480));
    }
    let s = engine.snapshot().signal.unwrap();
    assert!(s.eq && s.parametric_eq && !s.bit_perfect);
    assert!(
        (s.headroom_db + 10.0).abs() < 0.1,
        "headroom {}",
        s.headroom_db
    );
    // Boost at 1 kHz fully compensated: same level as before, limiter never engaged.
    let peak = tail_peak(&seg);
    assert!(
        (peak - base).abs() < 0.02,
        "compensated peak {peak} vs {base}"
    );
    assert!(!engine.snapshot().signal.unwrap().limiter_engaged);
    // The switch was crossfaded: no sample step beyond the sine's own slope (+ margin).
    let mut all = out.clone();
    all.extend_from_slice(&seg);
    let slope = 0.8 * 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0;
    let m = all
        .chunks(2)
        .collect::<Vec<_>>()
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).abs())
        .fold(0.0f32, f32::max);
    assert!(m <= slope * 3.2 + 1e-3, "step {m}");

    // A negative preamp that already leaves room reduces the automatic headroom.
    eq.preamp_db = -4.0;
    engine.set_eq(eq);
    engine.sync();
    let s = engine.snapshot().signal.unwrap();
    assert!(
        (s.headroom_db + 6.0).abs() < 0.1,
        "headroom {}",
        s.headroom_db
    );

    // Auto headroom off: the boost drives the limiter.
    engine.set_auto_headroom(false);
    engine.sync();
    let mut seg = Vec::new();
    for _ in 0..40 {
        seg.extend(render(&engine, 2, 480));
    }
    let s = engine.snapshot().signal.unwrap();
    assert_eq!(s.headroom_db, 0.0);
    assert!(
        s.limiter_engaged,
        "0.8 boosted by 6 dB must hit the limiter"
    );
    assert!(tail_peak(&seg) <= 1.0);
}

/// Separate PEQ channels: the left output gets the main set, the right output `right`; the
/// auto headroom covers the worse channel; switching back to linked applies the main set to
/// both.
#[test]
fn separate_peq_channels_reach_their_outputs() {
    let dir = TempDir::new("peq_lr");
    let path = dir.file("sine1k.wav");
    let rate = 48_000u32;
    let frames = rate as usize * 8;
    let s: Vec<i16> = (0..frames)
        .flat_map(|n| {
            let v = q(0.25 * (2.0 * std::f64::consts::PI * 1000.0 * n as f64 / 48_000.0).sin());
            [v, v]
        })
        .collect();
    write_wav(&path, rate, 2, &s);
    let engine = manual_engine(rate, 2);
    engine.set_auto_headroom(false);
    engine.load(path, 0, true);
    for _ in 0..20 {
        render(&engine, 2, 480);
    }
    let peaks = |engine: &AudioEngine| {
        let mut seg = Vec::new();
        for _ in 0..40 {
            seg.extend(render(engine, 2, 480));
        }
        let tail = &seg[seg.len() / 2..];
        let l = tail.iter().step_by(2).fold(0.0f32, |m, v| m.max(v.abs()));
        let r = tail.iter().skip(1).step_by(2).fold(0.0f32, |m, v| m.max(v.abs()));
        (20.0 * (l / 0.25).log10(), 20.0 * (r / 0.25).log10())
    };
    let pk = |gain_db| PeqFilter {
        kind: FilterKind::Peak,
        freq_hz: 1000.0,
        gain_db,
        q: 1.0,
    };
    let mut peq = ParametricEq {
        enabled: true,
        filters: vec![pk(3.0)],
        channel_mode: PeqChannelMode::Separate,
        right: PeqChannel {
            preamp_db: -2.0,
            filters: vec![pk(-6.0)],
        },
        ..Default::default()
    };
    engine.set_parametric_eq(peq.clone());
    engine.sync();
    let (l, r) = peaks(&engine);
    assert!((l - 3.0).abs() < 0.1, "left {l}");
    assert!((r + 8.0).abs() < 0.1, "right {r}");
    assert!(engine.snapshot().signal.unwrap().parametric_eq);
    // Auto headroom: the worse channel (L, +3 dB) decides.
    engine.set_auto_headroom(true);
    engine.sync();
    let hr = engine.snapshot().signal.unwrap().headroom_db;
    assert!((hr + 3.0).abs() < 0.1, "headroom {hr}");
    engine.set_auto_headroom(false);
    // Linked: the main set on both channels.
    peq.channel_mode = PeqChannelMode::Linked;
    engine.set_parametric_eq(peq);
    engine.sync();
    let (l, r) = peaks(&engine);
    assert!((l - 3.0).abs() < 0.1 && (r - 3.0).abs() < 0.1, "linked {l} {r}");
}

/// `bit_perfect` (and `processing_active`) only when every stage is off and the volume is 1.
#[test]
fn bit_perfect_only_when_everything_is_off() {
    let dir = TempDir::new("bitperfect");
    let p48 = dir.file("a48.wav");
    let p44 = dir.file("a44.wav");
    write_wav(&p48, 48_000, 2, &offset_sine(48_000, 440.0, 0, 48_000 * 8));
    write_wav(&p44, 44_100, 2, &offset_sine(44_100, 440.0, 0, 44_100 * 3));
    let engine = manual_engine(48_000, 2);
    engine.load(p48.clone(), 0, true);
    for _ in 0..20 {
        render(&engine, 2, 480);
    }
    let check = |expect: bool, what: &str| {
        engine.sync();
        for _ in 0..12 {
            render(&engine, 2, 480);
        }
        let s = engine.snapshot().signal.expect("signal path");
        assert_eq!(s.bit_perfect, expect, "{what}: {s:?}");
        assert_eq!(
            engine.rt.processing_active.load(Ordering::Relaxed),
            !expect,
            "processing_active ({what})"
        );
        s
    };
    let s = check(true, "everything off");
    assert_eq!(s.codec, "pcm_s16le");
    assert_eq!(
        (s.source_rate, s.source_bits, s.source_channels),
        (48_000, Some(16), 2)
    );
    assert!(!s.resampled && s.resampler.is_none() && !s.volume_active);
    assert_eq!(s.output.sample_rate, 48_000);

    engine.set_volume(0.5);
    assert!(check(false, "volume").volume_active);
    engine.set_volume(1.0);
    check(true, "volume back to 1");

    // Enabled but flat EQs are bypassed exactly.
    engine.set_eq(EqSettings {
        enabled: true,
        ..Default::default()
    });
    engine.set_parametric_eq(ParametricEq {
        enabled: true,
        ..Default::default()
    });
    check(true, "flat EQs");
    let mut eq = EqSettings {
        enabled: true,
        ..Default::default()
    };
    eq.bands_db[3] = -3.0;
    engine.set_eq(eq);
    assert!(check(false, "EQ").eq);
    engine.set_eq(EqSettings::default());
    check(true, "EQ off");

    engine.set_parametric_eq(ParametricEq {
        enabled: true,
        preamp_db: -1.0,
        filters: vec![],
        ..Default::default()
    });
    assert!(check(false, "PEQ").parametric_eq);
    // Separate channels: only the right channel's preamp changes the signal.
    engine.set_parametric_eq(ParametricEq {
        enabled: true,
        channel_mode: PeqChannelMode::Separate,
        right: PeqChannel {
            preamp_db: -1.0,
            filters: vec![],
        },
        ..Default::default()
    });
    assert!(check(false, "PEQ right only").parametric_eq);
    engine.set_parametric_eq(ParametricEq::default());
    check(true, "PEQ off");

    engine.set_crossfeed(Crossfeed {
        enabled: true,
        level: CrossfeedLevel::Strong,
    });
    assert_eq!(
        check(false, "crossfeed").crossfeed,
        Some(CrossfeedLevel::Strong)
    );
    engine.set_crossfeed(Crossfeed::default());
    check(true, "crossfeed off");

    engine.set_normalization(track_mode());
    check(true, "normalization without data");
    engine.set_loudness_hint(p48.clone(), Some(hint(-3.0)));
    // (a hint for the playing track applies on the next settings change)
    engine.set_normalization(track_mode());
    // (wait for the 300 ms ramp to settle into a steady gain)
    for _ in 0..40 {
        render(&engine, 2, 480);
    }
    let s = check(false, "normalization");
    assert_eq!(s.normalization_source, GainSource::Hint);
    engine.set_normalization(Normalization::default());
    for _ in 0..40 {
        render(&engine, 2, 480);
    }
    check(true, "normalization off");

    engine.load(p44, 0, true);
    for _ in 0..20 {
        render(&engine, 2, 480);
    }
    let s = check(false, "resampled");
    assert!(s.resampled);
    assert_eq!(s.resampler, Some(ResamplerKind::Fft));
}

/// Tiny deterministic PRNG (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo).max(1)
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// 100 Hz sine, amplitude 0.5, whole cycles (every join is continuous in the signal itself).
fn cycles_sine(rate: u32, cycles: usize) -> Vec<i16> {
    let period = rate as usize / 100;
    (0..period * cycles)
        .flat_map(|n| {
            let v = q(0.5 * (2.0 * std::f64::consts::PI * n as f64 / period as f64).sin());
            [v, v]
        })
        .collect()
}

/// Random transport mixed with random DSP changes (normalization + late hints, parametric
/// EQ, crossfeed, auto headroom): output stays finite, in range and free of steps beyond the
/// signal's own slope plus fade/ramp slopes.
#[test]
fn stress_transport_with_dsp_changes() {
    let dir = TempDir::new("dsp_stress");
    let files: Vec<PathBuf> = [(48_000u32, 150usize), (44_100, 100), (48_000, 20)]
        .iter()
        .enumerate()
        .map(|(i, &(rate, cycles))| {
            let p = dir.file(&format!("{i}.wav"));
            write_wav(&p, rate, 2, &cycles_sine(rate, cycles));
            p
        })
        .collect();
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
    let mut rng = Rng(0x5EED_1234_ABCD_0001);
    let mut buf = vec![0.0f32; 4096 * 2];
    let mut prev: Option<f32> = None;
    // 100 Hz at 0.5 with up to +6 dB normalization (clip-limited by the 0.5 peak): 0.013;
    // fade slope 1/240; volume <= 1. EQ boosts are compensated by the auto headroom.
    let bound = 0.03;
    let mut audible = 0usize;
    let mut log: Vec<String> = Vec::new();
    for step in 0..400 {
        let pick = |rng: &mut Rng| files[rng.range(0, files.len() as u64) as usize].clone();
        let op = match rng.range(0, 100) {
            0..=9 => {
                let p = pick(&mut rng);
                engine.load(p, rng.range(0, 1000), rng.range(0, 10) < 8);
                "load".to_string()
            }
            10..=17 => {
                engine.seek(rng.range(0, 1400));
                "seek".into()
            }
            18..=23 => {
                engine.pause();
                "pause".into()
            }
            24..=33 => {
                engine.play();
                "play".into()
            }
            34..=41 => {
                engine.set_next(Some(pick(&mut rng)));
                "set_next".into()
            }
            42..=51 => {
                let p = pick(&mut rng);
                let h = (rng.range(0, 4) > 0).then(|| LoudnessHint {
                    track_gain_db: 18.0 * rng.unit() - 12.0,
                    track_peak: 0.5,
                    album_gain_db: Some(-3.0),
                    album_peak: Some(0.5),
                });
                engine.set_loudness_hint(p, h);
                format!("hint {h:?}")
            }
            52..=57 => {
                let mode = match rng.range(0, 3) {
                    0 => NormalizationMode::Off,
                    1 => NormalizationMode::Track,
                    _ => NormalizationMode::Album,
                };
                engine.set_normalization(Normalization {
                    mode,
                    preamp_db: 3.0 * rng.unit() - 1.5,
                    prevent_clipping: true,
                });
                format!("normalization {mode:?}")
            }
            58..=67 => {
                let n = rng.range(0, 6) as usize;
                let filters = (0..n)
                    .map(|_| PeqFilter {
                        kind: match rng.range(0, 5) {
                            0 => FilterKind::Peak,
                            1 => FilterKind::LowShelf,
                            2 => FilterKind::HighShelf,
                            3 => FilterKind::LowPass,
                            _ => FilterKind::HighPass,
                        },
                        freq_hz: 20.0 * 1000f32.powf(rng.unit()),
                        gain_db: 24.0 * rng.unit() - 12.0,
                        q: 0.3 + 3.0 * rng.unit(),
                    })
                    .collect();
                engine.set_parametric_eq(ParametricEq {
                    enabled: rng.range(0, 10) < 8,
                    preamp_db: -3.0 * rng.unit(),
                    filters,
                    ..Default::default()
                });
                "peq".into()
            }
            68..=72 => {
                engine.set_crossfeed(Crossfeed {
                    enabled: rng.range(0, 2) == 0,
                    level: CrossfeedLevel::Medium,
                });
                "crossfeed".into()
            }
            73..=77 => {
                let mut eq = EqSettings {
                    enabled: true,
                    ..Default::default()
                };
                eq.bands_db[rng.range(0, EQ_BANDS as u64) as usize] = 12.0 * rng.unit() - 6.0;
                engine.set_eq(eq);
                "eq".into()
            }
            78..=80 => {
                engine.set_volume(rng.unit());
                "volume".into()
            }
            _ => "render".into(),
        };
        log.push(op);
        if log.len() > 10 {
            log.remove(0);
        }
        for _ in 0..rng.range(1, 5) {
            let n = rng.range(1, 4097) as usize;
            // Mostly keep pace with the decoder (some underruns are part of the test).
            let t = Instant::now();
            while engine.manual_available().unwrap_or(0) < n * 2 + 4800
                && t.elapsed() < Duration::from_millis(15)
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(engine.render_manual(&mut buf[..n * 2]));
            for f in buf[..n * 2].chunks_exact(2) {
                let v = f[0];
                assert!(
                    v.is_finite() && v.abs() <= 1.0,
                    "sample {v} step {step} {log:?}"
                );
                if let Some(p) = prev {
                    // A low-pass/high-pass changes the signal's own slope only marginally at
                    // 100 Hz; resonant boosts are compensated by the headroom.
                    assert!(
                        (v - p).abs() <= bound,
                        "step {} at op {step}: {log:?}",
                        (v - p).abs()
                    );
                }
                prev = Some(v);
                audible += (v != 0.0) as usize;
            }
        }
        engine.sync();
    }
    assert!(
        audible > 200_000,
        "only {audible} audible frames: test did not play much"
    );
}
