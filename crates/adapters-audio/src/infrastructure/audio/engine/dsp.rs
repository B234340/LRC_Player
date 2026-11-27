//! Small realtime-safe DSP building blocks: RBJ-cookbook biquads, the graphic EQ, the filter
//! chain run by the renderer (graphic EQ -> parametric EQ -> crossfeed -> auto headroom), the
//! output limiter and helpers for dB/linear conversion. Everything here is
//! `Copy`/allocation-free so it can be handed to the audio callback through a lock-free queue.

use super::crossfeed::{CrossfeedCoeffs, CrossfeedState};
use super::peq::{FilterKind, ParametricEq, PeqCoeffs, PeqSide, PeqState};

/// Number of graphic-EQ bands.
pub const EQ_BANDS: usize = 10;

/// Centre frequencies (Hz) of the graphic-EQ bands.
pub const EQ_FREQUENCIES_HZ: [f32; EQ_BANDS] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// Gain range of every band and of the preamp (dB).
pub const EQ_MAX_GAIN_DB: f32 = 12.0;

/// Q of each peaking band. ~1.41 gives one-octave bandwidth, matching octave-spaced bands.
const EQ_Q: f64 = 1.41;

/// Tiny offset added to the biquad input so filter states never decay into denormals
/// (denormal arithmetic can be 10-100x slower on x86 and cause callback overruns).
pub(crate) const ANTI_DENORMAL: f32 = 1.0e-20;

/// Maximum number of output channels the EQ processes (extra channels pass through unfiltered).
pub(crate) const EQ_MAX_CHANNELS: usize = 8;

/// User-facing equalizer settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqSettings {
    pub enabled: bool,
    /// Preamp in dB (clamped to ±12). Use a negative preamp to leave headroom for boosts.
    pub preamp_db: f32,
    /// Gains for [`EQ_FREQUENCIES_HZ`] in dB (each clamped to ±12).
    pub bands_db: [f32; EQ_BANDS],
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            preamp_db: 0.0,
            bands_db: [0.0; EQ_BANDS],
        }
    }
}

