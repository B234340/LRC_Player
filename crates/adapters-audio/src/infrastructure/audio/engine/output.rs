//! cpal output: device selection, stream configuration (fixed ~50 ms buffer), sample-format
//! conversion without allocation, callback-thread priority promotion.

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::Sender;

use super::dither::{format_ditherable, quantize, DitherControl, Tpdf};
use super::priority;
use super::renderer::Renderer;
use super::DeviceInfo;

/// cpal's default host is ALSA on these targets. ALSA's "default device" is the constant
/// `default` PCM (the sound server, if any, follows the system default by itself), so polling
/// it for changes is pointless, and probing devices means opening PCMs, which is expensive
/// (config re-evaluation, sound-server connections, stderr noise on failure).
pub(crate) const ALSA_HOST: bool = cfg!(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
));

/// Names of all output devices of the default host.
///
/// On ALSA the names come from the ALSA name hints (playback-capable PCMs), which needs no
/// PCM to be opened; cpal's enumeration would open every PCM of the system, twice.
pub(crate) fn output_device_names() -> Vec<String> {
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd"
    ))]
    {
        alsa_hint_names()
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd"
    )))]
    {
        let host = cpal::default_host();
        match host.output_devices() {
            Ok(devs) => devs.filter_map(|d| d.name().ok()).collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// Playback-capable PCM names from the ALSA configuration/card hints (the same names cpal
/// reports, minus `null`), without opening any PCM.
#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
))]
fn alsa_hint_names() -> Vec<String> {
    let Ok(hints) = alsa::device_name::HintIter::new_str(None, "pcm") else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    for hint in hints {
        let Some(name) = hint.name else { continue };
        if name == "null" || hint.direction == Some(alsa::Direction::Capture) {
            continue;
        }
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Whether `name` is currently a known output device (cheap on every host: on ALSA it reads
/// name hints only).
pub(crate) fn output_device_exists(name: &str) -> bool {
    output_device_names().iter().any(|n| n == name)
}

/// Name of the current system default output device.
pub(crate) fn default_output_device_name() -> Option<String> {
    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
}

/// Conversion from the renderer's f32 to a device sample type (rounded, clamped, optionally
/// dithered). Integer scales are 2^(bits-1), the inverse of symphonia's int -> f32 conversion,
/// so undithered integer sources round-trip bit-exactly.
trait OutSample: cpal::SizedSample + Send + 'static {
    /// Integer resolution (`None` for float formats).
    const BITS: Option<u32>;
    /// `noise_lsb` is TPDF dither in LSB units (0.0 when not dithering).
    fn from_f32(v: f32, noise_lsb: f64) -> Self;
}

impl OutSample for f32 {
    const BITS: Option<u32> = None;
    #[inline]
    fn from_f32(v: f32, _: f64) -> Self {
        v
    }
}
impl OutSample for f64 {
    const BITS: Option<u32> = None;
    #[inline]
    fn from_f32(v: f32, _: f64) -> Self {
        v as f64
    }
}
impl OutSample for i16 {
    const BITS: Option<u32> = Some(16);
    #[inline]
    fn from_f32(v: f32, noise_lsb: f64) -> Self {
        quantize(v, 16, noise_lsb) as i16
    }
}
impl OutSample for i32 {
    const BITS: Option<u32> = Some(32);
    #[inline]
    fn from_f32(v: f32, noise_lsb: f64) -> Self {
        quantize(v, 32, noise_lsb)
    }
}
impl OutSample for u16 {
    const BITS: Option<u32> = Some(16);
    #[inline]
    fn from_f32(v: f32, noise_lsb: f64) -> Self {
        (quantize(v, 16, noise_lsb) + 32768) as u16
    }
}
impl OutSample for i8 {
    const BITS: Option<u32> = Some(8);
    #[inline]
    fn from_f32(v: f32, noise_lsb: f64) -> Self {
        quantize(v, 8, noise_lsb) as i8
    }
}
impl OutSample for u8 {
    const BITS: Option<u32> = Some(8);
    #[inline]
    fn from_f32(v: f32, noise_lsb: f64) -> Self {
        (quantize(v, 8, noise_lsb) + 128) as u8
    }
}

/// Integer resolution of a `DeviceInfo::sample_format` name (cpal's `SampleFormat` debug
/// names and the exclusive output's names); `None` for float or unknown formats.
pub(crate) fn format_effective_bits(name: &str) -> Option<u32> {
    match name {
        "I8" | "U8" => Some(8),
        "I16" | "U16" => Some(16),
        "I24" | "I24in32" => Some(24),
        "I32" | "U32" => Some(32),
        _ => None,
    }
}

/// Converts one block of renderer output to device samples, dithering when `dither` is set
/// (and the format is an integer format of at most 24 bits).
fn convert_block<T: OutSample>(src: &[f32], dst: &mut [T], tpdf: &mut Tpdf, dither: bool) {
    if dither && format_ditherable(T::BITS) {
        for (d, s) in dst.iter_mut().zip(src) {
            *d = T::from_f32(*s, tpdf.next());
        }
    } else {
        for (d, s) in dst.iter_mut().zip(src) {
            *d = T::from_f32(*s, 0.0);
        }
    }
}

/// Scratch size (frames) for non-f32 device formats; allocated once, outside the callback.
const SCRATCH_FRAMES: usize = 2048;

fn build_typed<T: OutSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut renderer: Renderer,
    err_tx: Sender<String>,
    dither: DitherControl,
) -> std::result::Result<cpal::Stream, cpal::BuildStreamError> {
    let ch = renderer.channels();
    let rate = config.sample_rate.0.max(1) as u64;
    let is_f32 = std::any::TypeId::of::<T>() == std::any::TypeId::of::<f32>();
    let mut scratch: Vec<f32> = if is_f32 {
        Vec::new()
    } else {
        vec![0.0; SCRATCH_FRAMES * ch]
    };
    let mut prio: Option<priority::AudioThreadGuard> = None;
    let mut tpdf = Tpdf::seeded_from_clock();
    let data_cb = move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
        if prio.is_none() {
            prio = Some(priority::promote_audio_thread());
        }
        let ts = info.timestamp();
        let latency_ns = ts
            .playback
            .duration_since(&ts.callback)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        if is_f32 {
            // SAFETY: T is f32 (checked via TypeId above), so the slice can be reinterpreted.
            let out = unsafe {
                std::slice::from_raw_parts_mut(data.as_mut_ptr() as *mut f32, data.len())
            };
            renderer.render(out, latency_ns);
            return;
        }
        let dither_on = dither.active(T::BITS);
        let mut offset = 0usize;
        while offset < data.len() {
            let n = (data.len() - offset).min(scratch.len());
            let lat = latency_ns + (offset / ch) as u64 * 1_000_000_000 / rate;
            renderer.render(&mut scratch[..n], lat);
            convert_block(
                &scratch[..n],
                &mut data[offset..offset + n],
                &mut tpdf,
                dither_on,
            );
            offset += n;
        }
    };
    let err_cb = move |e: cpal::StreamError| {
        let _ = err_tx.try_send(e.to_string());
    };
    device.build_output_stream(config, data_cb, err_cb, None)
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    renderer: Renderer,
    err_tx: Sender<String>,
    dither: DitherControl,
) -> Result<cpal::Stream> {
    use cpal::SampleFormat as F;
    let (d, c, r, e, t) = (device, config, renderer, err_tx, dither);
    let r = match format {
        F::F32 => build_typed::<f32>(d, c, r, e, t),
        F::F64 => build_typed::<f64>(d, c, r, e, t),
        F::I16 => build_typed::<i16>(d, c, r, e, t),
        F::I32 => build_typed::<i32>(d, c, r, e, t),
        F::U16 => build_typed::<u16>(d, c, r, e, t),
        F::I8 => build_typed::<i8>(d, c, r, e, t),
        F::U8 => build_typed::<u8>(d, c, r, e, t),
        other => return Err(anyhow!("unsupported device sample format {other:?}")),
    };
    r.map_err(|e| anyhow!("{e}"))
}

