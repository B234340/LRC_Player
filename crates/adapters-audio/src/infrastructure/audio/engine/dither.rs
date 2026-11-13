//! Output-stage quantization: the renderer's f32 samples to the device's integer format, with
//! TPDF dither.
//!
//! Dither is added only when it does something useful *and* cannot break bit-perfect playback:
//! - the device format is an integer format of at most 24 effective bits (i8/u8, i16/u16,
//!   24-bit packed or 24-in-32). f32 output needs none, and 32-bit integers are finer than the
//!   f32 mantissa, so every f32 sample maps onto them without loss;
//! - the mode is [`DitherMode::Auto`];
//! - the renderer reports `processing_active` (gain != 1, EQ, resampling, fades...). Untouched
//!   audio from an integer source is converted back exactly (the scale factors mirror
//!   symphonia's int -> f32 conversion: 2^(bits-1)), so a 16-bit file on a 16-bit device stays
//!   bit-perfect.
//!
//! The dither is TPDF: the sum of two independent uniform values of +-0.5 LSB, i.e. a
//! triangular distribution over +-1 LSB. It makes the quantization error independent of the
//! signal (no harmonic distortion on quiet passages and fades) at the cost of a constant noise
//! floor (total error power 1/4 LSB^2 instead of 1/12). No noise shaping is applied: the plain
//! TPDF floor at 16 bit (about -101 dBFS) is inaudible at sane listening levels, and a shaped
//! filter would add a stability concern near clipping for no practical benefit.
//!
//! Everything here is allocation-free and safe to call from the realtime thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::renderer::RtShared;

/// Dither policy for integer output formats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DitherMode {
    /// TPDF dither on integer formats of <= 24 bits, only while the signal is processed.
    #[default]
    Auto,
    /// Never dither (plain rounding).
    Off,
}

/// Deepest effective integer format that is dithered.
pub(crate) const MAX_DITHER_BITS: u32 = 24;

/// Whether a format with `effective_bits` of integer resolution (`None` = float) is dithered
/// at all under [`DitherMode::Auto`].
pub(crate) fn format_ditherable(effective_bits: Option<u32>) -> bool {
    effective_bits.is_some_and(|b| b <= MAX_DITHER_BITS)
}

/// The decision taken for every output buffer.
#[inline]
pub(crate) fn should_dither(
    mode: DitherMode,
    processing_active: bool,
    effective_bits: Option<u32>,
) -> bool {
    mode == DitherMode::Auto && processing_active && format_ditherable(effective_bits)
}

/// Shared switch between the control side ([`super::AudioEngine::set_dither`]) and the output
/// streams, plus the renderer's `processing_active` flag.
#[derive(Clone)]
pub(crate) struct DitherControl {
    auto: Arc<AtomicBool>,
    shared: Arc<RtShared>,
}

impl DitherControl {
    pub(crate) fn new(shared: Arc<RtShared>, mode: DitherMode) -> Self {
        Self {
            auto: Arc::new(AtomicBool::new(mode == DitherMode::Auto)),
            shared,
        }
    }

    pub(crate) fn set_mode(&self, mode: DitherMode) {
        self.auto.store(mode == DitherMode::Auto, Ordering::Relaxed);
    }

    pub(crate) fn mode(&self) -> DitherMode {
        if self.auto.load(Ordering::Relaxed) {
            DitherMode::Auto
        } else {
            DitherMode::Off
        }
    }

    /// Whether a buffer for a format of `effective_bits` must be dithered right now.
    #[inline]
    pub(crate) fn active(&self, effective_bits: Option<u32>) -> bool {
        should_dither(
            self.mode(),
            self.shared.processing_active.load(Ordering::Relaxed),
            effective_bits,
        )
    }

    /// What `DeviceInfo::dither` reports for a format: dither is armed (it is applied while
    /// the signal is processed).
    pub(crate) fn armed(&self, effective_bits: Option<u32>) -> bool {
        self.mode() == DitherMode::Auto && format_ditherable(effective_bits)
    }
}

/// TPDF noise source: xorshift64* (one step per sample yields both uniform values).
#[derive(Clone, Debug)]
pub(crate) struct Tpdf {
    state: u64,
}

impl Tpdf {
    pub(crate) fn new(seed: u64) -> Self {
        // xorshift must never be seeded with 0; mix the seed so that small seeds differ.
        let s = seed ^ 0x9E37_79B9_7F4A_7C15;
        Self {
            state: if s == 0 { 0x2545_F491_4F6C_DD1D } else { s },
        }
    }