pub(crate) fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Normalised biquad coefficients (a0 == 1), transposed direct form II.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadCoeffs {
    pub(crate) const IDENTITY: BiquadCoeffs = BiquadCoeffs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// RBJ cookbook peaking EQ with the graphic EQ's Q. Returns identity for 0 dB or for a
    /// centre frequency that cannot be represented at this sample rate.
    pub(crate) fn peaking(sample_rate: u32, freq_hz: f32, gain_db: f32) -> Self {
        Self::design(FilterKind::Peak, sample_rate, freq_hz, gain_db, EQ_Q as f32)
    }

    /// RBJ cookbook biquad (computed in f64). Peaks and shelves at ~0 dB are identity; centre
    /// frequencies are kept below Nyquist (0.45 fs), except that a low-pass or high-shelf at or
    /// above Nyquist is identity (it would not act on any representable frequency).
    pub(crate) fn design(
        kind: FilterKind,
        sample_rate: u32,
        freq_hz: f32,
        gain_db: f32,
        q: f32,
    ) -> Self {
        if sample_rate == 0 || !freq_hz.is_finite() || !gain_db.is_finite() || !q.is_finite() {
            return Self::IDENTITY;
        }
        let has_gain = matches!(
            kind,
            FilterKind::Peak | FilterKind::LowShelf | FilterKind::HighShelf
        );
        if has_gain && gain_db.abs() < 1.0e-3 {
            return Self::IDENTITY;
        }
        let fs = sample_rate as f64;
        if matches!(kind, FilterKind::LowPass | FilterKind::HighShelf) && freq_hz as f64 >= fs * 0.5
        {
            return Self::IDENTITY;
        }
        // Keep the centre safely below Nyquist (e.g. the 16 kHz band on a 32 kHz device).
        let f0 = (freq_hz as f64).clamp(1.0, fs * 0.45);
        let q = (q as f64).max(1.0e-3);
        let a = 10f64.powf(gain_db as f64 / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterKind::Peak => (
                1.0 + alpha * a,
                -2.0 * cos_w0,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos_w0,
                1.0 - alpha / a,
            ),
            FilterKind::LowShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos_w0 + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0),
                    a * ((a + 1.0) - (a - 1.0) * cos_w0 - k),
                    (a + 1.0) + (a - 1.0) * cos_w0 + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0),
                    (a + 1.0) + (a - 1.0) * cos_w0 - k,
                )
            }
            FilterKind::HighShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos_w0 + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0),
                    a * ((a + 1.0) + (a - 1.0) * cos_w0 - k),
                    (a + 1.0) - (a - 1.0) * cos_w0 + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos_w0),
                    (a + 1.0) - (a - 1.0) * cos_w0 - k,
                )
            }
            FilterKind::LowPass => (
                (1.0 - cos_w0) / 2.0,
                1.0 - cos_w0,
                (1.0 - cos_w0) / 2.0,
                1.0 + alpha,
                -2.0 * cos_w0,
                1.0 - alpha,
            ),
            FilterKind::HighPass => (
                (1.0 + cos_w0) / 2.0,
                -(1.0 + cos_w0),
                (1.0 + cos_w0) / 2.0,
                1.0 + alpha,
                -2.0 * cos_w0,
                1.0 - alpha,
            ),
        };
        Self {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
        }
    }

    /// One transposed-direct-form-II step with state `s`.
    #[inline(always)]
    pub(crate) fn tick(&self, s: &mut [f32; 2], x: f32) -> f32 {
        let y = self.b0 * x + s[0];
        s[0] = self.b1 * x - self.a1 * y + s[1];
        s[1] = self.b2 * x - self.a2 * y;
        y
    }

    /// Magnitude response (dB) at `freq_hz` (evaluated from the f32 coefficients actually used).
    pub(crate) fn magnitude_db(&self, freq_hz: f32, sample_rate: u32) -> f32 {
        let w = 2.0 * std::f64::consts::PI * freq_hz as f64 / sample_rate.max(1) as f64;
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let (b0, b1, b2) = (self.b0 as f64, self.b1 as f64, self.b2 as f64);
        let (a1, a2) = (self.a1 as f64, self.a2 as f64);
        // H(e^jw) = (b0 + b1 e^-jw + b2 e^-2jw) / (1 + a1 e^-jw + a2 e^-2jw)
        let nr = b0 + b1 * c1 + b2 * c2;
        let ni = -(b1 * s1 + b2 * s2);
        let dr = 1.0 + a1 * c1 + a2 * c2;
        let di = -(a1 * s1 + a2 * s2);
        let num = nr * nr + ni * ni;
        let den = (dr * dr + di * di).max(1.0e-300);
        (10.0 * (num / den).max(1.0e-300).log10()) as f32
    }
}

/// Precomputed EQ (computed off the realtime thread, sent by value).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EqCoeffs {
    /// False => the EQ stage is bypassed entirely (bit-exact).
    pub(crate) active: bool,
    /// Linear preamp gain.
    pub(crate) preamp: f32,
    pub(crate) bands: [BiquadCoeffs; EQ_BANDS],
    /// Bit i set => band i is non-identity and must be processed.
    pub(crate) band_mask: u16,
}

impl EqCoeffs {
    pub(crate) const BYPASS: EqCoeffs = EqCoeffs {
        active: false,
        preamp: 1.0,
        bands: [BiquadCoeffs::IDENTITY; EQ_BANDS],
        band_mask: 0,
    };

    pub(crate) fn from_settings(settings: &EqSettings, sample_rate: u32) -> Self {
        if !settings.enabled {
            return Self::BYPASS;
        }
        let mut bands = [BiquadCoeffs::IDENTITY; EQ_BANDS];
        let mut mask = 0u16;
        for (i, band) in bands.iter_mut().enumerate() {
            let g = sanitize_db(settings.bands_db[i]);
            *band = BiquadCoeffs::peaking(sample_rate, EQ_FREQUENCIES_HZ[i], g);
            if *band != BiquadCoeffs::IDENTITY {
                mask |= 1 << i;
            }
        }
        let preamp_db = sanitize_db(settings.preamp_db);
        let preamp = if preamp_db.abs() < 1.0e-3 {
            1.0
        } else {
            db_to_linear(preamp_db)
        };
        Self {
            active: mask != 0 || preamp != 1.0,
            preamp,
            bands,
            band_mask: mask,
        }
    }
}

