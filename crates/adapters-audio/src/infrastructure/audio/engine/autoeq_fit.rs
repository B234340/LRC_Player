//! Automatic parametric-EQ fitting ("AutoEq"): reads measured / target frequency responses
//! from text files and fits up to [`PEQ_MAX_FILTERS`] RBJ filters that bring a measured
//! (source) response onto a target response. Pure code (no I/O, no threads, deterministic).
//!
//! # Curve files ([`parse_frequency_response`])
//!
//! Accepts REW text exports (`* ...` header lines, `freq spl phase` columns), AutoEQ /
//! squig.link CSV (`frequency,raw` header), microphone calibration files and generic
//! whitespace / comma / semicolon separated columns. Rules (ported from IEM_Measure's
//! `read_curve_text`):
//!
//! * lines starting with one of `* # " ' % ; /` or with a letter are skipped (headers,
//!   comments, `"Sens Factor ..."`);
//! * a line containing both `;` and `,` is European CSV: `;` separates, `,` is the decimal
//!   point;
//! * the first three tokens (split on whitespace, `,` and `;`) must be numbers, else the line
//!   is skipped; column 1 = frequency in Hz, column 2 = level in dB (column 3, e.g. phase, is
//!   ignored);
//! * points with a non-finite or non-positive frequency or a non-finite level are dropped; the
//!   rest is sorted by frequency (duplicates keep the first) and at least 2 points are needed.
//!
//! Text decoding: UTF-16 with a BOM, UTF-8 (with or without BOM), anything else byte-wise
//! (Latin-1). Shift-JIS / CP932 files therefore load too: their numbers are plain ASCII and
//! Japanese header/comment lines are skipped either way (a CP932 lead byte never forms a
//! digit), so no encoding tables are needed.
//!
//! # Fitting algorithm ([`auto_eq`])
//!
//! 1. Both curves are resampled on a log grid (1/48 octave, 20 Hz–20 kHz; linear
//!    interpolation in log-frequency, the end values held beyond the data) and smoothed with
//!    a moving average of `smoothing_octave` octaves.
//! 2. The source is shifted so that its mean difference to the target inside
//!    `normalize_band` is 0 (measurements are in arbitrary SPL).
//! 3. Residual `r = target − source` inside `[freq_min_hz, freq_max_hz]` (weight 1, reduced to
//!    [`TREBLE_WEIGHT`] above 10 kHz where measurements are unreliable, 0 outside); `r` is
//!    clamped to ±`max_gain_db`, and above 8 kHz boosts are capped at
//!    `treble_boost_limit_db`.
//! 4. Greedy initialisation: with `use_shelves`, a low shelf (105 Hz) and a high shelf
//!    (10 kHz) with least-squares gains (kept when ≥ 0.5 dB); then one peak after another at
//!    the largest weighted remaining residual, with gain = that residual and Q from its
//!    half-gain bandwidth, followed by a short joint optimisation.
//! 5. Joint optimisation of every parameter (log f, gain, log Q; shelves keep Q = 0.707)
//!    within the bounds: projected Levenberg–Marquardt with a forward-difference Jacobian on
//!    the closed-form RBJ magnitude response at 48 kHz. Loss = weighted mean squared error +
//!    a hinge penalty on combined boosts above the treble limit (above 8 kHz) + a small gain
//!    regularisation (`λ·Σ gain²`).
//! 6. Filters below 0.2 dB are dropped (then the rest is re-optimised), values are rounded
//!    (1 Hz, 0.1 dB, Q 0.01), filters sorted by frequency, and the preamp is set so the
//!    combined EQ response peaks at ≤ 0 dB.
//!
//! Everything is deterministic (no randomness, fixed iteration order). With 20 filters it
//! takes a few tens of milliseconds in an optimised build.

use std::fmt;

use super::peq::{FilterKind, PeqFilter, PEQ_MAX_FILTERS, PEQ_MAX_GAIN_DB};

/// Largest curve file accepted by [`parse_frequency_response`].
pub const CURVE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Lowest / highest frequency of the fitting grid.
pub const FIT_MIN_HZ: f32 = 20.0;
pub const FIT_MAX_HZ: f32 = 20_000.0;
/// Resolution of the fitting grid.
pub const FIT_POINTS_PER_OCTAVE: f32 = 48.0;

/// Sample rate the filter responses are evaluated at.
const FIT_FS: f64 = 48_000.0;
/// Q of the shelves (RBJ slope 1: the steepest without overshoot).
const SHELF_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;
const LOW_SHELF_HZ: f64 = 105.0;
const HIGH_SHELF_HZ: f64 = 10_000.0;
const LOW_SHELF_RANGE: (f64, f64) = (30.0, 300.0);
const HIGH_SHELF_RANGE: (f64, f64) = (4_000.0, 16_000.0);
/// Above this frequency boosts are limited by `treble_boost_limit_db`.
const TREBLE_CAP_HZ: f64 = 8_000.0;
/// Above this frequency the residual counts with [`TREBLE_WEIGHT`].
const TREBLE_WEIGHT_HZ: f64 = 10_000.0;
/// Weight of the residual above 10 kHz (measurements of IEMs scatter a lot up there).
pub const TREBLE_WEIGHT: f32 = 0.3;
/// Weight of the hinge penalty on treble boosts above the limit (relative to the data term).
const HINGE_WEIGHT: f64 = 30.0;
/// Gain regularisation (per dB², added to the weighted mean squared error in dB²).
const GAIN_REG: f64 = 2.0e-4;
/// Filters with a smaller |gain| are dropped.
const MIN_GAIN_DB: f64 = 0.2;
/// Shelves found by the initialisation are kept only from this |gain| on.
const MIN_SHELF_GAIN_DB: f64 = 0.5;
/// Levenberg–Marquardt iterations after each greedy peak / at the end.
const STEP_ITERS: usize = 8;
const FINAL_ITERS: usize = 80;

// ----- curves --------------------------------------------------------------------------------

/// A magnitude response: `db[i]` at `freqs[i]` Hz (frequencies ascending and positive when it
/// comes from [`parse_frequency_response`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrequencyResponse {
    pub freqs: Vec<f32>,
    pub db: Vec<f32>,
}

/// Why a curve file could not be read (the app translates these).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveParseError {
    /// Larger than [`CURVE_MAX_BYTES`].
    TooLarge,
    /// No line with a frequency and a level.
    NoData,
    /// Fewer than 2 usable points (the count found).
    TooFewPoints(usize),
}

impl fmt::Display for CurveParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CurveParseError::TooLarge => {
                write!(f, "file too large (over {} MiB)", CURVE_MAX_BYTES >> 20)
            }
            CurveParseError::NoData => write!(f, "no frequency / dB data found"),
            CurveParseError::TooFewPoints(n) => {
                write!(f, "only {n} usable point(s); at least 2 are needed")
            }
        }
    }
}

impl std::error::Error for CurveParseError {}

/// Decodes a text file: UTF-16 with BOM, UTF-8 (BOM stripped), else Latin-1 (see module docs).
fn decode_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && (bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF])) {
        let le = bytes[0] == 0xFF;
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(body) {
        Ok(s) => s.to_owned(),
        Err(_) => body.iter().map(|&b| b as char).collect(),
    }
}

