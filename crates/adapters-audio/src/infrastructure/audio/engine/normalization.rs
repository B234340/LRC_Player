//! Loudness normalization: ReplayGain tags / R128 hints -> one linear gain per track.
//!
//! The gain is computed on the engine thread when a track is queued (or when the settings or a
//! hint change) and handed to the renderer keyed by the track's sequence number, so it switches
//! exactly at gapless boundaries and ramps smoothly when it changes for the audible track.

/// Which ReplayGain value drives the per-track gain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NormalizationMode {
    #[default]
    Off,
    /// Per-track gain (every track at the same loudness).
    Track,
    /// Per-album gain (keeps loudness differences within an album); falls back to the track
    /// gain when a file has no album values.
    Album,
}

/// User-facing normalization settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Normalization {
    pub mode: NormalizationMode,
    /// Added to the ReplayGain/hint gain (dB). Ignored when `mode` is `Off` or the track has no
    /// loudness data.
    pub preamp_db: f32,
    /// Limits the gain so that `peak * gain <= 1.0` when the peak is known.
    pub prevent_clipping: bool,
}

impl Default for Normalization {
    fn default() -> Self {
        Self {
            mode: NormalizationMode::Off,
            preamp_db: 0.0,
            prevent_clipping: true,
        }
    }
}

/// Loudness data measured by the application (e.g. EBU R128 scan) for a file without
/// ReplayGain tags. Gains are ReplayGain-style (dB to apply), peaks are linear (1.0 = full
/// scale; `<= 0` or non-finite = unknown).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoudnessHint {
    pub track_gain_db: f32,
    pub track_peak: f32,
    pub album_gain_db: Option<f32>,
    pub album_peak: Option<f32>,
}

/// Where the normalization gain of a track came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GainSource {
    /// No loudness data (or normalization off): unity gain.
    #[default]
    None,
    /// ReplayGain tags in the file.
    Tag,
    /// A [`LoudnessHint`] supplied by the application.
    Hint,
}

/// ReplayGain values read from a file's tags (all optional).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ReplayGainTags {
    pub(crate) track_gain_db: Option<f32>,
    pub(crate) track_peak: Option<f32>,
    pub(crate) album_gain_db: Option<f32>,
    pub(crate) album_peak: Option<f32>,
}

impl ReplayGainTags {
    pub(crate) fn has_gain(&self) -> bool {
        self.track_gain_db.is_some() || self.album_gain_db.is_some()
    }
}

impl From<LoudnessHint> for ReplayGainTags {
    fn from(h: LoudnessHint) -> Self {
        Self {
            track_gain_db: Some(h.track_gain_db),
            track_peak: Some(h.track_peak),
            album_gain_db: h.album_gain_db,
            album_peak: h.album_peak,
        }
    }
}

/// Resolved per-track gain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TrackGain {
    /// Linear factor (exactly 1.0 when nothing applies, so the path stays bit-exact).
    pub(crate) linear: f32,
    pub(crate) db: f32,
    pub(crate) source: GainSource,
}

impl TrackGain {
    pub(crate) const UNITY: TrackGain = TrackGain {
        linear: 1.0,
        db: 0.0,
        source: GainSource::None,
    };
}

impl Default for TrackGain {
    fn default() -> Self {
        Self::UNITY
    }
}

/// Bounds of the final gain (protects against corrupt tags).
const MIN_GAIN_DB: f32 = -40.0;
const MAX_GAIN_DB: f32 = 24.0;

fn finite(v: Option<f32>) -> Option<f32> {
    v.filter(|x| x.is_finite())
}

fn valid_peak(v: Option<f32>) -> Option<f32> {
    v.filter(|p| p.is_finite() && *p > 0.0)
}

/// Computes the gain for a track. Tags win over the hint; the hint is used only when the file
/// has no ReplayGain gain at all.
pub(crate) fn resolve_gain(
    settings: &Normalization,
    tags: &ReplayGainTags,
    hint: Option<&LoudnessHint>,
) -> TrackGain {
    if settings.mode == NormalizationMode::Off {
        return TrackGain::UNITY;
    }
    let (data, source) = if tags.has_gain() {
        (*tags, GainSource::Tag)
    } else if let Some(h) = hint {
        (ReplayGainTags::from(*h), GainSource::Hint)
    } else {
        return TrackGain::UNITY;
    };
    let track = finite(data.track_gain_db).map(|g| (g, valid_peak(data.track_peak)));
    let album = finite(data.album_gain_db).map(|g| {
        // An album peak is >= every track peak; without one the track peak is the best bound.
        (
            g,
            valid_peak(data.album_peak).or(valid_peak(data.track_peak)),
        )
    });
    let chosen = match settings.mode {
        NormalizationMode::Album => album.or(track),
        _ => track.or(album),
    };
    let Some((gain_db, peak)) = chosen else {
        return TrackGain::UNITY;
    };
    let preamp = if settings.preamp_db.is_finite() {
        settings.preamp_db
    } else {
        0.0
    };
    let mut db = (gain_db + preamp).clamp(MIN_GAIN_DB, MAX_GAIN_DB);
    if settings.prevent_clipping {
        if let Some(p) = peak {
            // peak * gain <= 1  <=>  gain_db <= -20 log10(peak)
            db = db.min(-20.0 * p.log10());
        }
    }
    let linear = if db == 0.0 {
        1.0
    } else {
        10f32.powf(db / 20.0)
    };
    TrackGain { linear, db, source }
}