fn sanitize_db(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB)
    } else {
        0.0
    }
}

/// Per-channel filter state for the EQ cascade (fixed size, no allocation; `Copy` so the
/// renderer can snapshot it for crossfades without allocating).
#[derive(Clone, Copy)]
pub(crate) struct EqState {
    coeffs: EqCoeffs,
    /// [channel][band] = (s1, s2)
    state: [[[f32; 2]; EQ_BANDS]; EQ_MAX_CHANNELS],
}

impl EqState {
    pub(crate) fn new() -> Self {
        Self {
            coeffs: EqCoeffs::BYPASS,
            state: [[[0.0; 2]; EQ_BANDS]; EQ_MAX_CHANNELS],
        }
    }

    pub(crate) fn set(&mut self, coeffs: EqCoeffs) {
        if !self.coeffs.active && coeffs.active {
            self.reset();
        }
        self.coeffs = coeffs;
    }

    pub(crate) fn reset(&mut self) {
        self.state = [[[0.0; 2]; EQ_BANDS]; EQ_MAX_CHANNELS];
    }

    #[cfg(test)]
    pub(crate) fn active(&self) -> bool {
        self.coeffs.active
    }

    /// Filters one sample of channel `ch` (preamp NOT applied here).
    #[inline]
    pub(crate) fn process(&mut self, ch: usize, x: f32) -> f32 {
        if ch >= EQ_MAX_CHANNELS || self.coeffs.band_mask == 0 {
            return x;
        }
        let mut y = x + ANTI_DENORMAL;
        let states = &mut self.state[ch];
        let mut mask = self.coeffs.band_mask;
        while mask != 0 {
            let i = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            y = self.coeffs.bands[i].tick(&mut states[i], y);
        }
        y
    }
}

/// Soft-knee limiter threshold used when the EQ (which can boost) is active.
const SOFT_KNEE: f32 = 0.9;

/// Smooth saturation above [`SOFT_KNEE`]; identity below it. Output is bounded to ±1.
#[inline]
pub(crate) fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= SOFT_KNEE {
        x
    } else {
        let room = 1.0 - SOFT_KNEE;
        let y = SOFT_KNEE + room * ((a - SOFT_KNEE) / room).tanh();
        y.copysign(x)
    }
}

/// Hard clamp to the legal range (identity inside ±1, so pass-through stays bit-exact).
#[inline]
pub(crate) fn hard_limit(x: f32) -> f32 {
    x.clamp(-1.0, 1.0)
}

/// Soft-knee threshold: samples whose magnitude exceeds it were changed by the limiter.
#[inline]
pub(crate) fn soft_limit_engaged(x: f32) -> bool {
    x.abs() > SOFT_KNEE
}

// ----- filter chain ---------------------------------------------------------------------------

/// Everything the renderer's filter section needs (computed off the realtime thread):
/// graphic EQ -> parametric EQ -> crossfeed, then the auto-headroom gain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ChainCoeffs {
    pub(crate) eq: EqCoeffs,
    pub(crate) peq: PeqCoeffs,
    pub(crate) xfeed: CrossfeedCoeffs,
    /// Linear auto-headroom gain (<= 1; exactly 1.0 when off or not needed).
    pub(crate) headroom: f32,
}

impl ChainCoeffs {
    pub(crate) const BYPASS: ChainCoeffs = ChainCoeffs {
        eq: EqCoeffs::BYPASS,
        peq: PeqCoeffs::BYPASS,
        xfeed: CrossfeedCoeffs::BYPASS,
        headroom: 1.0,
    };

    /// False => the whole section is an exact bypass.
    pub(crate) fn active(&self) -> bool {
        self.eq.active || self.peq.active || self.xfeed.active || self.headroom != 1.0
    }
}

/// Filter-chain state: coefficients plus every filter's memory (fixed size, `Copy` so the
/// renderer can snapshot it for crossfades without allocating).
#[derive(Clone, Copy)]
pub(crate) struct ChainState {
    c: ChainCoeffs,
    active: bool,
    /// Product of both preamps and the headroom gain, for the left (index 0, also mono and
    /// channels >= 2) and the right channel (index 1); they differ only with separate PEQ
    /// channels.
    gain: [f32; 2],
    eq: EqState,
    peq: PeqState,
    xf: CrossfeedState,
}