/// Parses a frequency-response file (see the module docs for the accepted formats).
pub fn parse_frequency_response(bytes: &[u8]) -> Result<FrequencyResponse, CurveParseError> {
    if bytes.len() > CURVE_MAX_BYTES {
        return Err(CurveParseError::TooLarge);
    }
    parse_frequency_response_str(&decode_text(bytes))
}

/// [`parse_frequency_response`] on already decoded text.
pub fn parse_frequency_response_str(text: &str) -> Result<FrequencyResponse, CurveParseError> {
    let mut pts: Vec<(f32, f32)> = Vec::new();
    for line in text.lines() {
        let s = line.trim().trim_start_matches('\u{feff}').trim_start();
        let Some(first) = s.chars().next() else {
            continue;
        };
        if "*#\"'%;/".contains(first) || first.is_alphabetic() {
            continue;
        }
        let european;
        let s = if s.contains(';') && s.contains(',') {
            european = s.replace(',', ".");
            european.as_str()
        } else {
            s
        };
        let mut vals = [0.0f64; 3];
        let mut n = 0;
        let mut ok = true;
        for tok in s
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .filter(|t| !t.is_empty())
            .take(3)
        {
            match tok.parse::<f64>() {
                Ok(v) => {
                    vals[n] = v;
                    n += 1;
                }
                Err(_) => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok || n < 2 {
            continue;
        }
        let (f, d) = (vals[0] as f32, vals[1] as f32);
        if f.is_finite() && f > 0.0 && d.is_finite() {
            pts.push((f, d));
        }
    }
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    pts.dedup_by(|later, earlier| later.0 == earlier.0);
    match pts.len() {
        0 => Err(CurveParseError::NoData),
        1 => Err(CurveParseError::TooFewPoints(1)),
        _ => Ok(FrequencyResponse {
            freqs: pts.iter().map(|p| p.0).collect(),
            db: pts.iter().map(|p| p.1).collect(),
        }),
    }
}

impl FrequencyResponse {
    /// 0 dB everywhere (a "flat" target).
    pub fn flat() -> Self {
        FrequencyResponse {
            freqs: vec![FIT_MIN_HZ, FIT_MAX_HZ],
            db: vec![0.0, 0.0],
        }
    }

    /// Level at `hz`: linear interpolation in log-frequency, the end values held outside the
    /// data. 0 for an empty curve.
    pub fn db_at(&self, hz: f32) -> f32 {
        let n = self.freqs.len().min(self.db.len());
        if n == 0 {
            return 0.0;
        }
        if n == 1 || hz <= self.freqs[0] {
            return self.db[0];
        }
        if hz >= self.freqs[n - 1] {
            return self.db[n - 1];
        }
        let i = self.freqs[..n].partition_point(|f| *f < hz).clamp(1, n - 1);
        let (f0, f1) = (
            self.freqs[i - 1].max(1e-6) as f64,
            self.freqs[i].max(1e-6) as f64,
        );
        let (d0, d1) = (self.db[i - 1] as f64, self.db[i] as f64);
        let span = (f1 / f0).ln();
        if span <= 0.0 {
            return d1 as f32;
        }
        let t = ((hz as f64 / f0).ln() / span).clamp(0.0, 1.0);
        (d0 + t * (d1 - d0)) as f32
    }

    /// [`FrequencyResponse::db_at`] for each of `freqs`.
    pub fn resample(&self, freqs: &[f32]) -> Vec<f32> {
        freqs.iter().map(|&f| self.db_at(f)).collect()
    }

    /// The mean (in dB) of two responses, on the fitting grid (e.g. left and right earpiece).
    pub fn average(a: &FrequencyResponse, b: &FrequencyResponse) -> FrequencyResponse {
        let freqs = fit_grid();
        let db = freqs
            .iter()
            .map(|&f| 0.5 * (a.db_at(f) + b.db_at(f)))
            .collect();
        FrequencyResponse { freqs, db }
    }
}

/// The fitting grid: log spaced, [`FIT_POINTS_PER_OCTAVE`], from [`FIT_MIN_HZ`] to
/// [`FIT_MAX_HZ`] (both included).
pub fn fit_grid() -> Vec<f32> {
    let octaves = (FIT_MAX_HZ as f64 / FIT_MIN_HZ as f64).log2();
    let n = (octaves * FIT_POINTS_PER_OCTAVE as f64).round() as usize + 1;
    (0..n)
        .map(|i| (FIT_MIN_HZ as f64 * 2f64.powf(octaves * i as f64 / (n - 1) as f64)) as f32)
        .collect()
}

/// Points per octave of the grid returned by [`fit_grid`] (exact, not rounded).
fn grid_ppo(n: usize) -> f64 {
    (n - 1) as f64 / (FIT_MAX_HZ as f64 / FIT_MIN_HZ as f64).log2()
}

/// Moving average over `octave` octaves on a log grid with `ppo` points per octave (window
/// truncated at the ends). `octave <= 0` returns the input.
fn smooth(db: &[f64], ppo: f64, octave: f64) -> Vec<f64> {
    let half = (octave.max(0.0) * ppo / 2.0).round() as usize;
    if half == 0 || db.len() < 3 {
        return db.to_vec();
    }
    let mut prefix = Vec::with_capacity(db.len() + 1);
    prefix.push(0.0);
    for v in db {
        prefix.push(prefix.last().copied().unwrap_or(0.0) + v);
    }
    (0..db.len())
        .map(|i| {
            let lo = i.saturating_sub(half);
            let hi = (i + half).min(db.len() - 1);
            (prefix[hi + 1] - prefix[lo]) / (hi + 1 - lo) as f64
        })
        .collect()
}

// ----- options / results ---------------------------------------------------------------------

/// Options of [`auto_eq`]. Out-of-range values are brought into range
/// ([`AutoEqOptions::sanitized`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoEqOptions {
    /// Maximum number of filters (shelves included), `1..=PEQ_MAX_FILTERS`.
    pub max_filters: usize,
    /// Largest |gain| of a filter (dB); also the largest correction aimed for.
    pub max_gain_db: f32,
    /// Q range of the peaks.
    pub min_q: f32,
    pub max_q: f32,
    /// Frequency range that is corrected (peaks stay inside it).
    pub freq_min_hz: f32,
    pub freq_max_hz: f32,
    /// Largest boost above ~8 kHz (dB).
    pub treble_boost_limit_db: f32,
    /// One low shelf (~105 Hz) and one high shelf (~10 kHz) may be used.
    pub use_shelves: bool,
    /// Smoothing of both curves before fitting (octaves; 0 = none).
    pub smoothing_octave: f32,
    /// Band (Hz) in which the source is level-matched to the target.
    pub normalize_band: (f32, f32),
}

impl Default for AutoEqOptions {
    fn default() -> Self {
        AutoEqOptions {
            max_filters: 10,
            max_gain_db: 12.0,
            min_q: 0.5,
            max_q: 6.0,
            freq_min_hz: 20.0,
            freq_max_hz: 16_000.0,
            treble_boost_limit_db: 3.0,
            use_shelves: true,
            smoothing_octave: 1.0 / 12.0,
            normalize_band: (250.0, 2500.0),
        }
    }
}

fn finite_or(v: f32, d: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        d
    }
}

