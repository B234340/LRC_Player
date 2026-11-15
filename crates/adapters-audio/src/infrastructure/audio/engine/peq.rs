//! Parametric EQ: user-defined RBJ-cookbook biquads (peak, shelves, low/high-pass), up to
//! [`PEQ_MAX_FILTERS`], with its own preamp; plus a parser for the Equalizer APO / AutoEq
//! `ParametricEQ.txt` format. Runs after (in series with) the 10-band graphic EQ.

use anyhow::{anyhow, bail, Result};

use super::dsp::{db_to_linear, BiquadCoeffs, ANTI_DENORMAL, EQ_MAX_CHANNELS};

/// Maximum number of filters processed (extra filters are ignored).
pub const PEQ_MAX_FILTERS: usize = 20;

/// Gain range of a filter and of the PEQ preamp (dB).
pub const PEQ_MAX_GAIN_DB: f32 = 30.0;

/// Biquad type of a parametric-EQ filter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilterKind {
    #[default]
    Peak,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
}

/// One parametric-EQ filter. `gain_db` is ignored by the pass filters; `q` is the RBJ Q (for
/// shelves: the shelf Q, 0.7071 = slope 1, the steepest without overshoot).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeqFilter {
    pub kind: FilterKind,
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
}

/// User-facing parametric EQ. Only the first [`PEQ_MAX_FILTERS`] filters are used; the engine
/// converts this into a fixed-size coefficient array (no allocation in the audio callback).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParametricEq {
    pub enabled: bool,
    /// Preamp in dB (clamped to ±30).
    pub preamp_db: f32,
    pub filters: Vec<PeqFilter>,
}

const DEFAULT_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Precomputed parametric EQ (computed off the realtime thread, sent by value).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PeqCoeffs {
    pub(crate) active: bool,
    pub(crate) preamp: f32,
    pub(crate) filters: [BiquadCoeffs; PEQ_MAX_FILTERS],
    /// Number of (non-identity) filters at the front of `filters`.
    pub(crate) count: usize,
}

fn sanitize(v: f32, lo: f32, hi: f32, default: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        default
    }
}

impl PeqFilter {
    /// Filter with sanitized parameters (finite, in range).
    pub(crate) fn sanitized(&self) -> PeqFilter {
        PeqFilter {
            kind: self.kind,
            freq_hz: sanitize(self.freq_hz, 1.0, 100_000.0, 1000.0),
            gain_db: sanitize(self.gain_db, -PEQ_MAX_GAIN_DB, PEQ_MAX_GAIN_DB, 0.0),
            q: sanitize(self.q, 0.025, 40.0, DEFAULT_Q),
        }
    }

    pub(crate) fn coeffs(&self, sample_rate: u32) -> BiquadCoeffs {
        let f = self.sanitized();
        BiquadCoeffs::design(f.kind, sample_rate, f.freq_hz, f.gain_db, f.q)
    }
}

impl ParametricEq {
    /// Preamp in dB as applied (sanitized).
    pub(crate) fn preamp_db_sanitized(&self) -> f32 {
        sanitize(self.preamp_db, -PEQ_MAX_GAIN_DB, PEQ_MAX_GAIN_DB, 0.0)
    }

    /// The filters that are processed.
    pub(crate) fn used_filters(&self) -> &[PeqFilter] {
        &self.filters[..self.filters.len().min(PEQ_MAX_FILTERS)]
    }
}

impl PeqCoeffs {
    pub(crate) const BYPASS: PeqCoeffs = PeqCoeffs {
        active: false,
        preamp: 1.0,
        filters: [BiquadCoeffs::IDENTITY; PEQ_MAX_FILTERS],
        count: 0,
    };