impl ChainState {
    pub(crate) fn new() -> Self {
        Self {
            c: ChainCoeffs::BYPASS,
            active: false,
            gain: [1.0; 2],
            eq: EqState::new(),
            peq: PeqState::new(),
            xf: CrossfeedState::new(),
        }
    }

    /// Switches coefficients; filters that were idle start from a clean state.
    pub(crate) fn set(&mut self, c: ChainCoeffs) {
        self.eq.set(c.eq);
        if !self.c.peq.has_filters() && c.peq.has_filters() {
            self.peq.reset();
        }
        if !self.c.xfeed.active && c.xfeed.active {
            self.xf.reset();
        }
        self.c = c;
        self.active = c.active();
        self.gain = [0, 1].map(|i| c.eq.preamp * c.peq.sets[i].preamp * c.headroom);
    }

    #[inline]
    pub(crate) fn coeffs(&self) -> &ChainCoeffs {
        &self.c
    }

    #[inline]
    pub(crate) fn active(&self) -> bool {
        self.active
    }

    /// Processes one interleaved frame in place.
    #[inline]
    pub(crate) fn process_frame(&mut self, frame: &mut [f32]) {
        let eq_on = self.c.eq.band_mask != 0;
        let peq_on = self.c.peq.has_filters();
        for (ch, s) in frame.iter_mut().enumerate() {
            let g = self.gain[usize::from(ch == 1)];
            let mut y = *s;
            if eq_on {
                y = self.eq.process(ch, y);
            }
            if peq_on {
                y = self.peq.process(&self.c.peq, ch, y);
            }
            *s = y * g;
        }
        if self.c.xfeed.active {
            if let [l, r] = frame {
                self.xf.process(&self.c.xfeed, l, r);
            }
        }
    }
}

// ----- auto headroom --------------------------------------------------------------------------

/// Points of the log-spaced grid the combined EQ response is evaluated on.
const HEADROOM_GRID: usize = 512;

/// Maximum of the combined magnitude response (dB) of the graphic EQ and the parametric EQ
/// of `side` (filters only, preamps excluded) between 10 Hz and 0.49 fs.
///
/// The cascade's response is the sum (in dB) of every biquad's exact response, evaluated on a
/// 512-point log grid plus every filter's own centre frequency (where peaks and shelf corners
/// sit), so narrow high-Q peaks are not missed between grid points. Interacting filters
/// (overlapping boosts, a cut inside a boost) are therefore accounted for exactly, unlike the
/// cruder "sum of all positive gains" bound which over-attenuates.
pub(crate) fn max_boost_db(
    eq: &EqSettings,
    peq: &ParametricEq,
    side: PeqSide,
    sample_rate: u32,
) -> f32 {
    let eqc = EqCoeffs::from_settings(eq, sample_rate);
    let peqc = PeqCoeffs::from_settings(peq, sample_rate);
    let set = peqc.side(side);
    let filters: Vec<BiquadCoeffs> = eqc
        .bands
        .iter()
        .enumerate()
        .filter(|(i, _)| eqc.band_mask & (1 << i) != 0)
        .map(|(_, b)| *b)
        .chain(set.used().iter().copied())
        .collect();
    if filters.is_empty() || sample_rate == 0 {
        return 0.0;
    }
    let hi = sample_rate as f32 * 0.49;
    let lo = 10.0f32.min(hi);
    let mut freqs: Vec<f32> = (0..HEADROOM_GRID)
        .map(|i| lo * (hi / lo).powf(i as f32 / (HEADROOM_GRID - 1) as f32))
        .collect();
    if eqc.active {
        freqs.extend(EQ_FREQUENCIES_HZ.iter().copied());
    }
    if peqc.active {
        freqs.extend(
            peq.used_filters(side)
                .iter()
                .map(|f| f.sanitized().freq_hz),
        );
    }
    freqs
        .into_iter()
        .filter(|f| *f > 0.0 && *f <= hi)
        .map(|f| {
            filters
                .iter()
                .map(|b| b.magnitude_db(f, sample_rate))
                .sum::<f32>()
        })
        .fold(f32::MIN, f32::max)
}