impl AutoEqOptions {
    /// Every field finite and in range (a usable frequency range of at least half an octave,
    /// `min_q <= max_q`, treble limit within ±max gain).
    pub fn sanitized(&self) -> AutoEqOptions {
        let d = AutoEqOptions::default();
        let max_gain_db = finite_or(self.max_gain_db, d.max_gain_db).clamp(0.5, PEQ_MAX_GAIN_DB);
        let min_q = finite_or(self.min_q, d.min_q).clamp(0.1, 10.0);
        let max_q = finite_or(self.max_q, d.max_q).clamp(min_q, 10.0);
        let mut freq_min_hz =
            finite_or(self.freq_min_hz, d.freq_min_hz).clamp(FIT_MIN_HZ, FIT_MAX_HZ);
        let mut freq_max_hz =
            finite_or(self.freq_max_hz, d.freq_max_hz).clamp(FIT_MIN_HZ, FIT_MAX_HZ);
        if freq_max_hz < freq_min_hz * 1.42 {
            freq_max_hz = (freq_min_hz * 1.42).min(FIT_MAX_HZ);
            freq_min_hz = freq_min_hz.min(freq_max_hz / 1.42);
        }
        let (mut b0, mut b1) = (
            finite_or(self.normalize_band.0, d.normalize_band.0).clamp(FIT_MIN_HZ, FIT_MAX_HZ),
            finite_or(self.normalize_band.1, d.normalize_band.1).clamp(FIT_MIN_HZ, FIT_MAX_HZ),
        );
        if b1 < b0 {
            std::mem::swap(&mut b0, &mut b1);
        }
        if b1 < b0 * 1.05 {
            (b0, b1) = d.normalize_band;
        }
        AutoEqOptions {
            max_filters: self.max_filters.clamp(1, PEQ_MAX_FILTERS),
            max_gain_db,
            min_q,
            max_q,
            freq_min_hz,
            freq_max_hz,
            treble_boost_limit_db: finite_or(self.treble_boost_limit_db, d.treble_boost_limit_db)
                .clamp(0.0, max_gain_db),
            use_shelves: self.use_shelves,
            smoothing_octave: finite_or(self.smoothing_octave, d.smoothing_octave).clamp(0.0, 1.0),
            normalize_band: (b0, b1),
        }
    }
}

/// Result of [`auto_eq`].
#[derive(Clone, Debug, PartialEq)]
pub struct AutoEqResult {
    /// Preamp (dB, <= 0) so the combined filter response peaks at <= 0 dB.
    pub preamp_db: f32,
    /// The filters, sorted by frequency.
    pub filters: Vec<PeqFilter>,
    /// Weighted RMS of `target − source` (dB) over the corrected range (weights as in the
    /// fit: 1, [`TREBLE_WEIGHT`] above 10 kHz), before and after the EQ (filters only).
    pub rms_error_before_db: f32,
    pub rms_error_after_db: f32,
    /// Level shift applied to the source (dB) to match the target in the normalize band.
    pub source_offset_db: f32,
}

/// Source and target as the fit sees them: on [`fit_grid`], smoothed, the source shifted by
/// `source_offset_db` (see [`prepare_curves`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedCurves {
    pub freqs: Vec<f32>,
    pub source_db: Vec<f32>,
    pub target_db: Vec<f32>,
    pub source_offset_db: f32,
}

/// Steps 1–2 of the algorithm: resample, smooth and level-match the source to the target.
pub fn prepare_curves(
    source: &FrequencyResponse,
    target: &FrequencyResponse,
    opts: &AutoEqOptions,
) -> PreparedCurves {
    let opts = opts.sanitized();
    let freqs = fit_grid();
    let ppo = grid_ppo(freqs.len());
    let oct = opts.smoothing_octave as f64;
    let s = smooth(
        &source
            .resample(&freqs)
            .iter()
            .map(|&v| v as f64)
            .collect::<Vec<_>>(),
        ppo,
        oct,
    );
    let t = smooth(
        &target
            .resample(&freqs)
            .iter()
            .map(|&v| v as f64)
            .collect::<Vec<_>>(),
        ppo,
        oct,
    );
    let (b0, b1) = opts.normalize_band;
    let (mut sum, mut n) = (0.0, 0usize);
    for (i, &f) in freqs.iter().enumerate() {
        if f >= b0 && f <= b1 {
            sum += t[i] - s[i];
            n += 1;
        }
    }
    let offset = if n > 0 {
        sum / n as f64
    } else {
        let f = (b0 * b1).sqrt();
        (target.db_at(f) - source.db_at(f)) as f64
    };
    PreparedCurves {
        source_db: s.iter().map(|v| (v + offset) as f32).collect(),
        target_db: t.iter().map(|&v| v as f32).collect(),
        freqs,
        source_offset_db: offset as f32,
    }
}

// ----- filter responses (f64, closed form) ----------------------------------------------------

/// One filter during the fit: log frequency, gain (dB), log Q.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fp {
    kind: FilterKind,
    lf: f64,
    g: f64,
    lq: f64,
}

impl Fp {
    fn new(kind: FilterKind, f: f64, g: f64, q: f64) -> Self {
        Fp {
            kind,
            lf: f.ln(),
            g,
            lq: q.ln(),
        }
    }

    fn is_shelf(&self) -> bool {
        matches!(self.kind, FilterKind::LowShelf | FilterKind::HighShelf)
    }

    /// Number of free parameters (shelves keep their Q).
    fn free_params(&self) -> usize {
        if self.is_shelf() {
            2
        } else {
            3
        }
    }

    fn get(&self, k: usize) -> f64 {
        match k {
            0 => self.lf,
            1 => self.g,
            _ => self.lq,
        }
    }

    fn set(&mut self, k: usize, v: f64) {
        match k {
            0 => self.lf = v,
            1 => self.g = v,
            _ => self.lq = v,
        }
    }

    /// RBJ cookbook coefficients folded into the squared-magnitude polynomial in cos(w):
    /// `|H|² = (n0 + n1 cos w + n2 cos 2w) / (d0 + d1 cos w + d2 cos 2w)`.
    fn poly(&self) -> [f64; 6] {
        let f0 = self.lf.exp().clamp(1.0, FIT_FS * 0.45);
        let q = self.lq.exp().max(1e-3);
        let a = 10f64.powf(self.g / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * f0 / FIT_FS;
        let (sn, cs) = w0.sin_cos();
        let alpha = sn / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match self.kind {
            FilterKind::LowShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cs + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
                    a * ((a + 1.0) - (a - 1.0) * cs - k),
                    (a + 1.0) + (a - 1.0) * cs + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cs),
                    (a + 1.0) + (a - 1.0) * cs - k,
                )
            }
            FilterKind::HighShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cs + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                    a * ((a + 1.0) + (a - 1.0) * cs - k),
                    (a + 1.0) - (a - 1.0) * cs + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * cs),
                    (a + 1.0) - (a - 1.0) * cs - k,
                )
            }
            // Only peaks and shelves are fitted.
            _ => (
                1.0 + alpha * a,
                -2.0 * cs,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cs,
                1.0 - alpha / a,
            ),
        };
        let (b0, b1, b2, a1, a2) = (b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0);
        [
            b0 * b0 + b1 * b1 + b2 * b2,
            2.0 * (b0 * b1 + b1 * b2),
            2.0 * b0 * b2,
            1.0 + a1 * a1 + a2 * a2,
            2.0 * (a1 + a1 * a2),
            2.0 * a2,
        ]
    }

    /// Magnitude response (dB) at the points of `grid` into `out`.
    fn response_into(&self, grid: &Points, out: &mut [f64]) {
        let p = self.poly();
        for ((o, c1), c2) in out.iter_mut().zip(&grid.c1).zip(&grid.c2) {
            let num = p[0] + p[1] * c1 + p[2] * c2;
            let den = p[3] + p[4] * c1 + p[5] * c2;
            *o = 10.0 * (num.max(1e-300) / den.max(1e-300)).log10();
        }
    }

    fn response(&self, grid: &Points) -> Vec<f64> {
        let mut v = vec![0.0; grid.c1.len()];
        self.response_into(grid, &mut v);
        v
    }

    fn to_filter(self) -> PeqFilter {
        PeqFilter {
            kind: self.kind,
            freq_hz: self.lf.exp() as f32,
            gain_db: self.g as f32,
            q: self.lq.exp() as f32,
        }
    }
}

