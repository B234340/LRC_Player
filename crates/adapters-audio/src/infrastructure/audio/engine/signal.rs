//! Description of the audible track's signal chain ([`SignalPath`], in `EngineSnapshot`).

use super::convert::ResamplerKind;
use super::crossfeed::CrossfeedLevel;
use super::normalization::GainSource;
use super::DeviceInfo;

/// What happens to the audible track between the file and the device.
#[derive(Clone, Debug, PartialEq)]
pub struct SignalPath {
    /// Source codec (symphonia short name, e.g. "flac", "mp3", "pcm_s16le").
    pub codec: String,
    pub source_rate: u32,
    /// Bits per sample of the source, when the codec has one (lossless/PCM).
    pub source_bits: Option<u32>,
    pub source_channels: u16,
    /// The source rate differs from the device rate.
    pub resampled: bool,
    /// The resampler in use (`Some` only when `resampled`).
    pub resampler: Option<ResamplerKind>,
    /// Source channels differ from the device channels (up/down-mix).
    pub channels_mapped: bool,
    /// The output device (rate, format, channels, ...).
    pub output: DeviceInfo,
    /// The 10-band EQ is active (enabled and not flat).
    pub eq: bool,
    /// The parametric EQ is active (enabled with at least one effective filter or a preamp).
    pub parametric_eq: bool,
    /// Crossfeed level, when crossfeed is enabled and the output is stereo.
    pub crossfeed: Option<CrossfeedLevel>,
    /// Normalization gain of this track (dB; 0 when off or without loudness data).
    pub normalization_db: f32,
    pub normalization_source: GainSource,
    /// Automatic headroom applied for EQ boosts (dB, <= 0).
    pub headroom_db: f32,
    /// Effective volume (0 when muted).
    pub volume: f32,
    /// Volume differs from 1.0 (or muted).
    pub volume_active: bool,
    /// The output limiter changed samples within the last second.
    pub limiter_engaged: bool,
    /// The device receives the decoded samples unchanged: equal rate and channels, every
    /// stage off/unity, volume exactly 1.0, and a device format at least as wide as the source.
    pub bit_perfect: bool,
}

/// Effective resolution (bits) of a device sample format name ("I16", "F32", ...).
fn format_bits(fmt: &str) -> Option<u32> {
    let f = fmt.trim().to_ascii_uppercase();
    match f.as_str() {
        // f32 represents every integer sample up to 24 bits exactly.
        "F32" => Some(24),
        "F64" => Some(53),
        _ => f
            .strip_prefix('I')
            .or_else(|| f.strip_prefix('U'))
            .and_then(|b| b.parse().ok()),
    }
}

impl SignalPath {
    /// Recomputes [`SignalPath::bit_perfect`] from the other fields.
    pub(crate) fn compute_bit_perfect(&self) -> bool {
        let wide_enough = match (self.source_bits, format_bits(&self.output.sample_format)) {
            (Some(src), Some(dev)) => dev >= src,
            _ => true,
        };
        !self.resampled
            && !self.channels_mapped
            && !self.eq
            && !self.parametric_eq
            && self.crossfeed.is_none()
            && self.normalization_db == 0.0
            && self.headroom_db == 0.0
            && self.volume == 1.0
            && wide_enough
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> SignalPath {
        let mut s = SignalPath {
            codec: "flac".into(),
            source_rate: 48_000,
            source_bits: Some(16),
            source_channels: 2,
            resampled: false,
            resampler: None,
            channels_mapped: false,
            output: DeviceInfo {
                name: "dev".into(),
                sample_rate: 48_000,
                channels: 2,
                sample_format: "F32".into(),
                ..Default::default()
            },
            eq: false,
            parametric_eq: false,
            crossfeed: None,
            normalization_db: 0.0,
            normalization_source: GainSource::None,
            headroom_db: 0.0,
            volume: 1.0,
            volume_active: false,
            limiter_engaged: false,
            bit_perfect: false,
        };
        s.bit_perfect = s.compute_bit_perfect();
        s
    }

    #[test]
    fn bit_perfect_only_when_everything_is_off() {
        assert!(plain().compute_bit_perfect());
        let cases: [fn(&mut SignalPath); 9] = [
            |s| s.resampled = true,
            |s| s.channels_mapped = true,
            |s| s.eq = true,
            |s| s.parametric_eq = true,
            |s| s.crossfeed = Some(CrossfeedLevel::Strong),
            |s| s.normalization_db = -3.0,
            |s| s.headroom_db = -1.0,
            |s| s.volume = 0.5,
            |s| {
                s.source_bits = Some(24);
                s.output.sample_format = "I16".into();
            },
        ];
        for (i, f) in cases.iter().enumerate() {
            let mut s = plain();
            f(&mut s);
            assert!(!s.compute_bit_perfect(), "case {i}");
        }
        let mut s = plain();
        s.output.sample_format = "I16".into();
        assert!(s.compute_bit_perfect(), "16-bit source on a 16-bit device");
        s.source_bits = None;
        assert!(s.compute_bit_perfect(), "unknown source depth");
    }

    #[test]
    fn format_bits_parses_cpal_names() {
        assert_eq!(format_bits("I16"), Some(16));
        assert_eq!(format_bits("I24"), Some(24));
        assert_eq!(format_bits("U8"), Some(8));
        assert_eq!(format_bits("F32"), Some(24));
        assert_eq!(format_bits("weird"), None);
    }
}