/// Auto-headroom gain (dB, <= 0): lowers the level by the largest boost of the active EQs net
/// of their preamps (`max_boost + eq_preamp + peq_preamp`), so EQ boosts do not drive the
/// limiter. 0 when that sum is not positive (the preamps already leave enough room). With
/// separate PEQ channels this is the worst case over both channels (one gain for all).
pub(crate) fn auto_headroom_db(eq: &EqSettings, peq: &ParametricEq, sample_rate: u32) -> f32 {
    let eqc = EqCoeffs::from_settings(eq, sample_rate);
    let peqc = PeqCoeffs::from_settings(peq, sample_rate);
    if !eqc.active && !peqc.active {
        return 0.0;
    }
    let pre = |active: bool, lin: f32| if active { 20.0 * lin.log10() } else { 0.0 };
    let need = peq
        .sides()
        .iter()
        .map(|&side| {
            max_boost_db(eq, peq, side, sample_rate)
                + pre(eqc.active, eqc.preamp)
                + pre(peqc.active, peqc.side(side).preamp)
        })
        .fold(f32::MIN, f32::max);
    if need > 0.005 {
        -need
    } else {
        0.0
    }
}

// ----- response preview (UI) ----------------------------------------------------------------

/// Combined magnitude response (dB) of the graphic EQ and the parametric EQ of `side` at each
/// of `freqs`, including both preamps (each only when its EQ is active), evaluated from the
/// exact coefficients the renderer uses at `sample_rate`. With linked PEQ channels both sides
/// give the same result. Auto headroom is not included (see [`auto_headroom_preview_db`]). A
/// disabled/flat EQ contributes 0 dB. Frequencies at or above Nyquist are evaluated as-is (the
/// biquad response there is still defined).
pub fn combined_response_db(
    eq: &EqSettings,
    peq: &ParametricEq,
    side: PeqSide,
    sample_rate: u32,
    freqs: &[f32],
) -> Vec<f32> {
    let sample_rate = sample_rate.max(1);
    let eqc = EqCoeffs::from_settings(eq, sample_rate);
    let peqc = PeqCoeffs::from_settings(peq, sample_rate);
    let set = peqc.side(side);
    let filters: Vec<BiquadCoeffs> = eqc
        .bands
        .iter()
        .enumerate()
        .filter(|(i, _)| eqc.active && eqc.band_mask & (1 << i) != 0)
        .map(|(_, b)| *b)
        .chain(set.used().iter().copied())
        .collect();
    let pre = |active: bool, lin: f32| if active { 20.0 * lin.log10() } else { 0.0 };
    let preamps = pre(eqc.active, eqc.preamp) + pre(peqc.active, set.preamp);
    freqs
        .iter()
        .map(|&f| {
            preamps
                + filters
                    .iter()
                    .map(|b| b.magnitude_db(f, sample_rate))
                    .sum::<f32>()
        })
        .collect()
}