    pub(crate) fn from_settings(eq: &ParametricEq, sample_rate: u32) -> Self {
        if !eq.enabled {
            return Self::BYPASS;
        }
        let mut c = Self::BYPASS;
        for f in eq.used_filters() {
            let b = f.coeffs(sample_rate);
            if b != BiquadCoeffs::IDENTITY {
                c.filters[c.count] = b;
                c.count += 1;
            }
        }
        let pre_db = eq.preamp_db_sanitized();
        c.preamp = if pre_db.abs() < 1.0e-3 {
            1.0
        } else {
            db_to_linear(pre_db)
        };
        c.active = c.count > 0 || c.preamp != 1.0;
        c
    }
}

/// Per-channel filter state of the parametric EQ (fixed size, `Copy`).
#[derive(Clone, Copy)]
pub(crate) struct PeqState {
    state: [[[f32; 2]; PEQ_MAX_FILTERS]; EQ_MAX_CHANNELS],
}

impl PeqState {
    pub(crate) fn new() -> Self {
        Self {
            state: [[[0.0; 2]; PEQ_MAX_FILTERS]; EQ_MAX_CHANNELS],
        }
    }

    pub(crate) fn reset(&mut self) {
        self.state = [[[0.0; 2]; PEQ_MAX_FILTERS]; EQ_MAX_CHANNELS];
    }

    /// Filters one sample of channel `ch` (preamp NOT applied here).
    #[inline]
    pub(crate) fn process(&mut self, c: &PeqCoeffs, ch: usize, x: f32) -> f32 {
        if ch >= EQ_MAX_CHANNELS || c.count == 0 {
            return x;
        }
        let mut y = x + ANTI_DENORMAL;
        let states = &mut self.state[ch];
        for (f, s) in c.filters[..c.count].iter().zip(states.iter_mut()) {
            y = f.tick(s, y);
        }
        y
    }
}

// ----- AutoEq / Equalizer APO parser -------------------------------------------------------

/// Parses the Equalizer APO / AutoEq `ParametricEQ.txt` format:
///
/// ```text
/// Preamp: -6.2 dB
/// Filter 1: ON PK Fc 105 Hz Gain 4.2 dB Q 0.70
/// Filter 2: ON LSC Fc 105 Hz Gain 2.0 dB Q 0.70
/// Filter 3: OFF HSC Fc 10000 Hz Gain -3.0 dB Q 0.70
/// ```
///
/// Supported types: `PK`/`PEQ` (peak), `LSC`/`LS` and `HSC`/`HS` (shelves, with `Q x`, an
/// optional slope `LSC 6 dB ...` or the default slope 1 i.e. Q 0.7071), `LP`/`LPQ` and
/// `HP`/`HPQ` (Q optional, default 0.7071). A peak's bandwidth may be given as `BW Oct x`
/// instead of `Q`. Frequencies accept `Hz` or `kHz`. Several `Preamp:` lines add up. `OFF`
/// filters, comments (`#`, `;`, `//`) and unrecognised lines/filter types are ignored. Fails
/// when no active filter is found, on a malformed active filter (with its line number), or
/// when there are more than [`PEQ_MAX_FILTERS`] active filters. The result is `enabled`.
pub fn parse_autoeq(text: &str) -> Result<ParametricEq> {
    let mut preamp_db = 0.0f32;
    let mut filters = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with(';')
            || line.starts_with("//")
        {
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        if key == "preamp" {
            let v = parse_db_value(rest)
                .ok_or_else(|| anyhow!("line {line_no}: invalid preamp {:?}", rest.trim()))?;
            preamp_db += v;
        } else if key == "filter" || key.starts_with("filter ") {
            if let Some(f) = parse_filter(rest).map_err(|e| anyhow!("line {line_no}: {e}"))? {
                filters.push(f);
            }
        }
    }
    if filters.is_empty() {
        bail!(
            "no active filters found (expected lines like \
             \"Filter 1: ON PK Fc 105 Hz Gain 4.2 dB Q 0.70\")"
        );
    }
    if filters.len() > PEQ_MAX_FILTERS {
        bail!(
            "too many active filters ({} > {PEQ_MAX_FILTERS})",
            filters.len()
        );
    }
    Ok(ParametricEq {
        enabled: true,
        preamp_db,
        filters,
    })
}