/// Parses a ReplayGain gain value such as `"-6.53 dB"`, `"+2.1dB"` or `"-6,53 dB"`.
pub(crate) fn parse_gain_db(s: &str) -> Option<f32> {
    let t = s.trim().trim_end_matches('\0').trim();
    let t = t
        .strip_suffix("dB")
        .or_else(|| t.strip_suffix("db"))
        .or_else(|| t.strip_suffix("DB"))
        .or_else(|| t.strip_suffix("Db"))
        .unwrap_or(t);
    parse_number(t)
}

/// Parses a ReplayGain peak value such as `"0.988"` (linear).
pub(crate) fn parse_peak(s: &str) -> Option<f32> {
    parse_number(s.trim().trim_end_matches('\0')).filter(|p| *p >= 0.0)
}

fn parse_number(s: &str) -> Option<f32> {
    let t = s.trim();
    let v = t
        .parse::<f32>()
        .ok()
        .or_else(|| t.replace(',', ".").parse::<f32>().ok())?;
    v.is_finite().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(tg: Option<f32>, tp: Option<f32>, ag: Option<f32>, ap: Option<f32>) -> ReplayGainTags {
        ReplayGainTags {
            track_gain_db: tg,
            track_peak: tp,
            album_gain_db: ag,
            album_peak: ap,
        }
    }

    fn mode(m: NormalizationMode) -> Normalization {
        Normalization {
            mode: m,
            preamp_db: 0.0,
            prevent_clipping: false,
        }
    }

    #[test]
    fn parses_replaygain_values() {
        assert_eq!(parse_gain_db("-6.53 dB"), Some(-6.53));
        assert_eq!(parse_gain_db("+2.10 dB"), Some(2.1));
        assert_eq!(parse_gain_db(" -0.5dB "), Some(-0.5));
        assert_eq!(parse_gain_db("-6,53 dB"), Some(-6.53));
        assert_eq!(parse_gain_db("3"), Some(3.0));
        assert_eq!(parse_gain_db("loud"), None);
        assert_eq!(parse_gain_db("NaN dB"), None);
        assert_eq!(parse_peak("0.988"), Some(0.988));
        assert_eq!(parse_peak("1.000000"), Some(1.0));
        assert_eq!(parse_peak("-1"), None);
    }

    #[test]
    fn off_mode_is_unity_even_with_tags() {
        let g = resolve_gain(
            &Normalization::default(),
            &tags(Some(-6.0), Some(0.9), None, None),
            None,
        );
        assert_eq!(g, TrackGain::UNITY);
        assert_eq!(g.linear.to_bits(), 1.0f32.to_bits());
    }

    #[test]
    fn track_album_and_fallback() {
        let t = tags(Some(-6.0), Some(0.5), Some(-8.0), Some(0.6));
        let g = resolve_gain(&mode(NormalizationMode::Track), &t, None);
        assert_eq!((g.db, g.source), (-6.0, GainSource::Tag));
        assert!((g.linear - 0.501_187).abs() < 1e-5);
        let g = resolve_gain(&mode(NormalizationMode::Album), &t, None);
        assert_eq!(g.db, -8.0);
        // Album mode without album values falls back to the track gain.
        let t = tags(Some(-3.0), None, None, None);
        assert_eq!(
            resolve_gain(&mode(NormalizationMode::Album), &t, None).db,
            -3.0
        );
    }

    #[test]
    fn preamp_is_added() {
        let mut n = mode(NormalizationMode::Track);
        n.preamp_db = 2.5;
        let g = resolve_gain(&n, &tags(Some(-6.0), None, None, None), None);
        assert!((g.db - -3.5).abs() < 1e-6);
    }

    #[test]
    fn clipping_prevention_limits_gain_to_peak() {
        let mut n = mode(NormalizationMode::Track);
        n.prevent_clipping = true;
        // +6 dB on a track peaking at 0.8 would clip: limited to 1/0.8 (+1.94 dB).
        let g = resolve_gain(&n, &tags(Some(6.0), Some(0.8), None, None), None);
        assert!((g.linear * 0.8 - 1.0).abs() < 1e-5, "{g:?}");
        // Negative gains are unaffected.
        let g = resolve_gain(&n, &tags(Some(-6.0), Some(0.8), None, None), None);
        assert_eq!(g.db, -6.0);
        // Unknown peak: no limit.
        let g = resolve_gain(&n, &tags(Some(6.0), None, None, None), None);
        assert_eq!(g.db, 6.0);
        // Album mode uses the album peak.
        let mut a = n;
        a.mode = NormalizationMode::Album;
        let g = resolve_gain(&a, &tags(Some(1.0), Some(0.5), Some(6.0), Some(0.9)), None);
        assert!((g.linear * 0.9 - 1.0).abs() < 1e-5, "{g:?}");
        // Without clipping prevention the full gain applies.
        let g = resolve_gain(
            &mode(NormalizationMode::Track),
            &tags(Some(6.0), Some(0.8), None, None),
            None,
        );
        assert_eq!(g.db, 6.0);
    }

    #[test]
    fn tags_win_over_hints() {
        let n = mode(NormalizationMode::Track);
        let hint = LoudnessHint {
            track_gain_db: -9.0,
            track_peak: 1.0,
            album_gain_db: None,
            album_peak: None,
        };
        let g = resolve_gain(&n, &tags(Some(-2.0), None, None, None), Some(&hint));
        assert_eq!((g.db, g.source), (-2.0, GainSource::Tag));
        let g = resolve_gain(&n, &ReplayGainTags::default(), Some(&hint));
        assert_eq!((g.db, g.source), (-9.0, GainSource::Hint));
        let g = resolve_gain(&n, &ReplayGainTags::default(), None);
        assert_eq!(g, TrackGain::UNITY);
    }
}