/// The gain (dB, <= 0) automatic headroom would apply for these EQ settings at `sample_rate`
/// (what the engine applies when auto headroom is on).
pub fn auto_headroom_preview_db(eq: &EqSettings, peq: &ParametricEq, sample_rate: u32) -> f32 {
    auto_headroom_db(eq, peq, sample_rate.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_gain_eq_is_bypass() {
        let s = EqSettings {
            enabled: true,
            ..Default::default()
        };
        let c = EqCoeffs::from_settings(&s, 48_000);
        assert!(!c.active);
        assert_eq!(c.band_mask, 0);
    }

    #[test]
    fn peaking_filter_has_requested_gain_at_centre() {
        // Measure the steady-state gain of a 1 kHz sine through a +6 dB band at 1 kHz.
        let fs = 48_000;
        let mut settings = EqSettings {
            enabled: true,
            ..Default::default()
        };
        settings.bands_db[5] = 6.0;
        let mut st = EqState::new();
        st.set(EqCoeffs::from_settings(&settings, fs));
        let mut peak = 0.0f32;
        for n in 0..fs as usize {
            let x = 0.25 * (2.0 * std::f32::consts::PI * 1000.0 * n as f32 / fs as f32).sin();
            let y = st.process(0, x);
            if n > fs as usize / 2 {
                peak = peak.max(y.abs());
            }
        }
        let gain_db = 20.0 * (peak / 0.25).log10();
        assert!((gain_db - 6.0).abs() < 0.2, "gain {gain_db}");
    }

    #[test]
    fn combined_response_sums_eq_peq_and_preamps() {
        use super::super::peq::PeqFilter;
        let fs = 48_000;
        let freqs = [20.0, 100.0, 1000.0, 5000.0, 20_000.0];
        // Everything off: flat.
        let flat = combined_response_db(
            &EqSettings::default(),
            &ParametricEq::default(),
            PeqSide::Left,
            fs,
            &freqs,
        );
        assert_eq!(flat, vec![0.0; freqs.len()]);
        // Graphic EQ: +6 dB at 1 kHz, preamp -2 dB.
        let mut eq = EqSettings {
            enabled: true,
            preamp_db: -2.0,
            ..Default::default()
        };
        eq.bands_db[5] = 6.0;
        let r = combined_response_db(&eq, &ParametricEq::default(), PeqSide::Left, fs, &freqs);
        assert!((r[2] - 4.0).abs() < 0.1, "{r:?}");
        assert!((r[0] + 2.0).abs() < 0.1, "{r:?}");
        // Plus a PEQ peak of -4 dB at 1 kHz with a -1 dB preamp, and a 5 kHz high-pass.
        let peq = ParametricEq {
            enabled: true,
            preamp_db: -1.0,
            filters: vec![
                PeqFilter {
                    kind: FilterKind::Peak,
                    freq_hz: 1000.0,
                    gain_db: -4.0,
                    q: 1.0,
                },
                PeqFilter {
                    kind: FilterKind::HighPass,
                    freq_hz: 5000.0,
                    gain_db: 0.0,
                    q: std::f32::consts::FRAC_1_SQRT_2,
                },
            ],
            ..Default::default()
        };
        let r = combined_response_db(&eq, &peq, PeqSide::Left, fs, &freqs);
        // 1 kHz: +6 - 2 - 4 - 1 = -1 dB, minus the high-pass skirt (~-28 dB two octaves down).
        let hp_1k = r[2] - (-1.0);
        assert!(hp_1k < -20.0 && hp_1k > -35.0, "{r:?}");
        // At the cutoff a Butterworth high-pass is -3 dB.
        let hp_only = ParametricEq {
            enabled: true,
            preamp_db: -1.0,
            filters: vec![peq.filters[1]],
            ..Default::default()
        };
        let at_fc =
            combined_response_db(&EqSettings::default(), &hp_only, PeqSide::Left, fs, &[5000.0])
                [0];
        assert!((at_fc - (-1.0 - 3.01)).abs() < 0.05, "{at_fc}");
        // A disabled PEQ contributes nothing, preamp included.
        let off = ParametricEq {
            enabled: false,
            ..peq.clone()
        };
        let r2 = combined_response_db(&eq, &off, PeqSide::Left, fs, &freqs);
        assert!((r2[2] - 4.0).abs() < 0.1, "{r2:?}");
        // Matches the per-biquad sum used by auto headroom.
        let boost = max_boost_db(&eq, &ParametricEq::default(), PeqSide::Left, fs);
        assert!((boost - 6.0).abs() < 0.1);
        assert!((auto_headroom_preview_db(&eq, &ParametricEq::default(), fs) + 4.0).abs() < 0.1);
    }

    #[test]
    fn separate_peq_channels_in_response_headroom_and_chain() {
        use super::super::peq::{PeqChannel, PeqChannelMode, PeqFilter};
        let fs = 48_000;
        let pk = |freq_hz, gain_db| PeqFilter {
            kind: FilterKind::Peak,
            freq_hz,
            gain_db,
            q: 1.0,
        };
        // L: +3 dB at 1 kHz, R: +8 dB at 4 kHz with a -1 dB preamp.
        let mut peq = ParametricEq {
            enabled: true,
            filters: vec![pk(1000.0, 3.0)],
            channel_mode: PeqChannelMode::Separate,
            right: PeqChannel {
                preamp_db: -1.0,
                filters: vec![pk(4000.0, 8.0)],
            },
            ..Default::default()
        };
        let eq = EqSettings::default();
        let freqs = [1000.0, 4000.0];
        let l = combined_response_db(&eq, &peq, PeqSide::Left, fs, &freqs);
        let r = combined_response_db(&eq, &peq, PeqSide::Right, fs, &freqs);
        assert!((l[0] - 3.0).abs() < 0.05 && l[1].abs() < 1.5, "{l:?}");
        assert!((r[1] - 7.0).abs() < 0.05 && r[0] < 0.5, "{r:?}");
        // Headroom: the worse channel (R: +8 - 1 = +7 dB).
        assert!((auto_headroom_db(&eq, &peq, fs) + 7.0).abs() < 0.05);
        // Linked: only the main (L) set counts.
        peq.channel_mode = PeqChannelMode::Linked;
        assert!((auto_headroom_db(&eq, &peq, fs) + 3.0).abs() < 0.05);
        let r_linked = combined_response_db(&eq, &peq, PeqSide::Right, fs, &freqs);
        assert_eq!(r_linked, combined_response_db(&eq, &peq, PeqSide::Left, fs, &freqs));
        // The chain applies each channel's preamp to its own channel.
        peq.channel_mode = PeqChannelMode::Separate;
        peq.filters.clear();
        peq.right.filters.clear();
        peq.preamp_db = -6.0;
        let mut st = ChainState::new();
        st.set(ChainCoeffs {
            peq: PeqCoeffs::from_settings(&peq, fs),
            ..ChainCoeffs::BYPASS
        });
        assert!(st.active());
        let mut frame = [0.5f32, 0.5, 0.5];
        st.process_frame(&mut frame);
        assert!((frame[0] - 0.5 * db_to_linear(-6.0)).abs() < 1e-6);
        assert!((frame[1] - 0.5 * db_to_linear(-1.0)).abs() < 1e-6);
        assert!((frame[2] - frame[0]).abs() < 1e-9, "channel 2 uses the left set");
    }

    #[test]
    fn soft_limit_is_bounded_and_continuous() {
        assert_eq!(soft_limit(0.5), 0.5);
        assert!(soft_limit(10.0) <= 1.0);
        assert!(soft_limit(-10.0) >= -1.0);
        let below = soft_limit(SOFT_KNEE - 1e-6);
        let above = soft_limit(SOFT_KNEE + 1e-6);
        assert!((above - below).abs() < 1e-4);
    }

    /// Throughput of the full chain (10 bands + 20 PEQ filters + crossfeed, stereo).
    /// `cargo test --release -p lrc_player_adapters_audio measure_chain_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn measure_chain_cost() {
        use super::super::crossfeed::{Crossfeed, CrossfeedLevel};
        use super::super::peq::PeqFilter;
        let fs = 48_000;
        let mut eq = EqSettings {
            enabled: true,
            ..Default::default()
        };
        eq.bands_db = [3.0; EQ_BANDS];
        let peq = ParametricEq {
            enabled: true,
            preamp_db: -3.0,
            filters: (0..20)
                .map(|i| PeqFilter {
                    kind: FilterKind::Peak,
                    freq_hz: 30.0 * 1.4f32.powi(i),
                    gain_db: 2.0,
                    q: 1.0,
                })
                .collect(),
            ..Default::default()
        };
        let xf = Crossfeed {
            enabled: true,
            level: CrossfeedLevel::Medium,
        };
        let t = std::time::Instant::now();
        let hr = auto_headroom_db(&eq, &peq, fs);
        let t_headroom = t.elapsed();
        let mut st = ChainState::new();
        st.set(ChainCoeffs {
            eq: EqCoeffs::from_settings(&eq, fs),
            peq: PeqCoeffs::from_settings(&peq, fs),
            xfeed: CrossfeedCoeffs::new(&xf, fs, 2),
            headroom: db_to_linear(hr),
        });
        let frames = fs as usize * 10;
        let mut buf: Vec<f32> = (0..frames * 2)
            .map(|i| ((i as f32) * 0.001).sin() * 0.1)
            .collect();
        let t = std::time::Instant::now();
        for f in buf.chunks_exact_mut(2) {
            st.process_frame(f);
        }
        let el = t.elapsed();
        println!(
            "chain: {:.1} ns/frame, {:.3}% of one core for 48 kHz stereo; headroom calc {:?} ({hr:.2} dB); {}",
            el.as_nanos() as f64 / frames as f64,
            el.as_secs_f64() / 10.0 * 100.0,
            t_headroom,
            buf[123]
        );
    }
}