/// Frequencies with `cos w` and `cos 2w` precomputed (w at [`FIT_FS`]).
struct Points {
    c1: Vec<f64>,
    c2: Vec<f64>,
}

impl Points {
    fn new(freqs: impl Iterator<Item = f64>) -> Self {
        let (mut c1, mut c2) = (Vec::new(), Vec::new());
        for f in freqs {
            let w = 2.0 * std::f64::consts::PI * f / FIT_FS;
            c1.push(w.cos());
            c2.push((2.0 * w).cos());
        }
        Points { c1, c2 }
    }
}

// ----- the fit -------------------------------------------------------------------------------

/// The least-squares problem on the points inside the corrected range.
struct Problem {
    pts: Points,
    /// Frequencies of the points (Hz).
    freqs: Vec<f64>,
    /// Residual to fit (target − source, clamped / treble-capped).
    r: Vec<f64>,
    /// sqrt of the normalised data weight of each point.
    sw: Vec<f64>,
    /// Points above the treble cap (indices) and the hinge weight (sqrt, normalised).
    treble: Vec<usize>,
    sh: f64,
    treble_limit: f64,
    sreg: f64,
    opts: AutoEqOptions,
    ppo: f64,
}

impl Problem {
    fn rows(&self, nf: usize) -> usize {
        self.r.len() + self.treble.len() + nf
    }

    /// Residual vector for the summed response `total` and the filters (for the gain term).
    fn residuals(&self, filters: &[Fp], total: &[f64], out: &mut Vec<f64>) {
        out.clear();
        for ((t, r), w) in total.iter().zip(&self.r).zip(&self.sw) {
            out.push(w * (t - r));
        }
        for &j in &self.treble {
            out.push(self.sh * (total[j] - self.treble_limit).max(0.0));
        }
        for f in filters {
            out.push(self.sreg * f.g);
        }
    }

    fn cost(&self, filters: &[Fp], resp: &[Vec<f64>], scratch: &mut Vec<f64>) -> f64 {
        let total = sum_responses(resp, self.r.len());
        self.residuals(filters, &total, scratch);
        scratch.iter().map(|e| e * e).sum()
    }

    /// Brings a filter into the bounds of the options.
    fn project(&self, f: &mut Fp) {
        let o = &self.opts;
        let gmax = o.max_gain_db as f64;
        let limit = (o.treble_boost_limit_db as f64).min(gmax);
        match f.kind {
            FilterKind::LowShelf => {
                f.lf = f.lf.clamp(LOW_SHELF_RANGE.0.ln(), LOW_SHELF_RANGE.1.ln());
                f.g = f.g.clamp(-gmax, gmax);
                f.lq = SHELF_Q.ln();
            }
            FilterKind::HighShelf => {
                f.lf = f.lf.clamp(HIGH_SHELF_RANGE.0.ln(), HIGH_SHELF_RANGE.1.ln());
                f.g = f.g.clamp(-gmax, limit);
                f.lq = SHELF_Q.ln();
            }
            _ => {
                f.lf =
                    f.lf.clamp((o.freq_min_hz as f64).ln(), (o.freq_max_hz as f64).ln());
                f.lq = f.lq.clamp((o.min_q as f64).ln(), (o.max_q as f64).ln());
                f.g = f.g.clamp(-gmax, gmax);
                if f.lf.exp() >= TREBLE_CAP_HZ {
                    f.g = f.g.min(limit);
                }
            }
        }
        if !f.g.is_finite() {
            f.g = 0.0;
        }
    }

    /// Projected Levenberg–Marquardt on every free parameter of `filters`.
    fn optimize(&self, filters: &mut [Fp], max_iter: usize) {
        if filters.is_empty() {
            return;
        }
        let m = self.r.len();
        let rows = self.rows(filters.len());
        let free: Vec<(usize, usize)> = filters
            .iter()
            .enumerate()
            .flat_map(|(i, f)| (0..f.free_params()).map(move |k| (i, k)))
            .collect();
        let p = free.len();
        let mut resp: Vec<Vec<f64>> = filters.iter().map(|f| f.response(&self.pts)).collect();
        let mut scratch = Vec::with_capacity(rows);
        let mut cost = self.cost(filters, &resp, &mut scratch);
        let mut lambda = 1e-2;
        let mut jac = vec![0.0; p * rows];
        let mut pert = vec![0.0; m];
        let mut e = Vec::with_capacity(rows);
        let mut cand_resp = resp.clone();
        for _ in 0..max_iter {
            let total = sum_responses(&resp, m);
            self.residuals(filters, &total, &mut e);
            // Jacobian, column by column (forward differences on one filter at a time).
            jac.iter_mut().for_each(|v| *v = 0.0);
            for (c, &(i, k)) in free.iter().enumerate() {
                let h = if k == 1 { 1e-4 } else { 1e-5 };
                let mut fp = filters[i];
                fp.set(k, fp.get(k) + h);
                fp.response_into(&self.pts, &mut pert);
                let col = &mut jac[c * rows..(c + 1) * rows];
                for j in 0..m {
                    col[j] = self.sw[j] * (pert[j] - resp[i][j]) / h;
                }
                for (t, &j) in self.treble.iter().enumerate() {
                    if total[j] > self.treble_limit {
                        col[m + t] = self.sh * (pert[j] - resp[i][j]) / h;
                    }
                }
                if k == 1 {
                    col[m + self.treble.len() + i] = self.sreg;
                }
            }
            // Normal equations.
            let mut a = vec![0.0; p * p];
            let mut g = vec![0.0; p];
            for c in 0..p {
                let cc = &jac[c * rows..(c + 1) * rows];
                g[c] = dot(cc, &e);
                for d in 0..=c {
                    let v = dot(cc, &jac[d * rows..(d + 1) * rows]);
                    a[c * p + d] = v;
                    a[d * p + c] = v;
                }
            }
            let mut improved = None;
            while lambda < 1e9 {
                let mut damped = a.clone();
                for c in 0..p {
                    damped[c * p + c] += lambda * a[c * p + c].max(1e-9) + 1e-12;
                }
                let Some(step) = cholesky_solve(&mut damped, &g, p) else {
                    lambda *= 4.0;
                    continue;
                };
                let mut cand: Vec<Fp> = filters.to_vec();
                for (c, &(i, k)) in free.iter().enumerate() {
                    let v = cand[i].get(k) - step[c];
                    cand[i].set(k, v);
                }
                for (i, f) in cand.iter_mut().enumerate() {
                    self.project(f);
                    f.response_into(&self.pts, &mut cand_resp[i]);
                }
                let cand_cost = self.cost(&cand, &cand_resp, &mut scratch);
                if cand_cost < cost {
                    improved = Some(cost - cand_cost);
                    cost = cand_cost;
                    filters.copy_from_slice(&cand);
                    std::mem::swap(&mut resp, &mut cand_resp);
                    lambda = (lambda / 3.0).max(1e-7);
                    break;
                }
                lambda *= 4.0;
            }
            match improved {
                Some(d) if d > 1e-9 * cost.max(1e-12) => {}
                _ => break,
            }
        }
    }

