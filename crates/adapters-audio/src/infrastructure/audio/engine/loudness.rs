//! EBU R128 / ITU-R BS.1770 loudness measurement for tracks without ReplayGain tags.
//!
//! [`measure_loudness`] decodes a file with the playback decoder ([`TrackSource`]) at its
//! native rate and channel count (no resampling, no channel mapping) and feeds the samples to
//! the `ebur128` crate in mode `I | TRUE_PEAK`:
//!
//! - **Integrated loudness** (gated, LUFS) per BS.1770-4.
//! - **True peak** (linear, 4x oversampled below 96 kHz, 2x below 192 kHz). It is what keeps
//!   positive gains from clipping inter-sample peaks, and it is cheap enough, so it is used
//!   instead of the sample peak ([`measure_loudness_with`] can measure that instead).
//!
//! Measured cost (single thread, `measure_speed` test, optimized build), CPU seconds per
//! minute of audio: 48 kHz/24-bit FLAC 0.12 s (sample peak 0.09 s), 320 kbps MP3 0.12 s
//! (0.08 s), 44.1 kHz WAV 0.07 s (0.05 s). A 4-minute track takes about half a second.
//!
//! # Gain convention (ReplayGain 2.0)
//!
//! The reference level is [`REFERENCE_LUFS`] = −18 LUFS, so
//! `gain_db = -18 - integrated_lufs` ([`TrackLoudness::gain_db`]). This is what ReplayGain
//! 2.0 scanners (foobar2000, loudgain, rsgain) write into `REPLAYGAIN_TRACK_GAIN`, so
//! measured and tagged files end up at the same level.
//!
//! Mono files are measured as one channel with weight 1.0, like those scanners do (no
//! "dual mono" +3 dB), so a measured mono file gets the same gain a scanner would tag.
//!
//! # Album gain
//!
//! [`album_gain`] combines per-track results by mean energy weighted by duration. True
//! album gating would need every track's gating-block histogram; the weighted mean matches
//! it closely when the tracks have similar loudness ranges (see the function docs).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Result};
use ebur128::{Channel, EbuR128, Mode};
use symphonia::core::audio::Channels;

use super::normalization::ReplayGainTags;
use super::source::{apply_replay_gain, TrackSource};

/// ReplayGain 2.0 reference loudness (LUFS).
pub const REFERENCE_LUFS: f64 = -18.0;

/// Loudness reported for digital silence (BS.1770 absolute gate). Keeps results finite (the
/// cache is JSON) and gives silent tracks no weight in [`album_gain`].
pub const SILENCE_LUFS: f64 = -70.0;

/// Gains are clamped to ±this many dB (a near-silent track must not get +50 dB).
pub const MAX_GAIN_DB: f32 = 24.0;

/// Result of [`measure_loudness`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackLoudness {
    /// Gated integrated loudness (LUFS); [`SILENCE_LUFS`] for silence.
    pub integrated_lufs: f64,
    /// Highest true (or sample, see [`PeakMode`]) peak over all channels, linear (1.0 = 0 dBFS).
    pub true_peak: f32,
    /// Decoded audio length.
    pub duration_ms: u64,
}

impl TrackLoudness {
    /// ReplayGain 2.0 track gain: `-18 - integrated_lufs`, clamped to ±[`MAX_GAIN_DB`].
    pub fn gain_db(&self) -> f32 {
        gain_for_lufs(self.integrated_lufs)
    }
}

/// `-18 - lufs`, clamped to ±[`MAX_GAIN_DB`].
pub fn gain_for_lufs(lufs: f64) -> f32 {
    ((REFERENCE_LUFS - lufs) as f32).clamp(-MAX_GAIN_DB, MAX_GAIN_DB)
}

/// Which peak [`measure_loudness_with`] measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeakMode {
    /// Inter-sample (oversampled) peak, BS.1770-4 Annex 2. The default.
    True,
    /// Largest absolute sample value (cheaper, under-reads inter-sample overs).
    Sample,
}

