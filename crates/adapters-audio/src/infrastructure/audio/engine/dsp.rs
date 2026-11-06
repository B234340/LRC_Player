//! Small realtime-safe DSP building blocks: peaking-EQ biquads, the output limiter and
//! helpers for dB/linear conversion. Everything here is `Copy`/allocation-free so it can be
//! handed to the audio callback through a lock-free queue.

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
const ANTI_DENORMAL: f32 = 1.0e-20;

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

    /// RBJ cookbook peaking EQ. Returns identity for 0 dB or for a centre frequency that
    /// cannot be represented at this sample rate.
    pub(crate) fn peaking(sample_rate: u32, freq_hz: f32, gain_db: f32) -> Self {
        if gain_db.abs() < 1.0e-3 || sample_rate == 0 {
            return Self::IDENTITY;
        }
        let fs = sample_rate as f64;
        // Keep the centre safely below Nyquist (e.g. the 16 kHz band on a 32 kHz device).
        let f0 = (freq_hz as f64).min(fs * 0.45);
        let a = 10f64.powf(gain_db as f64 / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * EQ_Q);
        let a0 = 1.0 + alpha / a;
        Self {
            b0: ((1.0 + alpha * a) / a0) as f32,
            b1: ((-2.0 * cos_w0) / a0) as f32,
            b2: ((1.0 - alpha * a) / a0) as f32,
            a1: ((-2.0 * cos_w0) / a0) as f32,
            a2: ((1.0 - alpha / a) / a0) as f32,
        }
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

/// Per-channel filter state for the EQ cascade (fixed size, no allocation).
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

    #[inline]
    pub(crate) fn active(&self) -> bool {
        self.coeffs.active
    }

    #[inline]
    pub(crate) fn preamp(&self) -> f32 {
        self.coeffs.preamp
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
            let c = &self.coeffs.bands[i];
            let s = &mut states[i];
            let out = c.b0 * y + s[0];
            s[0] = c.b1 * y - c.a1 * out + s[1];
            s[1] = c.b2 * y - c.a2 * out;
            y = out;
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
    fn soft_limit_is_bounded_and_continuous() {
        assert_eq!(soft_limit(0.5), 0.5);
        assert!(soft_limit(10.0) <= 1.0);
        assert!(soft_limit(-10.0) >= -1.0);
        let below = soft_limit(SOFT_KNEE - 1e-6);
        let above = soft_limit(SOFT_KNEE + 1e-6);
        assert!((above - below).abs() < 1e-4);
    }
}