    /// Least-squares gain of a filter shape (the response at +1 dB) against `rem`.
    fn ls_gain(&self, shape: &Fp, rem: &[f64]) -> f64 {
        let mut unit = *shape;
        unit.g = 1.0;
        let u = unit.response(&self.pts);
        let (mut num, mut den) = (0.0, 0.0);
        for j in 0..rem.len() {
            let w = self.sw[j] * self.sw[j];
            num += w * rem[j] * u[j];
            den += w * u[j] * u[j];
        }
        if den > 0.0 {
            num / den
        } else {
            0.0
        }
    }

    /// Next peak from the remaining residual: at the largest weighted |residual|, gain = the
    /// residual there, Q from the half-gain bandwidth. `None` when everything left is small.
    fn next_peak(&self, rem: &[f64]) -> Option<Fp> {
        let rem = smooth(rem, self.ppo, 1.0 / 24.0);
        let (mut best, mut best_v) = (None, 0.0);
        for (j, v) in rem.iter().enumerate() {
            let score = self.sw[j] * v.abs();
            if score > best_v {
                best_v = score;
                best = Some(j);
            }
        }
        let j = best?;
        let gain = rem[j];
        if gain.abs() < MIN_GAIN_DB.max(0.3) {
            return None;
        }
        let half = gain.abs() / 2.0;
        let same = |v: f64| v.signum() == gain.signum() && v.abs() >= half;
        let mut lo = j;
        while lo > 0 && same(rem[lo - 1]) {
            lo -= 1;
        }
        let mut hi = j;
        while hi + 1 < rem.len() && same(rem[hi + 1]) {
            hi += 1;
        }
        let bw = ((self.freqs[hi] / self.freqs[lo]).log2()).max(2.0 / self.ppo);
        let p = 2f64.powf(bw);
        let q = p.sqrt() / (p - 1.0);
        let mut f = Fp::new(FilterKind::Peak, self.freqs[j], gain, q);
        self.project(&mut f);
        Some(f)
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn sum_responses(resp: &[Vec<f64>], m: usize) -> Vec<f64> {
    let mut total = vec![0.0; m];
    for r in resp {
        for (t, v) in total.iter_mut().zip(r) {
            *t += v;
        }
    }
    total
}

/// Solves `A x = b` for a symmetric positive definite `A` (n×n, overwritten by its Cholesky
/// factor). `None` if `A` is not positive definite.
fn cholesky_solve(a: &mut [f64], b: &[f64], n: usize) -> Option<Vec<f64>> {
    for i in 0..n {
        for j in 0..=i {
            let mut s = a[i * n + j];
            for k in 0..j {
                s -= a[i * n + k] * a[j * n + k];
            }
            if i == j {
                if s <= 0.0 || !s.is_finite() {
                    return None;
                }
                a[i * n + i] = s.sqrt();
            } else {
                a[i * n + j] = s / a[j * n + j];
            }
        }
    }
    let mut y = vec![0.0; n];
    for i in 0..n {
        let mut s = b[i];
        for k in 0..i {
            s -= a[i * n + k] * y[k];
        }
        y[i] = s / a[i * n + i];
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut s = y[i];
        for k in i + 1..n {
            s -= a[k * n + i] * x[k];
        }
        x[i] = s / a[i * n + i];
    }
    Some(x)
}

fn round_to(v: f64, step: f64) -> f64 {
    (v / step).round() * step
}

/// Fits a parametric EQ that brings `source` onto `target` (see the module docs for the
/// algorithm). Deterministic.
pub fn auto_eq(
    source: &FrequencyResponse,
    target: &FrequencyResponse,
    opts: &AutoEqOptions,
) -> AutoEqResult {
    let opts = opts.sanitized();
    let prep = prepare_curves(source, target, &opts);
    let n = prep.freqs.len();
    let ppo = grid_ppo(n);
    let gmax = opts.max_gain_db as f64;
    let limit = opts.treble_boost_limit_db as f64;

    // Points inside the corrected range, their weights and the residual.
    let mut idx = Vec::new();
    for (i, &f) in prep.freqs.iter().enumerate() {
        if f >= opts.freq_min_hz * 0.999 && f <= opts.freq_max_hz * 1.001 {
            idx.push(i);
        }
    }
    let freqs: Vec<f64> = idx.iter().map(|&i| prep.freqs[i] as f64).collect();
    let raw: Vec<f64> = idx
        .iter()
        .map(|&i| (prep.target_db[i] - prep.source_db[i]) as f64)
        .collect();
    let weights: Vec<f64> = freqs
        .iter()
        .map(|&f| {
            if f > TREBLE_WEIGHT_HZ {
                TREBLE_WEIGHT as f64
            } else {
                1.0
            }
        })
        .collect();
    let wsum: f64 = weights.iter().sum::<f64>().max(1e-9);
    let r: Vec<f64> = raw
        .iter()
        .zip(&freqs)
        .map(|(&v, &f)| {
            let v = v.clamp(-gmax, gmax);
            if f >= TREBLE_CAP_HZ {
                v.min(limit)
            } else {
                v
            }
        })
        .collect();
    let treble: Vec<usize> = if limit < gmax {
        (0..freqs.len())
            .filter(|&j| freqs[j] >= TREBLE_CAP_HZ)
            .collect()
    } else {
        Vec::new()
    };
    let npts = freqs.len();
    let rms = |err: &dyn Fn(usize) -> f64| -> f32 {
        let s: f64 = (0..npts).map(|j| weights[j] * err(j).powi(2)).sum();
        (s / wsum).sqrt() as f32
    };
    let rms_before = rms(&|j| raw[j]);
    let prob = Problem {
        pts: Points::new(freqs.iter().copied()),
        sw: weights.iter().map(|w| (w / wsum).sqrt()).collect(),
        sh: (HINGE_WEIGHT / wsum).sqrt(),
        treble,
        treble_limit: limit,
        sreg: GAIN_REG.sqrt(),
        r,
        freqs,
        opts,
        ppo,
    };

    // Greedy initialisation (+ short joint optimisations), then the full optimisation.
    let mut filters: Vec<Fp> = Vec::new();
    if opts.use_shelves && opts.max_filters >= 3 {
        for (kind, f0) in [
            (FilterKind::LowShelf, LOW_SHELF_HZ),
            (FilterKind::HighShelf, HIGH_SHELF_HZ),
        ] {
            let resp: Vec<Vec<f64>> = filters.iter().map(|f| f.response(&prob.pts)).collect();
            let total = sum_responses(&resp, prob.r.len());
            let rem: Vec<f64> = prob.r.iter().zip(&total).map(|(r, t)| r - t).collect();
            let mut shelf = Fp::new(kind, f0, 0.0, SHELF_Q);
            shelf.g = prob.ls_gain(&shelf, &rem);
            prob.project(&mut shelf);
            if shelf.g.abs() >= MIN_SHELF_GAIN_DB {
                filters.push(shelf);
            }
        }
        prob.optimize(&mut filters, STEP_ITERS);
    }
    while filters.len() < opts.max_filters {
        let resp: Vec<Vec<f64>> = filters.iter().map(|f| f.response(&prob.pts)).collect();
        let total = sum_responses(&resp, prob.r.len());
        let rem: Vec<f64> = prob.r.iter().zip(&total).map(|(r, t)| r - t).collect();
        let Some(peak) = prob.next_peak(&rem) else {
            break;
        };
        filters.push(peak);
        prob.optimize(&mut filters, STEP_ITERS);
    }
    prob.optimize(&mut filters, FINAL_ITERS);
    for _ in 0..2 {
        let before = filters.len();
        filters.retain(|f| f.g.abs() >= MIN_GAIN_DB);
        if filters.len() == before {
            break;
        }
        prob.optimize(&mut filters, FINAL_ITERS / 2);
    }

    // Round to display precision (still within the bounds), sort by frequency.
    for f in &mut filters {
        let hz = f.lf.exp();
        f.lf = round_to(hz, if hz >= 100.0 { 1.0 } else { 0.1 }).ln();
        f.g = round_to(f.g, 0.1);
        if !f.is_shelf() {
            f.lq = round_to(f.lq.exp(), 0.01).max(0.01).ln();
        }
        prob.project(f);
    }
    filters.retain(|f| f.g.abs() >= MIN_GAIN_DB - 1e-9);
    filters.sort_by(|a, b| {
        a.lf.total_cmp(&b.lf)
            .then(kind_order(a.kind).cmp(&kind_order(b.kind)))
    });

    let resp: Vec<Vec<f64>> = filters.iter().map(|f| f.response(&prob.pts)).collect();
    let total = sum_responses(&resp, prob.r.len());
    let rms_after = rms(&|j| raw[j] - total[j]);

    // Preamp: the combined response's peak over the whole audible band (grid + centres).
    let all = Points::new(
        prep.freqs
            .iter()
            .map(|&f| f as f64)
            .chain(filters.iter().map(|f| f.lf.exp())),
    );
    let mut peak = 0.0f64;
    if !filters.is_empty() {
        let mut sum = vec![0.0; all.c1.len()];
        for f in &filters {
            for (s, v) in sum.iter_mut().zip(f.response(&all)) {
                *s += v;
            }
        }
        peak = sum.iter().copied().fold(0.0, f64::max);
    }
    let preamp_db = if peak > 0.0 {
        -((peak * 10.0).ceil() / 10.0)
    } else {
        0.0
    };

    AutoEqResult {
        preamp_db: preamp_db as f32,
        filters: filters.into_iter().map(Fp::to_filter).collect(),
        rms_error_before_db: rms_before,
        rms_error_after_db: rms_after,
        source_offset_db: prep.source_offset_db,
    }
}

/// Ordering of the kinds for filters at the same frequency.
fn kind_order(k: FilterKind) -> u8 {
    match k {
        FilterKind::LowShelf => 0,
        FilterKind::Peak => 1,
        FilterKind::HighShelf => 2,
        FilterKind::LowPass => 3,
        FilterKind::HighPass => 4,
    }
}

/// Combined magnitude response (dB) of `filters` at `freqs` (filters only, no preamp), with the
/// same closed form the fit uses (48 kHz). Handy for previews of a fitted result.
pub fn filters_response_db(filters: &[PeqFilter], freqs: &[f32]) -> Vec<f32> {
    let pts = Points::new(freqs.iter().map(|&f| f as f64));
    let mut sum = vec![0.0f64; freqs.len()];
    for f in filters {
        if matches!(f.kind, FilterKind::LowPass | FilterKind::HighPass) {
            // Not produced by the fit; use the engine's design for completeness.
            let c = f.coeffs(FIT_FS as u32);
            for (s, &hz) in sum.iter_mut().zip(freqs) {
                *s += c.magnitude_db(hz, FIT_FS as u32) as f64;
            }
            continue;
        }
        let fp = Fp::new(
            f.kind,
            f.freq_hz.max(1.0) as f64,
            f.gain_db as f64,
            f.q.max(1e-3) as f64,
        );
        for (s, v) in sum.iter_mut().zip(fp.response(&pts)) {
            *s += v;
        }
    }
    sum.into_iter().map(|v| v as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parser ----

    #[test]
    fn parses_rew_text_export_with_header_and_phase() {
        let text = "* Measurement data measured by IEM_Measure 0.1.0\n\
            * Measurement: Left\n\
            * Dated: 2026-11-01 12:00:00\n\
            * Smoothing: 1/12 octave\n\
            * Note: \n\
            *\n\
            * Freq(Hz) SPL(dB) Phase(degrees)\n\
            20.000 95.120 -12.5000\n\
            1000.000 100.000 3.0000\n\
            100.000 98.500 -2.2500\n\
            20000.000 80.250 170.0000\n";
        let r = parse_frequency_response(text.as_bytes()).unwrap();
        assert_eq!(
            r.freqs,
            vec![20.0, 100.0, 1000.0, 20000.0],
            "sorted by frequency"
        );
        assert_eq!(r.db, vec![95.12, 98.5, 100.0, 80.25]);
    }

    #[test]
    fn parses_autoeq_csv_european_csv_and_generic_columns() {
        let csv = "frequency,raw\n20.000,1.500\n1000.000,0.000\n20000.000,-6.250\n";
        let r = parse_frequency_response(csv.as_bytes()).unwrap();
        assert_eq!(r.freqs, vec![20.0, 1000.0, 20000.0]);
        assert_eq!(r.db, vec![1.5, 0.0, -6.25]);
        // squig.link style with more columns.
        let csv = "frequency,raw,smoothed\n20,1,2\n40,3,4\n";
        assert_eq!(
            parse_frequency_response(csv.as_bytes()).unwrap().db,
            vec![1.0, 3.0]
        );
        // European: ';' separator, ',' decimal; BOM; CRLF.
        let eu = "\u{feff}Freq;dB\r\n100,5;-3,25\r\n1000;0\r\n2000,0;1,5\r\n";
        let r = parse_frequency_response(eu.as_bytes()).unwrap();
        assert_eq!(r.freqs, vec![100.5, 1000.0, 2000.0]);
        assert_eq!(r.db, vec![-3.25, 0.0, 1.5]);
        // Calibration file: quoted header, tabs, comments; bad / non-positive rows dropped.
        let cal = "\"Sens Factor =-.9dB, SERNO: 7000001\"\n# comment\n% matlab\n// c\n\
            10\t-0.5\n0\t3\n-5\t2\nabc def\n12.5\tnan\n20\t0.25\n20\t9\n1e3 1.0e0\n";
        let r = parse_frequency_response(cal.as_bytes()).unwrap();
        assert_eq!(r.freqs, vec![10.0, 20.0, 1000.0]);
        assert_eq!(
            r.db,
            vec![-0.5, 0.25, 1.0],
            "duplicate 20 Hz keeps the first"
        );
        // A line whose third column is not a number is skipped entirely (as in IEM_Measure).
        let r = parse_frequency_response(b"100 1 x\n200 2\n300 3\n").unwrap();
        assert_eq!(r.freqs, vec![200.0, 300.0]);
    }

    #[test]
    fn parses_cp932_and_utf16_files() {
        // "* ソース測定" + "周波数 dB" header in CP932 (ソ = 83 5C, ー = 81 5B, ス = 83 58,
        // 測 = 91 AA, 定 = 92 E8, 周 = 8E FC, 波 = 94 67, 数 = 90 94).
        let mut bytes = b"* ".to_vec();
        bytes.extend_from_slice(&[0x83, 0x5C, 0x81, 0x5B, 0x83, 0x58, 0x91, 0xAA, 0x92, 0xE8]);
        bytes.extend_from_slice(b"\r\n");
        bytes.extend_from_slice(&[0x8E, 0xFC, 0x94, 0x67, 0x90, 0x94]);
        bytes.extend_from_slice(b" dB\r\n50 1.5\r\n500 -2\r\n5000 3\r\n");
        assert!(std::str::from_utf8(&bytes).is_err(), "not UTF-8");
        let r = parse_frequency_response(&bytes).unwrap();
        assert_eq!(r.freqs, vec![50.0, 500.0, 5000.0]);
        assert_eq!(r.db, vec![1.5, -2.0, 3.0]);
        let mut utf16 = vec![0xFF, 0xFE];
        for u in "frequency,raw\n20,1\n200,2\n".encode_utf16() {
            utf16.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(parse_frequency_response(&utf16).unwrap().db, vec![1.0, 2.0]);
    }

    #[test]
    fn garbage_is_rejected_with_a_kind() {
        assert_eq!(parse_frequency_response(b""), Err(CurveParseError::NoData));
        assert_eq!(
            parse_frequency_response(b"Filter 1: ON PK Fc 100 Hz Gain 1 dB Q 1\nPreamp: -3 dB\n"),
            Err(CurveParseError::NoData)
        );
        assert_eq!(
            parse_frequency_response(&[0u8, 1, 2, 255, 254, 7]),
            Err(CurveParseError::NoData)
        );
        assert_eq!(
            parse_frequency_response(b"1000 3\n"),
            Err(CurveParseError::TooFewPoints(1))
        );
        assert_eq!(
            parse_frequency_response(b"1000 3\n1000 4\n"),
            Err(CurveParseError::TooFewPoints(1))
        );
        let big = vec![b' '; CURVE_MAX_BYTES + 1];
        assert_eq!(
            parse_frequency_response(&big),
            Err(CurveParseError::TooLarge)
        );
        assert!(CurveParseError::TooFewPoints(1).to_string().contains('1'));
    }

    #[test]
    fn interpolation_is_linear_in_log_frequency_and_holds_ends() {
        let r = FrequencyResponse {
            freqs: vec![100.0, 10_000.0],
            db: vec![0.0, 10.0],
        };
        assert_eq!(r.db_at(10.0), 0.0);
        assert_eq!(r.db_at(50_000.0), 10.0);
        assert!((r.db_at(1000.0) - 5.0).abs() < 1e-4);
        let g = fit_grid();
        assert_eq!(g.first().copied(), Some(20.0));
        assert!((g.last().unwrap() - 20_000.0).abs() < 0.5);
        assert!((grid_ppo(g.len()) - 48.0).abs() < 0.2);
        let avg = FrequencyResponse::average(&r, &FrequencyResponse::flat());
        assert!((avg.db_at(1000.0) - 2.5).abs() < 0.01);
    }

    // ---- fitting ----

    /// A smooth IEM-like target (bass shelf, pinna gain, treble roll-off), in dB.
    fn target_db(f: f64) -> f64 {
        let bass = 6.0 / (1.0 + (f / 120.0).powi(2));
        let pinna = 9.0 * (-(f / 2800.0).log2().powi(2) / (2.0 * 0.7f64.powi(2))).exp();
        let rolloff = -8.0 / (1.0 + (9000.0 / f).powi(4));
        75.0 + bass + pinna + rolloff
    }

    /// A "file" curve on a 1/24-octave grid from 10 Hz to 24 kHz.
    fn curve(f: impl Fn(f64) -> f64) -> FrequencyResponse {
        let mut freqs = Vec::new();
        let mut hz = 10.0f64;
        while hz <= 24_000.0 {
            freqs.push(hz as f32);
            hz *= 2f64.powf(1.0 / 24.0);
        }
        let db = freqs.iter().map(|&x| f(x as f64) as f32).collect();
        FrequencyResponse { freqs, db }
    }

    /// Engine (f32 biquad) response of a filter set, to check the fit independently.
    fn engine_response(filters: &[PeqFilter], hz: f64) -> f64 {
        filters
            .iter()
            .map(|f| f.coeffs(48_000).magnitude_db(hz as f32, 48_000) as f64)
            .sum()
    }

    fn pk(freq_hz: f32, gain_db: f32, q: f32) -> PeqFilter {
        PeqFilter {
            kind: FilterKind::Peak,
            freq_hz,
            gain_db,
            q,
        }
    }

    fn known_filters() -> Vec<PeqFilter> {
        vec![
            PeqFilter {
                kind: FilterKind::LowShelf,
                freq_hz: 90.0,
                gain_db: 5.0,
                q: std::f32::consts::FRAC_1_SQRT_2,
            },
            pk(180.0, -2.0, 1.4),
            pk(3800.0, 4.5, 2.2),
            pk(6500.0, -5.0, 3.5),
            pk(11_000.0, -3.0, 2.0),
        ]
    }

    #[test]
    fn fit_recovers_known_filters() {
        let known = known_filters();
        let target = curve(target_db);
        // The measured source needs exactly `known` to reach the target (+ any level).
        let source = curve(|f| target_db(f) + 20.0 - engine_response(&known, f));
        let opts = AutoEqOptions {
            normalize_band: (500.0, 1200.0),
            ..Default::default()
        };
        let res = auto_eq(&source, &target, &opts);
        println!("recovered: {res:#?}");
        assert!(res.rms_error_before_db > 2.0, "{res:?}");
        assert!(res.rms_error_after_db < 0.5, "{res:?}");
        assert!((res.source_offset_db + 20.0).abs() < 0.3, "{res:?}");
        assert!(res.preamp_db <= 0.0);
        // The engine's own biquads reproduce the predicted correction.
        let check = filters_response_db(&res.filters, &[100.0, 3800.0, 6500.0]);
        for (i, hz) in [100.0, 3800.0, 6500.0].into_iter().enumerate() {
            let eng = engine_response(&res.filters, hz);
            assert!(
                (eng - check[i] as f64).abs() < 0.05,
                "{hz}: {eng} vs {}",
                check[i]
            );
        }
        // Combined response incl. preamp peaks at <= 0 dB.
        let grid = fit_grid();
        let peak = grid
            .iter()
            .map(|&f| engine_response(&res.filters, f as f64))
            .fold(f64::MIN, f64::max);
        assert!(
            peak + res.preamp_db as f64 <= 0.06,
            "peak {peak} preamp {}",
            res.preamp_db
        );
        // Sorted by frequency.
        assert!(res.filters.windows(2).all(|w| w[0].freq_hz <= w[1].freq_hz));
    }

    #[test]
    fn bounds_and_max_filters_are_respected() {
        let target = curve(target_db);
        // A bumpy measurement: lots of narrow features, a big treble dip and a bass deficit.
        let source = curve(|f| {
            target_db(f) - 10.0 / (1.0 + (f / 60.0).powi(2))
                + 4.0 * (f / 300.0).log2().sin()
                + 3.0 * ((f / 1000.0).log2() * 9.0).cos()
                - 10.0 * (-(f / 12_000.0).log2().powi(2) / 0.02).exp()
                - 14.0 * (-(f / 5_000.0).log2().powi(2) / 0.01).exp()
        });
        for use_shelves in [true, false] {
            let opts = AutoEqOptions {
                max_filters: 5,
                max_gain_db: 6.0,
                min_q: 1.0,
                max_q: 3.0,
                freq_min_hz: 50.0,
                freq_max_hz: 10_000.0,
                treble_boost_limit_db: 2.0,
                use_shelves,
                ..Default::default()
            };
            let res = auto_eq(&source, &target, &opts);
            assert!(!res.filters.is_empty() && res.filters.len() <= 5, "{res:?}");
            assert!(res.rms_error_after_db < res.rms_error_before_db, "{res:?}");
            for f in &res.filters {
                assert!(f.gain_db.abs() <= 6.0 + 1e-4, "{f:?}");
                match f.kind {
                    FilterKind::Peak => {
                        assert!((50.0..=10_000.0).contains(&f.freq_hz), "{f:?}");
                        assert!((1.0 - 1e-4..=3.0 + 1e-4).contains(&f.q), "{f:?}");
                        if f.freq_hz >= 8000.0 {
                            assert!(f.gain_db <= 2.0 + 1e-4, "{f:?}");
                        }
                    }
                    FilterKind::LowShelf => assert!(use_shelves, "{f:?}"),
                    FilterKind::HighShelf => {
                        assert!(use_shelves, "{f:?}");
                        assert!(f.gain_db <= 2.0 + 1e-4, "{f:?}");
                    }
                    _ => panic!("unexpected kind {f:?}"),
                }
            }
            // The combined boost above 8 kHz stays near the treble limit.
            let grid: Vec<f32> = fit_grid()
                .into_iter()
                .filter(|f| (8000.0..=10_000.0).contains(f))
                .collect();
            let top = filters_response_db(&res.filters, &grid)
                .into_iter()
                .fold(f32::MIN, f32::max);
            assert!(top <= 2.0 + 0.5, "treble boost {top} {res:?}");
        }
        // One filter only.
        let res = auto_eq(
            &source,
            &target,
            &AutoEqOptions {
                max_filters: 1,
                ..Default::default()
            },
        );
        assert_eq!(res.filters.len(), 1);
        // Wild options are sanitized.
        let wild = AutoEqOptions {
            max_filters: 99,
            max_gain_db: f32::NAN,
            min_q: 50.0,
            max_q: 0.0,
            freq_min_hz: 30_000.0,
            freq_max_hz: 5.0,
            treble_boost_limit_db: 99.0,
            normalize_band: (5000.0, 5000.0),
            ..Default::default()
        }
        .sanitized();
        assert_eq!(wild.max_filters, PEQ_MAX_FILTERS);
        assert_eq!(wild.max_gain_db, 12.0);
        assert!(wild.min_q <= wild.max_q && wild.max_q <= 10.0);
        assert!(wild.freq_min_hz < wild.freq_max_hz && wild.freq_max_hz <= FIT_MAX_HZ);
        assert!(wild.treble_boost_limit_db <= wild.max_gain_db);
        assert_eq!(wild.normalize_band, AutoEqOptions::default().normalize_band);
    }

    #[test]
    fn identical_curves_need_no_filters() {
        let target = curve(target_db);
        let source = curve(|f| target_db(f) - 12.0);
        let res = auto_eq(&source, &target, &AutoEqOptions::default());
        assert!(res.filters.is_empty(), "{res:?}");
        assert_eq!(res.preamp_db, 0.0);
        assert!(res.rms_error_before_db < 0.01 && res.rms_error_after_db < 0.01);
        let flat = auto_eq(
            &FrequencyResponse::flat(),
            &FrequencyResponse::flat(),
            &AutoEqOptions::default(),
        );
        assert!(flat.filters.is_empty());
    }

    #[test]
    fn fit_is_deterministic() {
        let target = curve(target_db);
        let source = curve(|f| {
            target_db(f) + 3.0 * (f / 500.0).log2().sin() - 4.0 * ((f / 3000.0).log2() * 3.0).cos()
        });
        let opts = AutoEqOptions {
            max_filters: 12,
            ..Default::default()
        };
        let a = auto_eq(&source, &target, &opts);
        let b = auto_eq(&source, &target, &opts);
        assert_eq!(a, b);
        assert!(a.rms_error_after_db < a.rms_error_before_db * 0.5, "{a:?}");
    }

    #[test]
    fn flat_target_and_preparation() {
        let source = curve(|f| 100.0 + 1.0 * (f / 1000.0).log2());
        let prep = prepare_curves(
            &source,
            &FrequencyResponse::flat(),
            &AutoEqOptions::default(),
        );
        assert_eq!(prep.freqs.len(), prep.source_db.len());
        // Level-matched in 250–2500 Hz: the mean there is ~0.
        let band: Vec<f32> = prep
            .freqs
            .iter()
            .zip(&prep.source_db)
            .filter(|(f, _)| (250.0..=2500.0).contains(*f))
            .map(|(_, d)| *d)
            .collect();
        let mean = band.iter().sum::<f32>() / band.len() as f32;
        assert!(mean.abs() < 0.01, "{mean}");
        assert!(
            (-100.0..-99.0).contains(&prep.source_offset_db),
            "{}",
            prep.source_offset_db
        );
        let res = auto_eq(
            &source,
            &FrequencyResponse::flat(),
            &AutoEqOptions::default(),
        );
        assert!(res.rms_error_after_db < 1.0, "{res:?}");
    }

    /// `cargo test --release -p lrc_player_adapters_audio auto_eq_timing -- --ignored --nocapture`
    #[test]
    #[ignore = "timing; run in release"]
    fn auto_eq_timing() {
        let target = curve(target_db);
        let source = curve(|f| {
            target_db(f) + 4.0 * (f / 300.0).log2().sin() + 3.0 * ((f / 1000.0).log2() * 9.0).cos()
                - 8.0 * (-(f / 6_000.0).log2().powi(2) / 0.01).exp()
        });
        let opts = AutoEqOptions {
            max_filters: PEQ_MAX_FILTERS,
            ..Default::default()
        };
        let t = std::time::Instant::now();
        let runs = 5;
        let mut res = None;
        for _ in 0..runs {
            res = Some(auto_eq(&source, &target, &opts));
        }
        let per = t.elapsed() / runs;
        let res = res.unwrap();
        println!(
            "auto_eq with {} filters: {per:?} per fit, RMS {:.2} -> {:.2} dB",
            res.filters.len(),
            res.rms_error_before_db,
            res.rms_error_after_db
        );
        assert!(per < std::time::Duration::from_secs(1));
    }

    /// Generous smoke test that also runs in normal test builds.
    #[test]
    fn twenty_filters_finish_quickly() {
        let target = curve(target_db);
        let source = curve(|f| target_db(f) + 5.0 * ((f / 700.0).log2() * 4.0).sin());
        let t = std::time::Instant::now();
        let res = auto_eq(
            &source,
            &target,
            &AutoEqOptions {
                max_filters: PEQ_MAX_FILTERS,
                ..Default::default()
            },
        );
        assert!(res.filters.len() <= PEQ_MAX_FILTERS);
        assert!(
            t.elapsed() < std::time::Duration::from_secs(20),
            "{:?}",
            t.elapsed()
        );
    }
}