/// "-6.2 dB" / "-6.2dB" / "-6.2".
fn parse_db_value(s: &str) -> Option<f32> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    let num = lower.strip_suffix("db").unwrap_or(&lower).trim();
    num.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn num(tok: Option<&str>, what: &str) -> Result<f32> {
    let t = tok.ok_or_else(|| anyhow!("missing value after {what}"))?;
    let lower = t.to_ascii_lowercase();
    let t = lower
        .strip_suffix("khz")
        .map(|s| (s, 1000.0))
        .or_else(|| lower.strip_suffix("hz").map(|s| (s, 1.0)))
        .or_else(|| lower.strip_suffix("db").map(|s| (s, 1.0)))
        .unwrap_or((&lower, 1.0));
    let v =
        t.0.parse::<f32>()
            .map_err(|_| anyhow!("invalid {what} value {t:?}", t = t.0))?;
    if !v.is_finite() {
        bail!("invalid {what} value");
    }
    Ok(v * t.1)
}

/// Parses the part after "Filter N:". `Ok(None)` for OFF filters and unknown types.
fn parse_filter(rest: &str) -> Result<Option<PeqFilter>> {
    let toks: Vec<&str> = rest.split_whitespace().collect();
    let mut i = 0;
    match toks.first().map(|t| t.to_ascii_uppercase()) {
        Some(t) if t == "ON" => i += 1,
        Some(t) if t == "OFF" => return Ok(None),
        _ => {}
    }
    let Some(ty) = toks.get(i).map(|t| t.to_ascii_uppercase()) else {
        bail!("missing filter type");
    };
    i += 1;
    let kind = match ty.as_str() {
        "PK" | "PEQ" | "MODAL" => FilterKind::Peak,
        "LS" | "LSC" | "LSQ" => FilterKind::LowShelf,
        "HS" | "HSC" | "HSQ" => FilterKind::HighShelf,
        "LP" | "LPQ" => FilterKind::LowPass,
        "HP" | "HPQ" => FilterKind::HighPass,
        _ => return Ok(None),
    };
    // Optional shelf slope right after the type: "LSC 6 dB" / "LS 12dB".
    let mut slope_db: Option<f32> = None;
    if let Some(t) = toks.get(i) {
        let lower = t.to_ascii_lowercase();
        let n = lower.strip_suffix("db").unwrap_or(&lower);
        if let Ok(v) = n.parse::<f32>() {
            slope_db = Some(v);
            i += 1;
            if toks.get(i).is_some_and(|t| t.eq_ignore_ascii_case("db")) {
                i += 1;
            }
        }
    }
    let (mut fc, mut gain, mut q, mut bw) = (None, None, None, None);
    while i < toks.len() {
        let key = toks[i].to_ascii_lowercase();
        i += 1;
        match key.as_str() {
            "fc" => {
                let mut v = num(toks.get(i).copied(), "Fc")?;
                i += 1;
                match toks.get(i).map(|t| t.to_ascii_lowercase()) {
                    Some(u) if u == "hz" => i += 1,
                    Some(u) if u == "khz" => {
                        v *= 1000.0;
                        i += 1;
                    }
                    _ => {}
                }
                fc = Some(v);
            }
            "gain" => {
                gain = Some(num(toks.get(i).copied(), "Gain")?);
                i += 1;
                if toks.get(i).is_some_and(|t| t.eq_ignore_ascii_case("db")) {
                    i += 1;
                }
            }
            "q" => {
                q = Some(num(toks.get(i).copied(), "Q")?);
                i += 1;
            }
            "bw" => {
                // "BW Oct 1.0"
                if toks.get(i).is_some_and(|t| t.eq_ignore_ascii_case("oct")) {
                    i += 1;
                }
                bw = Some(num(toks.get(i).copied(), "BW")?);
                i += 1;
            }
            _ => {}
        }
    }
    let fc = fc.ok_or_else(|| anyhow!("{ty} filter without Fc"))?;
    if fc <= 0.0 {
        bail!("{ty} filter with non-positive Fc {fc}");
    }
    let needs_gain = matches!(
        kind,
        FilterKind::Peak | FilterKind::LowShelf | FilterKind::HighShelf
    );
    let gain = match gain {
        Some(g) => g,
        None if needs_gain => bail!("{ty} filter without Gain"),
        None => 0.0,
    };
    let q = match (q, bw, slope_db) {
        (Some(q), _, _) => q,
        (None, Some(bw), _) if bw > 0.0 => {
            let p = 2f32.powf(bw);
            p.sqrt() / (p - 1.0)
        }
        (None, _, Some(s)) if needs_gain && kind != FilterKind::Peak && s > 0.0 => {
            shelf_q_from_slope(gain, (s / 12.0).min(1.0))
        }
        _ => DEFAULT_Q,
    };
    if q <= 0.0 {
        bail!("{ty} filter with non-positive Q {q}");
    }
    Ok(Some(PeqFilter {
        kind,
        freq_hz: fc,
        gain_db: gain,
        q,
    }))
}

