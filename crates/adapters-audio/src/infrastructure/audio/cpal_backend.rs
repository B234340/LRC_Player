use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
use lrc_player_ports::audio_output::{AudioFormat, AudioOutput};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

// Type aliases to reduce complex type signatures flagged by clippy
type ArcF32Vec = Arc<Vec<f32>>;
type ProducerSender = Sender<ArcF32Vec>;
type ProducerReceiver = Receiver<ArcF32Vec>;
use ringbuf::RingBuffer;
use std::sync::Mutex;
use std::thread::JoinHandle;

// Simple buffer pool for Vec<f32> reuse. Caller and writer can obtain and return Vecs
// to reduce allocations.
pub struct BufferPool {
    inner: Mutex<Vec<Vec<f32>>>,
}

impl BufferPool {
    fn new() -> Self {
        BufferPool {
            inner: Mutex::new(Vec::new()),
        }
    }

    pub fn get(&self, min_cap: usize) -> Vec<f32> {
        let mut guard = self.inner.lock().unwrap();
        if let Some(mut v) = guard.pop() {
            v.clear();
            if v.capacity() < min_cap {
                v.reserve(min_cap - v.capacity());
            }
            v
        } else {
            Vec::with_capacity(min_cap)
        }
    }

    pub fn put(&self, mut v: Vec<f32>) {
        v.clear();
        let mut guard = self.inner.lock().unwrap();
        guard.push(v);
    }
}

/// Legacy CPAL output (decoder -> channel -> writer thread -> ring -> callback).
///
/// Superseded by [`super::engine::AudioEngine`]; kept for API compatibility. Fixes applied:
/// the writer thread exits on `stop()`/drop even when the ring is full and the consumer is
/// gone (F2), the callback never allocates (I16/U16 use a preallocated scratch buffer and
/// rounding/clamping conversion), the ring is sized from the actual device format, a ~50 ms
/// fixed device buffer is requested, and the callback thread is promoted to realtime priority.
pub struct CpalOutput {
    stream: Option<cpal::Stream>,
    // Sender to a background writer thread that owns the Producer (no locks in realtime callback)
    producer_tx: Option<Sender<Arc<Vec<f32>>>>,
    writer_handle: Option<JoinHandle<()>>,
    writer_stop: Option<Arc<AtomicBool>>,
    pool: Option<Arc<BufferPool>>,
    format: Option<AudioFormat>,
    selected_device_name: Option<String>,
}

/// Writer loop: drains `rx` into the ring. Exits when `stop` is set or all senders are gone,
/// including while waiting for ring space (the consumer may already be dropped).
fn spawn_writer(
    mut producer: ringbuf::Producer<f32>,
    rx: ProducerReceiver,
    pool: Arc<BufferPool>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("lrc-producer-writer".to_string())
        .spawn(move || loop {
            if stop.load(Ordering::Acquire) {
                return;
            }
            let buf_arc = match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(b) => b,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            let buf = &*buf_arc;
            let mut offset = 0;
            while offset < buf.len() {
                if stop.load(Ordering::Acquire) {
                    return;
                }
                let pushed = producer.push_slice(&buf[offset..]);
                if pushed == 0 {
                    std::thread::sleep(Duration::from_millis(3));
                } else {
                    offset += pushed;
                }
            }
            if let Ok(vec) = Arc::try_unwrap(buf_arc) {
                pool.put(vec);
            }
        })
}

const LEGACY_SCRATCH: usize = 8192;

fn build_legacy_stream<T: cpal::SizedSample + Send + 'static>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut consumer: ringbuf::Consumer<f32>,
    convert: fn(f32) -> T,
) -> std::result::Result<cpal::Stream, cpal::BuildStreamError> {
    let mut scratch = vec![0.0f32; LEGACY_SCRATCH];
    let mut promoted = false;
    let err_fn = |e| tracing::error!("cpal stream error: {}", e);
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            if !promoted {
                promoted = true;
                super::engine::priority::promote_audio_thread();
            }
            for block in data.chunks_mut(LEGACY_SCRATCH) {
                let tmp = &mut scratch[..block.len()];
                let popped = consumer.pop_slice(tmp);
                tmp[popped..].iter_mut().for_each(|s| *s = 0.0);
                for (d, s) in block.iter_mut().zip(tmp.iter()) {
                    *d = convert(*s);
                }
            }
        },
        err_fn,
        None,
    )
}