/// [`measure_loudness`] stopped because `cancel` was set. Check with
/// `err.is::<MeasureCancelled>()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasureCancelled;

impl std::fmt::Display for MeasureCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("loudness measurement cancelled")
    }
}

impl std::error::Error for MeasureCancelled {}

/// Measures integrated loudness and true peak of `path` (decode only, native rate).
///
/// Checks `cancel` after every decoded packet and then returns [`MeasureCancelled`].
/// Runs on the calling thread; call it from a low-priority worker.
pub fn measure_loudness(path: &Path, cancel: &AtomicBool) -> Result<TrackLoudness> {
    measure_loudness_with(path, cancel, PeakMode::True)
}

/// [`measure_loudness`] with an explicit peak mode.
pub fn measure_loudness_with(path: &Path, cancel: &AtomicBool, peak: PeakMode) -> Result<TrackLoudness> {
    let mode = Mode::I
        | match peak {
            PeakMode::True => Mode::TRUE_PEAK,
            PeakMode::Sample => Mode::SAMPLE_PEAK,
        };
    let mut source = TrackSource::open(path)?;
    // One meter per (rate, channels) run: a chained stream that changes format mid-file starts
    // a new meter, and the integrated loudness is gated over all of them together.
    let mut meters: Vec<EbuR128> = Vec::new();
    let mut current: Option<(u32, usize, Option<u32>)> = None;
    let mut frames_total: u128 = 0;
    let mut ms_total: u128 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(MeasureCancelled.into());
        }
        let Some(fmt) = source.decode_next()? else { break };
        let key = (fmt.rate, fmt.channels, fmt.layout);
        if current != Some(key) {
            if let Some((rate, _, _)) = current {
                ms_total += frames_total * 1000 / rate.max(1) as u128;
                frames_total = 0;
            }
            let mut meter = EbuR128::new(fmt.channels as u32, fmt.rate, mode)
                .map_err(|e| anyhow!("loudness meter: {e:?} ({} Hz, {} ch)", fmt.rate, fmt.channels))?;
            if let Some(map) = fmt.layout.and_then(|bits| channel_map(bits, fmt.channels)) {
                meter
                    .set_channel_map(&map)
                    .map_err(|e| anyhow!("loudness channel map: {e:?}"))?;
            }
            meters.push(meter);
            current = Some(key);
        }
        let block = source.block();
        let meter = meters.last_mut().expect("meter for current format");
        meter
            .add_frames_f32(block)
            .map_err(|e| anyhow!("loudness meter: {e:?}"))?;
        frames_total += (block.len() / fmt.channels.max(1)) as u128;
    }
    if let Some((rate, _, _)) = current {
        ms_total += frames_total * 1000 / rate.max(1) as u128;
    }
    if meters.is_empty() {
        return Err(anyhow!("no audio in {}", path.display()));
    }
    let integrated = EbuR128::loudness_global_multiple(meters.iter())
        .map_err(|e| anyhow!("loudness: {e:?}"))?;
    let integrated = if integrated.is_finite() { integrated.max(SILENCE_LUFS) } else { SILENCE_LUFS };
    let mut max_peak = 0.0f64;
    for meter in &meters {
        for ch in 0..meter.channels() {
            let p = match peak {
                PeakMode::True => meter.true_peak(ch),
                PeakMode::Sample => meter.sample_peak(ch),
            }
            .unwrap_or(0.0);
            if p.is_finite() {
                max_peak = max_peak.max(p);
            }
        }
    }
    Ok(TrackLoudness {
        integrated_lufs: integrated,
        true_peak: max_peak as f32,
        duration_ms: ms_total.min(u64::MAX as u128) as u64,
    })
}