/// Opens the preferred device (or the default when `preferred` is None / not found) and
/// starts a stream driven by a renderer obtained from `make_renderer(rate, channels)`.
/// `make_renderer` may be called more than once (fixed-buffer attempt, then fallback).
pub(crate) fn open_cpal(
    preferred: Option<&str>,
    buffer_ms: u32,
    make_renderer: &mut dyn FnMut(u32, u16) -> Renderer,
    err_tx: Sender<String>,
    dither: &DitherControl,
) -> Result<(cpal::Stream, DeviceInfo)> {
    let host = cpal::default_host();
    let mut fell_back = false;
    let device = match preferred {
        Some(name) => {
            // cpal's ALSA enumeration opens PCMs one by one until the name matches: skip it
            // when the name hints already say the device is not there.
            let found = if ALSA_HOST && !output_device_exists(name) {
                None
            } else {
                host.output_devices().ok().and_then(|mut devs| {
                    devs.find(|d| d.name().map(|n| n == name).unwrap_or(false))
                })
            };
            match found {
                Some(d) => Some(d),
                None => {
                    fell_back = true;
                    host.default_output_device()
                }
            }
        }
        None => host.default_output_device(),
    }
    .ok_or_else(|| anyhow!("no audio output device available"))?;
    let name = device
        .name()
        .unwrap_or_else(|_| "<unknown device>".to_string());
    if fell_back {
        tracing::warn!(
            "preferred output device {:?} not found; using default {name:?}",
            preferred
        );
    }
    let supported = device
        .default_output_config()
        .map_err(|e| anyhow!("{name}: {e}"))?;
    let format = supported.sample_format();
    let base: cpal::StreamConfig = supported.config();
    let rate = base.sample_rate.0;
    let channels = base.channels;
    let wanted = (rate as u64 * buffer_ms.max(5) as u64 / 1000) as u32;
    let fixed = match supported.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => wanted.clamp(*min, (*max).max(*min)),
        cpal::SupportedBufferSize::Unknown => wanted,
    };

    let mut attempts: Vec<cpal::BufferSize> = Vec::with_capacity(2);
    if buffer_ms > 0 {
        attempts.push(cpal::BufferSize::Fixed(fixed));
    }
    attempts.push(cpal::BufferSize::Default);
    let mut last_err = None;
    for buffer_size in attempts {
        let config = cpal::StreamConfig {
            channels,
            sample_rate: base.sample_rate,
            buffer_size,
        };
        let renderer = make_renderer(rate, channels);
        let built = build_stream(
            &device,
            &config,
            format,
            renderer,
            err_tx.clone(),
            dither.clone(),
        );
        match built {
            Ok(stream) => {
                if let Err(e) = stream.play() {
                    last_err = Some(anyhow!("{name}: {e}"));
                    continue;
                }
                let buffer_frames = match buffer_size {
                    cpal::BufferSize::Fixed(n) => Some(n),
                    cpal::BufferSize::Default => None,
                };
                tracing::info!(
                    "audio output: {name} {rate} Hz {channels} ch {format:?}, buffer {:?}",
                    buffer_frames
                );
                return Ok((
                    stream,
                    DeviceInfo {
                        name,
                        sample_rate: rate,
                        channels,
                        sample_format: format!("{format:?}"),
                        buffer_frames,
                        follows_default: preferred.is_none() || fell_back,
                        exclusive: false,
                        dither: dither.armed(format_effective_bits(&format!("{format:?}"))),
                    },
                ));
            }
            Err(e) => {
                tracing::warn!("building output stream ({buffer_size:?}) failed: {e}");
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("could not open {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_sources_round_trip_bit_exactly_without_dither() {
        let mut t = Tpdf::new(5);
        let src: Vec<f32> = (i16::MIN..=i16::MAX).map(|s| s as f32 / 32768.0).collect();
        let mut out = vec![0i16; src.len()];
        convert_block(&src, &mut out, &mut t, false);
        assert!(out.iter().zip(i16::MIN..=i16::MAX).all(|(o, s)| *o == s));
        let mut u = vec![0u16; src.len()];
        convert_block(&src, &mut u, &mut t, false);
        assert!(u
            .iter()
            .zip(i16::MIN..=i16::MAX)
            .all(|(o, s)| *o as i32 == s as i32 + 32768));
        // 32-bit integer output is never dithered and holds every f32 sample exactly.
        let mut w = vec![0i32; src.len()];
        convert_block(&src, &mut w, &mut t, true);
        assert!(w
            .iter()
            .zip(i16::MIN..=i16::MAX)
            .all(|(o, s)| *o == (s as i32) << 16));
        let src8: Vec<f32> = (i8::MIN..=i8::MAX).map(|s| s as f32 / 128.0).collect();
        let mut b = vec![0u8; src8.len()];
        convert_block(&src8, &mut b, &mut t, false);
        assert!(b.iter().zip(0u8..=255).all(|(o, s)| *o == s));
        // Full scale clamps instead of wrapping.
        let mut c = [0i16; 3];
        convert_block(&[1.5, -1.5, 1.0], &mut c, &mut t, false);
        assert_eq!(c, [i16::MAX, i16::MIN, i16::MAX]);
    }

    #[test]
    fn cpal_integer_formats_dither_only_when_asked() {
        let mut t = Tpdf::new(5);
        // 0.3 LSB at 16 bit: silent without dither, audible (on average 0.3) with it.
        let src = vec![0.3f32 / 32768.0; 50_000];
        let mut out = vec![0i16; src.len()];
        convert_block(&src, &mut out, &mut t, false);
        assert!(out.iter().all(|s| *s == 0));
        convert_block(&src, &mut out, &mut t, true);
        let mean = out.iter().map(|s| *s as f64).sum::<f64>() / out.len() as f64;
        assert!((mean - 0.3).abs() < 0.02, "mean {mean}");
        assert!(out.iter().all(|s| s.abs() <= 2));
        // Float output is untouched even when dither is requested.
        let mut f = vec![0f32; src.len()];
        convert_block(&src, &mut f, &mut t, true);
        assert_eq!(f, src);
    }

    #[test]
    fn effective_bits_of_format_names() {
        assert_eq!(format_effective_bits("I16"), Some(16));
        assert_eq!(format_effective_bits("U8"), Some(8));
        assert_eq!(format_effective_bits("I24in32"), Some(24));
        assert_eq!(format_effective_bits("I32"), Some(32));
        assert_eq!(format_effective_bits("F32"), None);
    }
}