    /// A seed that differs between streams (no allocation, not for crypto).
    pub(crate) fn seeded_from_clock() -> Self {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x1234_5678);
        let local = 0u8;
        Self::new(t ^ ((&local as *const u8 as u64).rotate_left(32)))
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Triangular noise in LSB units, in (-1, 1): the sum of two uniforms in [-0.5, 0.5).
    #[inline]
    pub(crate) fn next(&mut self) -> f64 {
        const INV: f64 = 1.0 / (1u64 << 32) as f64;
        let r = self.next_u64();
        let a = (r >> 32) as f64 * INV - 0.5;
        let b = (r & 0xFFFF_FFFF) as f64 * INV - 0.5;
        a + b
    }
}

/// Quantizes `v` (full scale +-1.0) to a signed `bits`-bit integer (2 <= bits <= 32), adding
/// `noise_lsb` before rounding; clamps to the integer range. NaN maps to 0.
#[inline]
pub(crate) fn quantize(v: f32, bits: u32, noise_lsb: f64) -> i32 {
    let scale = (1u64 << (bits - 1)) as f64;
    let x = (v as f64 * scale + noise_lsb).round();
    x.clamp(-scale, scale - 1.0) as i32
}

/// Sample layouts the exclusive (WASAPI) output can write, in order of preference.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SampleEncoding {
    /// 32-bit signed integer, 32 valid bits.
    I32,
    /// 24 valid bits, left-justified in a 32-bit container.
    I24In32,
    /// 24-bit packed (3 bytes per sample).
    I24,
    I16,
    F32,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl SampleEncoding {
    /// Negotiation order: deepest integer formats first (the device then does no conversion
    /// of its own), float last.
    pub(crate) const PREFERENCE: [SampleEncoding; 5] = [
        SampleEncoding::I32,
        SampleEncoding::I24In32,
        SampleEncoding::I24,
        SampleEncoding::I16,
        SampleEncoding::F32,
    ];

    /// Container size of one sample.
    pub(crate) fn bytes(self) -> usize {
        match self {
            SampleEncoding::I32 | SampleEncoding::I24In32 | SampleEncoding::F32 => 4,
            SampleEncoding::I24 => 3,
            SampleEncoding::I16 => 2,
        }
    }

    pub(crate) fn container_bits(self) -> u32 {
        self.bytes() as u32 * 8
    }

    pub(crate) fn valid_bits(self) -> u32 {
        match self {
            SampleEncoding::I32 | SampleEncoding::F32 => 32,
            SampleEncoding::I24In32 | SampleEncoding::I24 => 24,
            SampleEncoding::I16 => 16,
        }
    }

    pub(crate) fn is_float(self) -> bool {
        self == SampleEncoding::F32
    }

    /// Integer resolution (`None` for float).
    pub(crate) fn effective_bits(self) -> Option<u32> {
        (!self.is_float()).then_some(self.valid_bits())
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            SampleEncoding::I32 => "I32",
            SampleEncoding::I24In32 => "I24in32",
            SampleEncoding::I24 => "I24",
            SampleEncoding::I16 => "I16",
            SampleEncoding::F32 => "F32",
        }
    }
}