/// BS.1770 channel weights from symphonia's layout bits (interleaved in bit order). `None`
/// when the bits do not describe `channels` channels (the meter's default map is used then).
fn channel_map(bits: u32, channels: usize) -> Option<Vec<Channel>> {
    let layout = Channels::from_bits_truncate(bits);
    if layout.count() != channels || channels <= 2 {
        // Mono/stereo: the default map (L, R) already has the right weights (1.0).
        return None;
    }
    let left = Channels::REAR_LEFT | Channels::SIDE_LEFT | Channels::REAR_LEFT_CENTRE;
    let right = Channels::REAR_RIGHT | Channels::SIDE_RIGHT | Channels::REAR_RIGHT_CENTRE;
    Some(
        layout
            .iter()
            .map(|c| {
                if c == Channels::FRONT_LEFT {
                    Channel::Left
                } else if c == Channels::FRONT_RIGHT {
                    Channel::Right
                } else if c == Channels::LFE1 || c == Channels::LFE2 {
                    Channel::Unused
                } else if left.contains(c) {
                    Channel::LeftSurround
                } else if right.contains(c) {
                    Channel::RightSurround
                } else {
                    // Centre and every other position: weight 1.0.
                    Channel::Center
                }
            })
            .collect(),
    )
}

/// Album gain (dB, ReplayGain 2.0 convention) and album peak (linear, the highest track
/// peak) from per-track measurements.
///
/// Approximation of album gating: the album loudness is the duration-weighted mean *energy*
/// of the tracks, `L = -0.691 + 10·log10(Σ dᵢ·Eᵢ / Σ dᵢ)` with `Eᵢ = 10^((Lᵢ + 0.691)/10)`.
/// This equals BS.1770 gating over the concatenated album when every track's gated blocks
/// cover its whole duration, and differs from it (typically by well under 1 LU) when tracks
/// have long quiet passages, which real gating would exclude relative to the *album*
/// threshold rather than each track's. Silent tracks ([`SILENCE_LUFS`] or below) are left
/// out, as the absolute gate would. Returns `(0.0, 0.0)` for an empty or silent album
/// (peak is still the maximum when tracks are silent).
pub fn album_gain(tracks: &[TrackLoudness]) -> (f32, f32) {
    let peak = tracks.iter().map(|t| t.true_peak).fold(0.0f32, f32::max);
    let mut weighted = 0.0f64;
    let mut total = 0.0f64;
    for t in tracks {
        // NaN-safe: anything not above the silence floor is skipped.
        if t.integrated_lufs.partial_cmp(&SILENCE_LUFS) != Some(std::cmp::Ordering::Greater) || t.duration_ms == 0 {
            continue;
        }
        let d = t.duration_ms as f64;
        weighted += d * 10f64.powf((t.integrated_lufs + 0.691) / 10.0);
        total += d;
    }
    if total <= 0.0 || weighted <= 0.0 {
        return (0.0, peak);
    }
    let lufs = -0.691 + 10.0 * (weighted / total).log10();
    (gain_for_lufs(lufs), peak)
}

/// Lowers the calling thread's priority for background analysis (Windows background mode,
/// nice +10 on Linux; best effort). Call once at the start of an analysis worker thread.
pub fn lower_current_thread_priority() {
    super::priority::demote_background_thread();
}

