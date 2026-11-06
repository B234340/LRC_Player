//! cpal output: device selection, stream configuration (fixed ~50 ms buffer), sample-format
//! conversion without allocation, callback-thread priority promotion.

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::Sender;

use super::priority;
use super::renderer::Renderer;
use super::DeviceInfo;

/// Names of all output devices of the default host.
pub(crate) fn output_device_names() -> Vec<String> {
    let host = cpal::default_host();
    match host.output_devices() {
        Ok(devs) => devs.filter_map(|d| d.name().ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Name of the current system default output device.
pub(crate) fn default_output_device_name() -> Option<String> {
    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
}

/// Conversion from the renderer's f32 to a device sample type (rounded, clamped).
trait OutSample: cpal::SizedSample + Send + 'static {
    fn from_f32(v: f32) -> Self;
}

impl OutSample for f32 {
    #[inline]
    fn from_f32(v: f32) -> Self {
        v
    }
}
impl OutSample for f64 {
    #[inline]
    fn from_f32(v: f32) -> Self {
        v as f64
    }
}
impl OutSample for i16 {
    #[inline]
    fn from_f32(v: f32) -> Self {
        (v.clamp(-1.0, 1.0) * 32767.0).round() as i16
    }
}
impl OutSample for i32 {
    #[inline]
    fn from_f32(v: f32) -> Self {
        (v.clamp(-1.0, 1.0) as f64 * 2_147_483_647.0).round() as i32
    }
}
impl OutSample for u16 {
    #[inline]
    fn from_f32(v: f32) -> Self {
        ((v.clamp(-1.0, 1.0) * 32767.0).round() as i32 + 32768) as u16
    }
}
impl OutSample for i8 {
    #[inline]
    fn from_f32(v: f32) -> Self {
        (v.clamp(-1.0, 1.0) * 127.0).round() as i8
    }
}
impl OutSample for u8 {
    #[inline]
    fn from_f32(v: f32) -> Self {
        ((v.clamp(-1.0, 1.0) * 127.0).round() as i32 + 128) as u8
    }
}

/// Scratch size (frames) for non-f32 device formats; allocated once, outside the callback.
const SCRATCH_FRAMES: usize = 2048;

fn build_typed<T: OutSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut renderer: Renderer,
    err_tx: Sender<String>,
) -> std::result::Result<cpal::Stream, cpal::BuildStreamError> {
    let ch = renderer.channels();
    let rate = config.sample_rate.0.max(1) as u64;
    let is_f32 = std::any::TypeId::of::<T>() == std::any::TypeId::of::<f32>();
    let mut scratch: Vec<f32> = if is_f32 {
        Vec::new()
    } else {
        vec![0.0; SCRATCH_FRAMES * ch]
    };
    let mut promoted = false;
    let data_cb = move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
        if !promoted {
            promoted = true;
            priority::promote_audio_thread();
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
        let mut offset = 0usize;
        while offset < data.len() {
            let n = (data.len() - offset).min(scratch.len());
            let lat = latency_ns + (offset / ch) as u64 * 1_000_000_000 / rate;
            renderer.render(&mut scratch[..n], lat);
            for (d, s) in data[offset..offset + n].iter_mut().zip(&scratch[..n]) {
                *d = T::from_f32(*s);
            }
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
) -> Result<cpal::Stream> {
    use cpal::SampleFormat as F;
    let r = match format {
        F::F32 => build_typed::<f32>(device, config, renderer, err_tx),
        F::F64 => build_typed::<f64>(device, config, renderer, err_tx),
        F::I16 => build_typed::<i16>(device, config, renderer, err_tx),
        F::I32 => build_typed::<i32>(device, config, renderer, err_tx),
        F::U16 => build_typed::<u16>(device, config, renderer, err_tx),
        F::I8 => build_typed::<i8>(device, config, renderer, err_tx),
        F::U8 => build_typed::<u8>(device, config, renderer, err_tx),
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
) -> Result<(cpal::Stream, DeviceInfo)> {
    let host = cpal::default_host();
    let mut fell_back = false;
    let device = match preferred {
        Some(name) => {
            let found = host
                .output_devices()
                .ok()
                .and_then(|mut devs| devs.find(|d| d.name().map(|n| n == name).unwrap_or(false)));
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
        match build_stream(&device, &config, format, renderer, err_tx.clone()) {
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