/// RBJ shelf slope S (1 = steepest monotonic) -> equivalent Q.
fn shelf_q_from_slope(gain_db: f32, s: f32) -> f32 {
    let a = 10f32.powf(gain_db / 40.0);
    let inv_q2 = (a + 1.0 / a) * (1.0 / s - 1.0) + 2.0;
    if inv_q2 > 0.0 {
        1.0 / inv_q2.sqrt()
    } else {
        DEFAULT_Q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real AutoEq result (Sennheiser HD 650, oratory1990 target), Equalizer APO format.
    const AUTOEQ_SAMPLE: &str = "\
Preamp: -6.4 dB
Filter 1: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.70
Filter 2: ON PK Fc 61 Hz Gain -0.9 dB Q 0.69
Filter 3: ON PK Fc 2153 Hz Gain 3.2 dB Q 1.63
Filter 4: ON PK Fc 5523 Hz Gain -2.3 dB Q 3.85
Filter 5: ON PK Fc 3255 Hz Gain -1.7 dB Q 3.73
Filter 6: ON PK Fc 147 Hz Gain -0.7 dB Q 1.73
Filter 7: ON PK Fc 1186 Hz Gain -0.4 dB Q 2.66
Filter 8: ON PK Fc 8214 Hz Gain 1.5 dB Q 3.88
Filter 9: ON HSC Fc 10000 Hz Gain -1.4 dB Q 0.70
Filter 10: ON PK Fc 19693 Hz Gain -4.7 dB Q 0.40
";

    #[test]
    fn parses_autoeq_sample() {
        let eq = parse_autoeq(AUTOEQ_SAMPLE).unwrap();
        assert!(eq.enabled);
        assert_eq!(eq.preamp_db, -6.4);
        assert_eq!(eq.filters.len(), 10);
        assert_eq!(
            eq.filters[0],
            PeqFilter {
                kind: FilterKind::LowShelf,
                freq_hz: 105.0,
                gain_db: 5.5,
                q: 0.70
            }
        );
        assert_eq!(
            eq.filters[2],
            PeqFilter {
                kind: FilterKind::Peak,
                freq_hz: 2153.0,
                gain_db: 3.2,
                q: 1.63
            }
        );
        assert_eq!(eq.filters[8].kind, FilterKind::HighShelf);
        assert_eq!(eq.filters[9].freq_hz, 19693.0);
    }

    #[test]
    fn parser_handles_variants_comments_and_off_filters() {
        let text = "\
# headphone EQ
; another comment
Device: Speakers
Preamp: -3 dB
Preamp: -1.5dB
Filter 1: OFF PK Fc 100 Hz Gain 9 dB Q 1
Filter 2: ON PEQ Fc 1 kHz Gain -2 dB Q 2
Filter 3: ON LS Fc 80 Hz Gain 4 dB
Filter 4: ON HPQ Fc 20 Hz Q 0.5
Filter 5: ON LP Fc 18000 Hz
Filter 6: ON NO Fc 50 Hz
Filter 7: ON PK Fc 300 Hz Gain 1 dB BW Oct 1.0
Filter 8: ON HSC 6 dB Fc 8000 Hz Gain -4 dB
Filter: ON PK Fc 440 Hz Gain 1 dB Q 4
something unknown
";
        let eq = parse_autoeq(text).unwrap();
        assert_eq!(eq.preamp_db, -4.5);
        let kinds: Vec<FilterKind> = eq.filters.iter().map(|f| f.kind).collect();
        assert_eq!(
            kinds,
            vec![
                FilterKind::Peak,
                FilterKind::LowShelf,
                FilterKind::HighPass,
                FilterKind::LowPass,
                FilterKind::Peak,
                FilterKind::HighShelf,
                FilterKind::Peak,
            ]
        );
        assert_eq!(eq.filters[0].freq_hz, 1000.0);
        assert_eq!(eq.filters[1].q, DEFAULT_Q);
        assert_eq!(eq.filters[2].q, 0.5);
        assert_eq!(eq.filters[3].q, DEFAULT_Q);
        // One octave bandwidth ~ Q 1.414.
        assert!((eq.filters[4].q - 1.414).abs() < 0.01);
        // Half slope (6 dB/oct) -> shallower than Q 0.7071.
        assert!(eq.filters[5].q < 0.6);
    }

    #[test]
    fn parser_errors_are_clear() {
        let e = parse_autoeq("Preamp: -3 dB\n# nothing\n").unwrap_err();
        assert!(e.to_string().contains("no active filters"), "{e}");
        let e = parse_autoeq("Filter 1: OFF PK Fc 100 Hz Gain 1 dB Q 1\n").unwrap_err();
        assert!(e.to_string().contains("no active filters"), "{e}");
        let e = parse_autoeq("").unwrap_err();
        assert!(e.to_string().contains("no active filters"), "{e}");
        let e = parse_autoeq("Filter 1: ON PK Gain 1 dB Q 1\n").unwrap_err();
        assert!(
            e.to_string().contains("line 1") && e.to_string().contains("Fc"),
            "{e}"
        );
        let e = parse_autoeq("\nFilter 1: ON PK Fc abc Hz Gain 1 dB Q 1\n").unwrap_err();
        assert!(e.to_string().contains("line 2"), "{e}");
        let e = parse_autoeq("Filter 1: ON PK Fc 100 Hz Q 1\n").unwrap_err();
        assert!(e.to_string().contains("Gain"), "{e}");
        let many: String = (1..=21)
            .map(|i| format!("Filter {i}: ON PK Fc {} Hz Gain 1 dB Q 1\n", 100 * i))
            .collect();
        let e = parse_autoeq(&many).unwrap_err();
        assert!(e.to_string().contains("too many"), "{e}");
    }

    #[test]
    fn coeffs_skip_identity_filters_and_respect_enabled() {
        let mut eq = parse_autoeq(AUTOEQ_SAMPLE).unwrap();
        let c = PeqCoeffs::from_settings(&eq, 48_000);
        assert!(c.active);
        assert_eq!(c.count, 10);
        eq.filters.push(PeqFilter {
            kind: FilterKind::Peak,
            freq_hz: 1000.0,
            gain_db: 0.0,
            q: 1.0,
        });
        assert_eq!(PeqCoeffs::from_settings(&eq, 48_000).count, 10);
        eq.enabled = false;
        assert!(!PeqCoeffs::from_settings(&eq, 48_000).active);
        let flat = ParametricEq {
            enabled: true,
            preamp_db: 0.0,
            filters: vec![],
        };
        assert!(!PeqCoeffs::from_settings(&flat, 48_000).active);
    }

    /// Steady-state gain of a sine at `freq` through the cascade, in dB.
    fn measured_gain_db(c: &PeqCoeffs, fs: u32, freq: f32) -> f32 {
        let mut st = PeqState::new();
        let n = fs as usize;
        let mut peak = 0.0f32;
        for i in 0..n {
            let x = 0.1 * (2.0 * std::f64::consts::PI * freq as f64 * i as f64 / fs as f64).sin();
            let y = st.process(c, 0, x as f32);
            if i > n / 2 {
                peak = peak.max(y.abs());
            }
        }
        20.0 * (peak / 0.1).log10()
    }

    #[test]
    fn filter_responses_match_their_parameters() {
        let fs = 48_000;
        let one = |kind, f, g, q| {
            PeqCoeffs::from_settings(
                &ParametricEq {
                    enabled: true,
                    preamp_db: 0.0,
                    filters: vec![PeqFilter {
                        kind,
                        freq_hz: f,
                        gain_db: g,
                        q,
                    }],
                },
                fs,
            )
        };
        // Peak: the gain at Fc, ~0 dB far away (analytic response and measured sine).
        let pk = one(FilterKind::Peak, 1000.0, 6.0, 1.0);
        let b = pk.filters[0];
        assert!((b.magnitude_db(1000.0, fs) - 6.0).abs() < 0.01);
        assert!(b.magnitude_db(20.0, fs).abs() < 0.05);
        assert!((measured_gain_db(&pk, fs, 1000.0) - 6.0).abs() < 0.1);
        let cut = one(FilterKind::Peak, 3000.0, -9.0, 4.0);
        assert!((cut.filters[0].magnitude_db(3000.0, fs) + 9.0).abs() < 0.01);
        // Shelves: full gain deep in the shelf, half the gain (in dB) at Fc, flat elsewhere.
        let ls = one(FilterKind::LowShelf, 200.0, 8.0, DEFAULT_Q).filters[0];
        assert!((ls.magnitude_db(10.0, fs) - 8.0).abs() < 0.1);
        assert!((ls.magnitude_db(200.0, fs) - 4.0).abs() < 0.05);
        assert!(ls.magnitude_db(10_000.0, fs).abs() < 0.05);
        let hs = one(FilterKind::HighShelf, 4000.0, -6.0, DEFAULT_Q).filters[0];
        assert!((hs.magnitude_db(20_000.0, fs) + 6.0).abs() < 0.2);
        assert!((hs.magnitude_db(4000.0, fs) + 3.0).abs() < 0.05);
        assert!(hs.magnitude_db(50.0, fs).abs() < 0.05);
        // Butterworth pass filters: -3 dB at Fc, flat in the pass band, steep stop band.
        let lp = one(FilterKind::LowPass, 2000.0, 0.0, DEFAULT_Q).filters[0];
        assert!((lp.magnitude_db(2000.0, fs) + 3.01).abs() < 0.05);
        assert!(lp.magnitude_db(100.0, fs).abs() < 0.05);
        assert!(lp.magnitude_db(16_000.0, fs) < -30.0);
        let hp = one(FilterKind::HighPass, 100.0, 0.0, DEFAULT_Q).filters[0];
        assert!((hp.magnitude_db(100.0, fs) + 3.01).abs() < 0.05);
        assert!(hp.magnitude_db(5000.0, fs).abs() < 0.05);
        assert!(hp.magnitude_db(25.0, fs) < -20.0);
        let lpm = one(FilterKind::LowPass, 2000.0, 0.0, DEFAULT_Q);
        assert!((measured_gain_db(&lpm, fs, 2000.0) + 3.01).abs() < 0.1);
    }

    #[test]
    fn autoeq_cascade_matches_sum_of_responses() {
        let eq = parse_autoeq(AUTOEQ_SAMPLE).unwrap();
        let c = PeqCoeffs::from_settings(&eq, 44_100);
        for f in [50.0f32, 2153.0, 8214.0] {
            let analytic: f32 = c.filters[..c.count]
                .iter()
                .map(|b| b.magnitude_db(f, 44_100))
                .sum();
            let measured = measured_gain_db(&c, 44_100, f);
            assert!(
                (analytic - measured).abs() < 0.15,
                "{f} Hz: {analytic} vs {measured}"
            );
        }
    }
}