/// Encodes interleaved f32 samples as little-endian device samples. `dst` must hold exactly
/// `src.len() * enc.bytes()` bytes (extra bytes are left untouched). `tpdf` is used only when
/// `dither` is true (and never for formats deeper than 24 bits or float).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn encode(
    enc: SampleEncoding,
    src: &[f32],
    dst: &mut [u8],
    tpdf: &mut Tpdf,
    dither: bool,
) {
    let dither = dither && format_ditherable(enc.effective_bits());
    let mut noise = || if dither { tpdf.next() } else { 0.0 };
    match enc {
        SampleEncoding::F32 => {
            for (d, s) in dst.chunks_exact_mut(4).zip(src) {
                d.copy_from_slice(&s.to_le_bytes());
            }
        }
        SampleEncoding::I32 => {
            for (d, s) in dst.chunks_exact_mut(4).zip(src) {
                d.copy_from_slice(&quantize(*s, 32, 0.0).to_le_bytes());
            }
        }
        SampleEncoding::I24In32 => {
            for (d, s) in dst.chunks_exact_mut(4).zip(src) {
                let q = quantize(*s, 24, noise()) << 8;
                d.copy_from_slice(&q.to_le_bytes());
            }
        }
        SampleEncoding::I24 => {
            for (d, s) in dst.chunks_exact_mut(3).zip(src) {
                let q = quantize(*s, 24, noise()).to_le_bytes();
                d.copy_from_slice(&q[..3]);
            }
        }
        SampleEncoding::I16 => {
            for (d, s) in dst.chunks_exact_mut(2).zip(src) {
                let q = quantize(*s, 16, noise()) as i16;
                d.copy_from_slice(&q.to_le_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(v: &[f64]) -> (f64, f64) {
        let n = v.len() as f64;
        let mean = v.iter().sum::<f64>() / n;
        let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
        (mean, var)
    }

    #[test]
    fn tpdf_is_triangular_with_expected_moments() {
        let mut t = Tpdf::new(1);
        let v: Vec<f64> = (0..400_000).map(|_| t.next()).collect();
        let (mean, var) = stats(&v);
        assert!(mean.abs() < 0.005, "mean {mean}");
        // Var of the sum of two U(-0.5, 0.5) = 2 * 1/12.
        assert!((var - 1.0 / 6.0).abs() < 0.005, "var {var}");
        assert!(v.iter().all(|x| x.abs() < 1.0));
        // Triangular: values near 0 are much more frequent than values near +-1.
        let center = v.iter().filter(|x| x.abs() < 0.1).count();
        let edge = v.iter().filter(|x| x.abs() > 0.9).count();
        assert!(center > 10 * edge, "center {center} edge {edge}");
    }

    #[test]
    fn dithered_16_bit_minus_100_dbfs_sine_is_not_silent_and_error_is_decorrelated() {
        let amp = 10f64.powf(-100.0 / 20.0); // 1e-5 FS = 0.33 LSB at 16 bit
        let n = 200_000usize;
        let freq = 997.0 / 48_000.0;
        let x: Vec<f32> = (0..n)
            .map(|i| (amp * (2.0 * std::f64::consts::PI * freq * i as f64).sin()) as f32)
            .collect();

        // Plain rounding: below half an LSB, the sine vanishes completely.
        let mut t = Tpdf::new(7);
        let mut plain = vec![0u8; n * 2];
        encode(SampleEncoding::I16, &x, &mut plain, &mut t, false);
        assert!(
            plain.iter().all(|b| *b == 0),
            "undithered output should be silent"
        );

        let mut bytes = vec![0u8; n * 2];
        encode(SampleEncoding::I16, &x, &mut bytes, &mut t, true);
        let q: Vec<f64> = bytes
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f64)
            .collect();
        assert!(q.iter().any(|v| *v != 0.0), "dithered output is silent");

        // Error in LSB units.
        let xs: Vec<f64> = x.iter().map(|v| *v as f64 * 32768.0).collect();
        let err: Vec<f64> = q.iter().zip(&xs).map(|(q, x)| q - x).collect();
        let (mean, var) = stats(&err);
        assert!(mean.abs() < 0.01, "error mean {mean}");
        // TPDF dither (1/6) plus the now signal-independent rounding error (1/12).
        assert!((var - 0.25).abs() < 0.015, "error variance {var}");

        // Decorrelated from the signal: the error does not follow the sine...
        let (_, xvar) = stats(&xs);
        let cov = err.iter().zip(&xs).map(|(e, x)| e * x).sum::<f64>() / n as f64;
        let corr = cov / (var.sqrt() * xvar.sqrt());
        assert!(corr.abs() < 0.02, "error/signal correlation {corr}");
        // ...nor itself (white): lag-1 autocorrelation ~ 0.
        let ac1 = err
            .windows(2)
            .map(|w| (w[0] - mean) * (w[1] - mean))
            .sum::<f64>()
            / ((n - 1) as f64 * var);
        assert!(ac1.abs() < 0.02, "error autocorrelation {ac1}");

        // And the sine survives on average (TPDF makes the quantizer linear in the mean):
        // projecting the output onto the sine recovers its amplitude.
        let proj = q.iter().zip(&xs).map(|(q, x)| q * x).sum::<f64>()
            / xs.iter().map(|x| x * x).sum::<f64>();
        assert!((proj - 1.0).abs() < 0.05, "recovered gain {proj}");
    }

    #[test]
    fn no_dither_without_processing_and_16_bit_round_trip_is_bit_exact() {
        // The gate: never dither while the renderer passes audio through untouched.
        assert!(!should_dither(DitherMode::Auto, false, Some(16)));
        assert!(should_dither(DitherMode::Auto, true, Some(16)));
        assert!(should_dither(DitherMode::Auto, true, Some(24)));
        assert!(!should_dither(DitherMode::Off, true, Some(16)));
        assert!(!should_dither(DitherMode::Auto, true, Some(32)));
        assert!(!should_dither(DitherMode::Auto, true, None));

        let shared = Arc::new(RtShared::new(1.0));
        let ctl = DitherControl::new(shared.clone(), DitherMode::Auto);
        assert!(!ctl.active(Some(16)));
        assert!(ctl.armed(Some(16)));
        shared.processing_active.store(true, Ordering::Relaxed);
        assert!(ctl.active(Some(16)));
        ctl.set_mode(DitherMode::Off);
        assert!(!ctl.active(Some(16)) && !ctl.armed(Some(16)));
        assert_eq!(ctl.mode(), DitherMode::Off);

        // Every 16-bit value, decoded the way symphonia does (s / 32768), comes back exactly,
        // both to 16-bit and (shifted) to the 24-bit formats.
        let src: Vec<f32> = (i16::MIN..=i16::MAX).map(|s| s as f32 / 32768.0).collect();
        let mut t = Tpdf::new(3);
        let mut b16 = vec![0u8; src.len() * 2];
        encode(SampleEncoding::I16, &src, &mut b16, &mut t, false);
        for (c, s) in b16.chunks_exact(2).zip(i16::MIN..=i16::MAX) {
            assert_eq!(i16::from_le_bytes([c[0], c[1]]), s);
        }
        let mut b24 = vec![0u8; src.len() * 3];
        encode(SampleEncoding::I24, &src, &mut b24, &mut t, false);
        for (c, s) in b24.chunks_exact(3).zip(i16::MIN..=i16::MAX) {
            let v = i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8;
            assert_eq!(v, (s as i32) << 8);
        }
        let mut b32 = vec![0u8; src.len() * 4];
        encode(SampleEncoding::I32, &src, &mut b32, &mut t, true); // never dithered
        for (c, s) in b32.chunks_exact(4).zip(i16::MIN..=i16::MAX) {
            assert_eq!(
                i32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                (s as i32) << 16
            );
        }
        let mut f = vec![0u8; src.len() * 4];
        encode(SampleEncoding::F32, &src, &mut f, &mut t, true); // never dithered
        for (c, s) in f.chunks_exact(4).zip(&src) {
            assert_eq!(f32::from_le_bytes([c[0], c[1], c[2], c[3]]), *s);
        }
    }

    #[test]
    fn packs_24_bit_samples_little_endian() {
        let lsb = 1.0 / 8_388_608.0f32;
        let cases: [(f32, [u8; 3]); 7] = [
            (0.0, [0, 0, 0]),
            (0.5, [0x00, 0x00, 0x40]),
            (-1.0, [0x00, 0x00, 0x80]),
            (1.0, [0xFF, 0xFF, 0x7F]), // clamped to 2^23 - 1
            (lsb, [0x01, 0x00, 0x00]),
            (-lsb, [0xFF, 0xFF, 0xFF]),
            (0x123456 as f32 * lsb, [0x56, 0x34, 0x12]),
        ];
        let src: Vec<f32> = cases.iter().map(|c| c.0).collect();
        let mut t = Tpdf::new(9);
        let mut packed = vec![0u8; src.len() * 3];
        encode(SampleEncoding::I24, &src, &mut packed, &mut t, false);
        for (c, (v, want)) in packed.chunks_exact(3).zip(&cases) {
            assert_eq!(c, want, "packed 24-bit for {v}");
        }
        let mut in32 = vec![0u8; src.len() * 4];
        encode(SampleEncoding::I24In32, &src, &mut in32, &mut t, false);
        for (c, (v, want)) in in32.chunks_exact(4).zip(&cases) {
            // Left-justified: the low byte is padding, the sample sits in the top 24 bits.
            assert_eq!(c, [0, want[0], want[1], want[2]], "24-in-32 for {v}");
        }
        // Out-of-range and NaN input never wraps.
        let mut b = [0u8; 9];
        encode(
            SampleEncoding::I24,
            &[3.0, -3.0, f32::NAN],
            &mut b,
            &mut t,
            true,
        );
        assert_eq!(&b[..3], &[0xFF, 0xFF, 0x7F]);
        assert_eq!(&b[3..6], &[0x00, 0x00, 0x80]);
        assert_eq!(&b[6..], &[0, 0, 0]);
        // Container/valid bits used to describe the formats to WASAPI.
        assert_eq!(
            (
                SampleEncoding::I24.container_bits(),
                SampleEncoding::I24.valid_bits()
            ),
            (24, 24)
        );
        assert_eq!(
            (
                SampleEncoding::I24In32.container_bits(),
                SampleEncoding::I24In32.valid_bits()
            ),
            (32, 24)
        );
    }

    #[test]
    fn dithered_24_bit_stays_within_one_lsb() {
        let src: Vec<f32> = (0..10_000)
            .map(|i| ((i as f32) * 0.001).sin() * 0.7)
            .collect();
        let mut t = Tpdf::new(11);
        let mut b = vec![0u8; src.len() * 3];
        encode(SampleEncoding::I24, &src, &mut b, &mut t, true);
        for (c, s) in b.chunks_exact(3).zip(&src) {
            let v = i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8;
            let exact = *s as f64 * 8_388_608.0;
            assert!((v as f64 - exact).abs() <= 1.5, "{v} vs {exact}");
        }
    }
}