/// True when `path` carries a usable ReplayGain track or album gain (ID3v2 `TXXX`, Vorbis
/// comment, APE or MP4 freeform), parsed exactly as the engine parses it, so "tagged" here
/// means "the engine normalizes it from its tags". Reads headers/metadata only.
pub fn has_replaygain_tags(path: &Path) -> bool {
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let Ok(file) = std::fs::File::open(path) else { return false };
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let Ok(mut probed) = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) else {
        return false;
    };
    let mut rg = ReplayGainTags::default();
    if let Some(rev) = probed.metadata.get().as_ref().and_then(|m| m.current().cloned()) {
        apply_replay_gain(&mut rg, rev.tags());
    }
    if let Some(rev) = probed.format.metadata().skip_to_latest() {
        apply_replay_gain(&mut rg, rev.tags());
    }
    rg.has_gain()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Instant;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "lrc_loudness_{name}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
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

    /// 32-bit float WAV (exact levels, no dither questions).
    fn write_wav_f32(path: &Path, rate: u32, channels: u16, samples: &[f32]) {
        let data_len = (samples.len() * 4) as u32;
        let mut out = Vec::with_capacity(58 + samples.len() * 4);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(50 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&18u32.to_le_bytes());
        out.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * channels as u32 * 4).to_le_bytes());
        out.extend_from_slice(&(channels * 4).to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(b"fact");
        out.extend_from_slice(&4u32.to_le_bytes());
        out.extend_from_slice(&((samples.len() / channels as usize) as u32).to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(path, out).unwrap();
    }

    /// Interleaved sine on every channel: `amp` linear, `phase` in radians.
    fn sine(rate: u32, channels: u16, freq: f64, amp: f64, seconds: f64, phase: f64) -> Vec<f32> {
        let frames = (rate as f64 * seconds) as usize;
        let mut v = Vec::with_capacity(frames * channels as usize);
        for i in 0..frames {
            let s = (amp * (std::f64::consts::TAU * freq * i as f64 / rate as f64 + phase).sin()) as f32;
            for _ in 0..channels {
                v.push(s);
            }
        }
        v
    }

    fn db(x: f64) -> f64 {
        10f64.powf(x / 20.0)
    }

    fn not_cancelled() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn sine_1khz_at_minus_20_dbfs_measures_minus_23_lufs() {
        let dir = TempDir::new("sine");
        let path = dir.file("sine.wav");
        // BS.1770: a full-scale 1 kHz sine in one channel is -3.01 LUFS, so -20 dBFS mono
        // is -23.01 LUFS.
        write_wav_f32(&path, 48_000, 1, &sine(48_000, 1, 1000.0, db(-20.0), 20.0, 0.0));
        let r = measure_loudness(&path, &not_cancelled()).unwrap();
        assert!((r.integrated_lufs - -23.0).abs() <= 0.1, "integrated {}", r.integrated_lufs);
        assert!((r.gain_db() - 5.0).abs() <= 0.1, "gain {}", r.gain_db());
        assert_eq!(r.duration_ms, 20_000);
        // Sample-aligned 1 kHz at 48 kHz: the true peak is the amplitude.
        assert!((r.true_peak as f64 - db(-20.0)).abs() < 0.002, "peak {}", r.true_peak);
    }

    #[test]
    fn full_scale_mono_sine_is_minus_3_lufs_at_44k1() {
        let dir = TempDir::new("fs");
        let path = dir.file("fs.wav");
        write_wav_f32(&path, 44_100, 1, &sine(44_100, 1, 1000.0, 1.0, 10.0, 0.0));
        let r = measure_loudness(&path, &not_cancelled()).unwrap();
        assert!((r.integrated_lufs - -3.01).abs() <= 0.1, "integrated {}", r.integrated_lufs);
    }

    #[test]
    fn ebu_tech_3341_case_1_stereo_sine_at_minus_23_dbfs() {
        // EBU Tech 3341 test 1: 1 kHz sine at -23 dBFS in both channels = -23.0 ±0.1 LUFS.
        let dir = TempDir::new("3341");
        let path = dir.file("stereo.wav");
        write_wav_f32(&path, 48_000, 2, &sine(48_000, 2, 1000.0, db(-23.0), 20.0, 0.0));
        let r = measure_loudness(&path, &not_cancelled()).unwrap();
        assert!((r.integrated_lufs - -23.0).abs() <= 0.1, "integrated {}", r.integrated_lufs);
    }

    #[test]
    fn true_peak_catches_inter_sample_peaks_and_sample_peak_does_not() {
        let dir = TempDir::new("peak");
        let path = dir.file("isp.wav");
        // fs/4 sine with a 45° phase: every sample is at ±amp·0.7071, the waveform between
        // them reaches ±amp (≈ +3 dB inter-sample over).
        let amp = 0.9;
        write_wav_f32(
            &path,
            48_000,
            2,
            &sine(48_000, 2, 12_000.0, amp, 5.0, std::f64::consts::FRAC_PI_4),
        );
        let tp = measure_loudness(&path, &not_cancelled()).unwrap();
        let sp = measure_loudness_with(&path, &not_cancelled(), PeakMode::Sample).unwrap();
        assert!((sp.true_peak as f64 - amp * std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-3, "sample peak {}", sp.true_peak);
        assert!(tp.true_peak as f64 > amp * 0.95, "true peak {} should approach {amp}", tp.true_peak);
        assert!((tp.integrated_lufs - sp.integrated_lufs).abs() < 1e-9);
    }

    #[test]
    fn mono_is_measured_with_unit_weight() {
        let dir = TempDir::new("mono");
        let (mono, stereo) = (dir.file("mono.wav"), dir.file("stereo.wav"));
        write_wav_f32(&mono, 48_000, 1, &sine(48_000, 1, 1000.0, db(-20.0), 10.0, 0.0));
        write_wav_f32(&stereo, 48_000, 2, &sine(48_000, 2, 1000.0, db(-20.0), 10.0, 0.0));
        let m = measure_loudness(&mono, &not_cancelled()).unwrap();
        let s = measure_loudness(&stereo, &not_cancelled()).unwrap();
        // The same signal in one channel is 3 dB below both channels (no dual-mono boost).
        assert!((s.integrated_lufs - m.integrated_lufs - 3.01).abs() <= 0.05, "{} vs {}", s.integrated_lufs, m.integrated_lufs);
    }

    #[test]
    fn silence_is_finite_and_gain_is_clamped() {
        let dir = TempDir::new("silence");
        let path = dir.file("silence.wav");
        write_wav_f32(&path, 44_100, 2, &vec![0.0; 44_100 * 2 * 3]);
        let r = measure_loudness(&path, &not_cancelled()).unwrap();
        assert_eq!(r.integrated_lufs, SILENCE_LUFS);
        assert_eq!(r.true_peak, 0.0);
        assert_eq!(r.gain_db(), MAX_GAIN_DB);
    }

    #[test]
    fn cancellation_stops_the_measurement() {
        let dir = TempDir::new("cancel");
        let path = dir.file("long.wav");
        write_wav_f32(&path, 44_100, 2, &sine(44_100, 2, 440.0, 0.5, 30.0, 0.0));
        let cancel = AtomicBool::new(true);
        let err = measure_loudness(&path, &cancel).unwrap_err();
        assert!(err.is::<MeasureCancelled>(), "{err:#}");

        // Cancelled from another thread while running.
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let p2 = path.clone();
        let worker = std::thread::spawn(move || measure_loudness(&p2, &c2));
        cancel.store(true, Ordering::Relaxed);
        // Finishing before the flag was seen is also fine.
        if let Err(e) = worker.join().unwrap() {
            assert!(e.is::<MeasureCancelled>(), "{e:#}");
        }
    }

    #[test]
    fn missing_or_corrupt_files_are_errors() {
        let dir = TempDir::new("bad");
        assert!(measure_loudness(&dir.file("nope.wav"), &not_cancelled()).is_err());
        let junk = dir.file("junk.wav");
        std::fs::write(&junk, b"not audio at all").unwrap();
        assert!(measure_loudness(&junk, &not_cancelled()).is_err());
        assert!(!has_replaygain_tags(&junk));
    }

    #[test]
    fn album_gain_is_energy_weighted_by_duration_with_max_peak() {
        let t = |lufs: f64, peak: f32, ms: u64| TrackLoudness { integrated_lufs: lufs, true_peak: peak, duration_ms: ms };
        // Equal loudness: album = track.
        let (g, p) = album_gain(&[t(-20.0, 0.5, 1000), t(-20.0, 0.8, 3000)]);
        assert!((g - 2.0).abs() < 1e-4, "{g}");
        assert_eq!(p, 0.8);
        // Energy, not dB, is averaged: -10 and -20 LUFS for equal time is
        // 10*log10((10^-1 + 10^-2)/2) ≈ -12.596 LUFS (+ the 0.691 offsets cancel).
        let (g, _) = album_gain(&[t(-10.0, 0.1, 60_000), t(-20.0, 0.1, 60_000)]);
        let expected = -18.0 - (10.0 * ((0.1f64 + 0.01) / 2.0).log10());
        assert!((g as f64 - expected).abs() < 1e-3, "{g} vs {expected}");
        // Weighted by duration: 3:1 towards the quieter track.
        let (g, _) = album_gain(&[t(-10.0, 0.1, 30_000), t(-20.0, 0.1, 90_000)]);
        let expected = -18.0 - (10.0 * ((0.1f64 + 3.0 * 0.01) / 4.0).log10());
        assert!((g as f64 - expected).abs() < 1e-3, "{g} vs {expected}");
        // Silent tracks are gated out but still count for the peak.
        let (g, p) = album_gain(&[t(-20.0, 0.3, 1000), t(SILENCE_LUFS, 0.0, 100_000)]);
        assert!((g - 2.0).abs() < 1e-4);
        assert_eq!(p, 0.3);
        assert_eq!(album_gain(&[]), (0.0, 0.0));
    }

    #[test]
    fn album_gain_matches_measuring_the_concatenation() {
        let dir = TempDir::new("album");
        let a = sine(44_100, 2, 1000.0, db(-14.0), 8.0, 0.0);
        let b = sine(44_100, 2, 500.0, db(-24.0), 12.0, 0.0);
        let (pa, pb, pab) = (dir.file("a.wav"), dir.file("b.wav"), dir.file("ab.wav"));
        write_wav_f32(&pa, 44_100, 2, &a);
        write_wav_f32(&pb, 44_100, 2, &b);
        write_wav_f32(&pab, 44_100, 2, &[a.clone(), b.clone()].concat());
        let ra = measure_loudness(&pa, &not_cancelled()).unwrap();
        let rb = measure_loudness(&pb, &not_cancelled()).unwrap();
        let rab = measure_loudness(&pab, &not_cancelled()).unwrap();
        let (g, p) = album_gain(&[ra, rb]);
        // Both tracks are above the album's relative gate, so the approximation is close.
        assert!((g - rab.gain_db()).abs() < 0.2, "album {g} vs concatenated {}", rab.gain_db());
        assert!((p - rab.true_peak).abs() < 0.01);
    }

    /// Minimal FLAC: STREAMINFO + VORBIS_COMMENT with `comments`, then one frame of 192
    /// silent stereo samples (two CONSTANT subframes).
    fn write_flac_tags(path: &Path, comments: &[&str]) {
        let mut out = b"fLaC".to_vec();
        out.extend_from_slice(&[0x00, 0, 0, 34]); // STREAMINFO, not last, 34 bytes
        out.extend_from_slice(&192u16.to_be_bytes());
        out.extend_from_slice(&192u16.to_be_bytes());
        out.extend_from_slice(&[0; 6]); // min/max frame size unknown
        // 44100 Hz (20 bits) | 2 ch - 1 (3 bits) | 16 bps - 1 (5 bits) | 192 samples (36 bits)
        let packed: u64 = (44_100u64 << 44) | (1u64 << 41) | (15u64 << 36) | 192u64;
        out.extend_from_slice(&packed.to_be_bytes());
        out.extend_from_slice(&[0; 16]); // MD5
        let mut vc = Vec::new();
        let vendor = b"test";
        vc.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        vc.extend_from_slice(vendor);
        vc.extend_from_slice(&(comments.len() as u32).to_le_bytes());
        for c in comments {
            vc.extend_from_slice(&(c.len() as u32).to_le_bytes());
            vc.extend_from_slice(c.as_bytes());
        }
        out.push(0x80 | 4); // VORBIS_COMMENT, last block
        out.extend_from_slice(&(vc.len() as u32).to_be_bytes()[1..]);
        out.extend_from_slice(&vc);
        let crc8 = |data: &[u8]| {
            data.iter().fold(0u8, |mut crc, &b| {
                crc ^= b;
                for _ in 0..8 {
                    crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
                }
                crc
            })
        };
        let crc16 = |data: &[u8]| {
            data.iter().fold(0u16, |mut crc, &b| {
                crc ^= (b as u16) << 8;
                for _ in 0..8 {
                    crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x8005 } else { crc << 1 };
                }
                crc
            })
        };
        // Sync (fixed block size) | 192 samples, 44.1 kHz | stereo, 16 bit | frame 0.
        let mut frame = vec![0xFF, 0xF8, 0x19, 0x18, 0x00];
        frame.push(crc8(&frame));
        // Two CONSTANT subframes (header 0x00) with value 0.
        frame.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        let crc = crc16(&frame);
        frame.extend_from_slice(&crc.to_be_bytes());
        out.extend_from_slice(&frame);
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn detects_replaygain_tags() {
        let dir = TempDir::new("tags");
        let plain = dir.file("plain.wav");
        write_wav_f32(&plain, 44_100, 2, &sine(44_100, 2, 440.0, 0.1, 0.5, 0.0));
        assert!(!has_replaygain_tags(&plain));
        let tagged = dir.file("tagged.flac");
        write_flac_tags(&tagged, &["TITLE=x", "REPLAYGAIN_TRACK_GAIN=-6.50 dB", "REPLAYGAIN_TRACK_PEAK=0.98"]);
        assert!(has_replaygain_tags(&tagged));
        let album_only = dir.file("album.flac");
        write_flac_tags(&album_only, &["replaygain_album_gain=-3.00 dB"]);
        assert!(has_replaygain_tags(&album_only));
        // A gain the engine cannot parse does not count (the file gets measured instead).
        let garbage = dir.file("garbage.flac");
        write_flac_tags(&garbage, &["REPLAYGAIN_TRACK_GAIN=loud"]);
        assert!(!has_replaygain_tags(&garbage));
        let untagged = dir.file("untagged.flac");
        write_flac_tags(&untagged, &["TITLE=x", "ARTIST=y"]);
        assert!(!has_replaygain_tags(&untagged));
    }

    /// CPU cost per minute of audio (run with `--ignored --nocapture`).
    #[test]
    #[ignore]
    fn measure_speed() {
        let dir = TempDir::new("speed");
        let path = dir.file("speed.wav");
        let minutes = 3.0;
        // Noise-like content (sum of sines) so nothing is trivially cheap.
        let rate = 44_100;
        let frames = (rate as f64 * 60.0 * minutes) as usize;
        let mut v = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let t = i as f64 / rate as f64;
            let s = 0.2 * (t * 440.0 * std::f64::consts::TAU).sin()
                + 0.1 * (t * 3_217.0 * std::f64::consts::TAU).sin()
                + 0.05 * (t * 9_901.0 * std::f64::consts::TAU).sin();
            v.push(s as f32);
            v.push((s * 0.9) as f32);
        }
        write_wav_f32(&path, rate, 2, &v);
        // Extra real files: LOUDNESS_BENCH_FILES="a.flac;b.mp3".
        let mut files = vec![path];
        if let Ok(list) = std::env::var("LOUDNESS_BENCH_FILES") {
            files.extend(list.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
        }
        for file in &files {
            for peak in [PeakMode::Sample, PeakMode::True] {
                let start = Instant::now();
                let r = measure_loudness_with(file, &not_cancelled(), peak).unwrap();
                let secs = start.elapsed().as_secs_f64();
                let minutes = r.duration_ms as f64 / 60_000.0;
                println!(
                    "{}: {peak:?}: {secs:.3} s for {minutes:.2} min ({:.3} s per minute), {:.2} LUFS, peak {:.3}",
                    file.file_name().unwrap_or_default().to_string_lossy(),
                    secs / minutes,
                    r.integrated_lufs,
                    r.true_peak
                );
            }
        }
    }
}
