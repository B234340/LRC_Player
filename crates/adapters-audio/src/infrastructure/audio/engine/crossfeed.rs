//! Headphone crossfeed (Bauer stereophonic-to-binaural, bs2b-style).
//!
//! Each ear receives the opposite channel low-passed (first-order, cutoff `fc`; its group delay
//! of ~1/(2*pi*fc), ~230 us at 700 Hz, is the interaural delay) while its own channel gets the
//! complementary high-shelf. The implementation uses the complementary form
//!
//! ```text
//! out_L = L + LP(R - L)        out_R = R - LP(R - L)
//! ```
//!
//! i.e. direct path `1 - LP` (a high shelf: `1 - G` at low frequencies, 1 at high
//! frequencies) plus crossfeed `LP` (`G` at low frequencies). With
//! `G = 1 / (1 + 10^(feed_db / 20))` the low-frequency direct/crossfeed ratio is exactly the
//! bs2b feed level, and the direct/crossfed levels at DC equal libbs2b's. Unlike libbs2b
//! (whose centre image drops ~1.8 dB at high frequencies), a centre (L == R) signal passes
//! exactly at unity at every frequency, so the result is fully mono compatible. One one-pole
//! filter per stereo frame: negligible CPU.

/// Crossfeed strength presets (libbs2b's presets), named by audible strength: the feed level is
/// how far the crossfed signal sits below the direct one, so a *smaller* dB value is a
/// *stronger* crossfeed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CrossfeedLevel {
    /// 650 Hz, 9.5 dB (libbs2b "Jan Meier"): the subtlest.
    Weak,
    /// 700 Hz, 6.0 dB (libbs2b "Chu Moy").
    #[default]
    Medium,
    /// 700 Hz, 4.5 dB (libbs2b "default"): the strongest.
    Strong,
}

impl CrossfeedLevel {
    /// (low-pass cutoff in Hz, feed level in dB).
    pub fn params(self) -> (f32, f32) {
        match self {
            CrossfeedLevel::Strong => (700.0, 4.5),
            CrossfeedLevel::Medium => (700.0, 6.0),
            CrossfeedLevel::Weak => (650.0, 9.5),
        }
    }
}

/// User-facing crossfeed settings. Applies only to stereo output (bypassed otherwise).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Crossfeed {
    pub enabled: bool,
    pub level: CrossfeedLevel,
}

/// Precomputed crossfeed filter (computed off the realtime thread).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CrossfeedCoeffs {
    pub(crate) active: bool,
    /// One-pole low-pass: y = a * x + b * y[-1].
    a: f32,
    b: f32,
}

impl CrossfeedCoeffs {
    pub(crate) const BYPASS: CrossfeedCoeffs = CrossfeedCoeffs {
        active: false,
        a: 0.0,
        b: 0.0,
    };

    pub(crate) fn new(settings: &Crossfeed, sample_rate: u32, channels: usize) -> Self {
        if !settings.enabled || channels != 2 || sample_rate == 0 {
            return Self::BYPASS;
        }
        let (fc, feed_db) = settings.level.params();
        let g = 1.0 / (1.0 + 10f64.powf(feed_db as f64 / 20.0));
        let fc = (fc as f64).min(sample_rate as f64 * 0.45);
        let x = (-2.0 * std::f64::consts::PI * fc / sample_rate as f64).exp();
        Self {
            active: true,
            a: (g * (1.0 - x)) as f32,
            b: x as f32,
        }
    }
}

/// Crossfeed filter state (one stereo pair).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CrossfeedState {
    lp: f32,
}

impl CrossfeedState {
    pub(crate) fn new() -> Self {
        Self { lp: 0.0 }
    }

    pub(crate) fn reset(&mut self) {
        self.lp = 0.0;
    }

    /// Processes one stereo frame in place.
    #[inline]
    pub(crate) fn process(&mut self, c: &CrossfeedCoeffs, l: &mut f32, r: &mut f32) {
        let mut lp = c.a * (*r - *l) + c.b * self.lp;
        // Keep the decaying state out of the denormal range.
        if lp.abs() < 1.0e-20 {
            lp = 0.0;
        }
        self.lp = lp;
        *l += lp;
        *r -= lp;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, n: usize, fs: f32) -> f32 {
        (2.0 * std::f32::consts::PI * freq * n as f32 / fs).sin()
    }

    #[test]
    fn centre_signal_stays_at_unity() {
        for level in [
            CrossfeedLevel::Strong,
            CrossfeedLevel::Medium,
            CrossfeedLevel::Weak,
        ] {
            let c = CrossfeedCoeffs::new(
                &Crossfeed {
                    enabled: true,
                    level,
                },
                48_000,
                2,
            );
            assert!(c.active);
            let mut st = CrossfeedState::new();
            for freq in [30.0, 700.0, 5000.0, 15000.0] {
                for n in 0..4800 {
                    let x = 0.7 * sine(freq, n, 48_000.0);
                    let (mut l, mut r) = (x, x);
                    st.process(&c, &mut l, &mut r);
                    assert_eq!(l, x, "{level:?} {freq} Hz");
                    assert_eq!(r, x, "{level:?} {freq} Hz");
                }
            }
        }
    }

    /// Hard-panned low frequencies: the far ear gets the feed level below the near ear (as
    /// libbs2b at DC); high frequencies stay on their side.
    #[test]
    fn hard_panned_levels_match_bs2b_feed() {
        let fs = 48_000.0;
        for level in [
            CrossfeedLevel::Strong,
            CrossfeedLevel::Medium,
            CrossfeedLevel::Weak,
        ] {
            let c = CrossfeedCoeffs::new(
                &Crossfeed {
                    enabled: true,
                    level,
                },
                48_000,
                2,
            );
            let (_, feed) = level.params();
            let measure = |freq: f32| {
                let mut st = CrossfeedState::new();
                let (mut pl, mut pr) = (0.0f32, 0.0f32);
                for n in 0..96_000 {
                    let (mut l, mut r) = (0.5 * sine(freq, n, fs), 0.0);
                    st.process(&c, &mut l, &mut r);
                    if n > 48_000 {
                        pl = pl.max(l.abs());
                        pr = pr.max(r.abs());
                    }
                }
                (pl, pr)
            };
            let (l, r) = measure(20.0);
            let ratio_db = 20.0 * (l / r).log10();
            assert!((ratio_db - feed).abs() < 0.3, "{level:?}: {ratio_db} dB");
            let (l, r) = measure(12_000.0);
            assert!(l > 0.49 && r < 0.02, "{level:?}: hf {l} {r}");
        }
    }

    /// Names follow audible strength: a stronger level feeds more of the opposite channel.
    #[test]
    fn level_names_follow_audible_strength() {
        let gain = |level: CrossfeedLevel| {
            let (_, feed_db) = level.params();
            1.0 / (1.0 + 10f32.powf(feed_db / 20.0))
        };
        assert!(gain(CrossfeedLevel::Weak) < gain(CrossfeedLevel::Medium));
        assert!(gain(CrossfeedLevel::Medium) < gain(CrossfeedLevel::Strong));
    }

    #[test]
    fn bypassed_when_not_stereo_or_disabled() {
        let on = Crossfeed {
            enabled: true,
            level: CrossfeedLevel::Medium,
        };
        assert!(!CrossfeedCoeffs::new(&on, 48_000, 1).active);
        assert!(!CrossfeedCoeffs::new(&on, 48_000, 6).active);
        assert!(!CrossfeedCoeffs::new(&Crossfeed::default(), 48_000, 2).active);
    }
}