impl CpalOutput {
    /// Start output, attempting to use `preferred` device name when provided.
    pub fn start_with_device(
        &mut self,
        _requested: AudioFormat,
        preferred: Option<String>,
    ) -> Result<AudioFormat> {
        // Restarting must not leak the previous writer.
        let _ = AudioOutput::stop(self);
        let host = cpal::default_host();
        let device_opt = if let Some(pref) = preferred.clone() {
            match host.output_devices() {
                Ok(devs) => devs
                    .into_iter()
                    .find(|d| d.name().map(|n| n == pref).unwrap_or(false)),
                Err(_) => None,
            }
        } else {
            None
        };

        let device = device_opt
            .or_else(|| host.default_output_device())
            .ok_or_else(|| anyhow!("no output device available"))?;
        self.selected_device_name = device.name().ok();

        let supported = device.default_output_config()?;
        let base: cpal::StreamConfig = supported.config();
        let sample_format = supported.sample_format();
        let rate = base.sample_rate.0;
        let wanted = rate / 20; // ~50 ms
        let fixed = match supported.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => wanted.clamp(*min, (*max).max(*min)),
            cpal::SupportedBufferSize::Unknown => wanted,
        };

        let pool = Arc::new(BufferPool::new());
        let mut last_err: Option<anyhow::Error> = None;
        for buffer_size in [cpal::BufferSize::Fixed(fixed), cpal::BufferSize::Default] {
            let stream_config = cpal::StreamConfig {
                buffer_size,
                ..base.clone()
            };
            // ~1 s of audio at the ACTUAL device format.
            let cap = (rate as usize)
                .saturating_mul(stream_config.channels as usize)
                .clamp(4096, 1 << 20);
            let (prod, cons) = RingBuffer::<f32>::new(cap).split();
            let built = match sample_format {
                cpal::SampleFormat::F32 => {
                    build_legacy_stream::<f32>(&device, &stream_config, cons, |v| v)
                }
                cpal::SampleFormat::I16 => {
                    build_legacy_stream::<i16>(&device, &stream_config, cons, |v| {
                        (v.clamp(-1.0, 1.0) * 32767.0).round() as i16
                    })
                }
                cpal::SampleFormat::U16 => {
                    build_legacy_stream::<u16>(&device, &stream_config, cons, |v| {
                        ((v.clamp(-1.0, 1.0) * 32767.0).round() as i32 + 32768) as u16
                    })
                }
                _ => return Err(anyhow!("unsupported sample format")),
            };
            let stream = match built {
                Ok(s) => s,
                Err(e) => {
                    last_err = Some(anyhow!("{e}"));
                    continue;
                }
            };
            let (tx, rx): (ProducerSender, ProducerReceiver) = bounded(16);
            let stop = Arc::new(AtomicBool::new(false));
            let writer = spawn_writer(prod, rx, pool.clone(), stop.clone())?;
            stream.play()?;
            self.stream = Some(stream);
            self.producer_tx = Some(tx);
            self.writer_handle = Some(writer);
            self.writer_stop = Some(stop);
            self.pool = Some(pool);
            let actual_format = AudioFormat {
                sample_rate: rate,
                channels: stream_config.channels,
                sample_format: match sample_format {
                    cpal::SampleFormat::I16 => lrc_player_ports::audio_output::SampleFormat::I16,
                    _ => lrc_player_ports::audio_output::SampleFormat::F32,
                },
            };
            self.format = Some(actual_format);
            return Ok(actual_format);
        }
        Err(last_err.unwrap_or_else(|| anyhow!("could not open output stream")))
    }
    pub fn new() -> Self {
        CpalOutput {
            stream: None,
            producer_tx: None,
            writer_handle: None,
            writer_stop: None,
            pool: None,
            format: None,
            selected_device_name: None,
        }
    }

    /// List available output device names for the default host.
    pub fn available_device_names() -> Vec<String> {
        let host = cpal::default_host();
        match host.output_devices() {
            Ok(devs) => devs.filter_map(|d| d.name().ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Return the name of the default output device if available.
    pub fn default_device_name() -> Option<String> {
        let host = cpal::default_host();
        host.default_output_device().and_then(|d| d.name().ok())
    }

    /// Return a clone of the producer sender so callers can send Arc<Vec<f32>> directly.
    pub fn producer_sender(&self) -> Option<Sender<Arc<Vec<f32>>>> {
        self.producer_tx.clone()
    }

    /// Return a handle to the internal buffer pool (if started) so callers can obtain reusable Vecs.
    pub fn pool_handle(&self) -> Option<Arc<BufferPool>> {
        self.pool.clone()
    }
}

impl AudioOutput for CpalOutput {
    fn start(&mut self, format: AudioFormat) -> Result<AudioFormat> {
        self.start_with_device(format, None)
    }

    fn stop(&mut self) -> Result<()> {
        // Signal the writer first: it may be waiting for ring space that will never come
        // once the stream (and with it the consumer) is dropped.
        if let Some(stop) = self.writer_stop.take() {
            stop.store(true, Ordering::Release);
        }
        self.stream = None;
        self.producer_tx = None;
        if let Some(h) = self.writer_handle.take() {
            let _ = h.join();
        }
        Ok(())
    }

    fn write_f32(&mut self, data: &[f32]) -> Result<usize> {
        let tx = self
            .producer_tx
            .as_ref()
            .ok_or_else(|| anyhow!("output not started"))?;
        // try to reuse Vec from pool if available to avoid allocation
        let arc = if let Some(pool) = &self.pool {
            let mut buf = pool.get(data.len());
            buf.extend_from_slice(data);
            Arc::new(buf)
        } else {
            Arc::new(data.to_vec())
        };
        match tx.try_send(arc) {
            Ok(()) => Ok(data.len()),
            Err(err) => {
                // channel full or disconnected
                if err.is_full() {
                    Ok(0)
                } else {
                    Err(anyhow!("producer channel disconnected"))
                }
            }
        }
    }
}

impl CpalOutput {
    /// Send an already-allocated `Arc<Vec<f32>>` to the producer writer thread.
    /// This avoids an extra allocation/copy when the caller can reuse buffers.
    pub fn write_f32_arc(&mut self, data: Arc<Vec<f32>>) -> Result<usize> {
        let tx = self
            .producer_tx
            .as_ref()
            .ok_or_else(|| anyhow!("output not started"))?;
        let len = data.len();
        match tx.try_send(data) {
            Ok(()) => Ok(len),
            Err(err) => {
                if err.is_full() {
                    Ok(0)
                } else {
                    Err(anyhow!("producer channel disconnected"))
                }
            }
        }
    }

    /// Get a reusable Vec<f32> from the pool (or a new one) with at least `min_capacity`.
    /// Caller can fill the Vec and send it via `write_f32_arc(Arc::new(vec))`.
    pub fn get_buffer(&self, min_capacity: usize) -> Vec<f32> {
        if let Some(pool) = &self.pool {
            pool.get(min_capacity)
        } else {
            Vec::with_capacity(min_capacity)
        }
    }

    /// Return the selected device name if available (after `start_with_device`).
    pub fn selected_device_name(&self) -> Option<String> {
        self.selected_device_name.clone()
    }
}

impl Default for CpalOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for CpalOutput {
    fn drop(&mut self) {
        let _ = AudioOutput::stop(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F2 regression: with the consumer dropped and the ring full, the writer used to spin
    /// forever (and `stop()` hung in `join`). It must exit once stop is signalled.
    #[test]
    fn writer_exits_on_stop_with_full_ring_and_dropped_consumer() {
        let (prod, cons) = RingBuffer::<f32>::new(64).split();
        drop(cons);
        let (tx, rx) = bounded::<Arc<Vec<f32>>>(4);
        let stop = Arc::new(AtomicBool::new(false));
        let h = spawn_writer(prod, rx, Arc::new(BufferPool::new()), stop.clone()).unwrap();
        tx.send(Arc::new(vec![0.0; 1000])).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert!(
            !h.is_finished(),
            "writer should be blocked on the full ring"
        );
        // The sender is still alive (as the decoder's clone would be): only the flag stops it.
        stop.store(true, Ordering::Release);
        let t = std::time::Instant::now();
        while !h.is_finished() {
            assert!(t.elapsed() < Duration::from_secs(2), "writer leaked");
            std::thread::sleep(Duration::from_millis(5));
        }
        h.join().unwrap();
        drop(tx);
    }
}
