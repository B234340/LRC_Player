use crate::ports::audio_output::{AudioFormat, AudioOutput};
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::Arc;

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

/// CPAL-based output using a ring buffer for lower-latency, less-allocation playback.
/// Uses a small Mutex only to guard the `Producer`/`Consumer` handles when accessed from
/// the non-realtime and realtime threads respectively. This is a pragmatic compromise
/// for demo purposes; a fully lock-free setup can be implemented if needed.
pub struct CpalOutput {
    stream: Option<cpal::Stream>,
    // Sender to a background writer thread that owns the Producer (no locks in realtime callback)
    producer_tx: Option<Sender<Arc<Vec<f32>>>>,
    writer_handle: Option<JoinHandle<()>>,
    pool: Option<Arc<BufferPool>>,
    format: Option<AudioFormat>,
    selected_device_name: Option<String>,
}

impl CpalOutput {
    /// Start output, attempting to use `preferred` device name when provided.
    pub fn start_with_device(
        &mut self,
        format: AudioFormat,
        preferred: Option<String>,
    ) -> Result<AudioFormat> {
        let host = cpal::default_host();
        // try to find device by preferred name if provided
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
        // record selected device name when possible
        self.selected_device_name = device.name().ok();

        // attempt to use the device's default output config
        let supported = device.default_output_config()?;
        let mut stream_config: cpal::StreamConfig = supported.config();

        // prefer requested sample rate/channels when possible
        stream_config.channels = format.channels;
        stream_config.sample_rate = cpal::SampleRate(format.sample_rate);

        let sample_format = supported.sample_format();

        // create a ring buffer sized to ~1 second of audio at requested rate (clamped)
        let cap_frames = (format.sample_rate as usize).saturating_mul(format.channels as usize);
        // clamp capacity to reasonable bounds
        let cap = cap_frames.clamp(4096, 262_144);
        let rb = RingBuffer::<f32>::new(cap);
        let (prod, cons) = rb.split();

        let err_fn = |e| tracing::error!("cpal stream error: {}", e);

        // move `cons` directly into the audio callback to avoid locking in realtime thread
        let cons_cb = cons;

        // create buffer pool and a bounded channel to send write requests (Arc<Vec<f32>>) to the
        // background writer that owns `prod`. Small capacity avoids unbounded memory growth while
        // remaining mostly non-blocking for the caller via `try_send`.
        let pool = Arc::new(BufferPool::new());
        let (tx, rx): (ProducerSender, ProducerReceiver) = bounded(16);

        // spawn writer thread that owns `prod` and drains the channel, pushing into ring buffer
        let pool_clone = pool.clone();
        let writer = std::thread::Builder::new()
            .name("lrc-producer-writer".to_string())
            .spawn(move || {
                let mut producer = prod;
                // drain until sender is dropped
                while let Ok(buf_arc) = rx.recv() {
                    let buf = &*buf_arc;
                    // try to push entire buffer; if only partial fits, we push what we can and drop rest
                    let mut offset = 0;
                    while offset < buf.len() {
                        let pushed = producer.push_slice(&buf[offset..]);
                        if pushed == 0 {
                            // ring buffer full; yield briefly to allow consumer to catch up
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        } else {
                            offset += pushed;
                        }
                    }

                    // attempt to reclaim Vec from Arc to return to pool
                    if let Ok(vec) = Arc::try_unwrap(buf_arc) {
                        pool_clone.put(vec);
                    }
                }
            })?;

        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let mut consumer = cons_cb;
                let cb = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    // pop as many samples as available into data
                    let popped = consumer.pop_slice(data);
                    if popped < data.len() {
                        // zero-fill remainder
                        for s in data.iter_mut().skip(popped) {
                            *s = 0.0;
                        }
                    }
                };
                device.build_output_stream(&stream_config, cb, err_fn, None)?
            }
            cpal::SampleFormat::I16 => {
                let mut consumer = cons_cb;
                let cb = move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    let mut tmp = vec![0.0f32; data.len()];
                    let popped = consumer.pop_slice(&mut tmp);
                    for (i, sample) in data.iter_mut().enumerate() {
                        let v = if i < popped { tmp[i] } else { 0.0 };
                        *sample = (v * i16::MAX as f32) as i16;
                    }
                };
                device.build_output_stream(&stream_config, cb, err_fn, None)?
            }
            cpal::SampleFormat::U16 => {
                let mut consumer = cons_cb;
                let cb = move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                    let mut tmp = vec![0.0f32; data.len()];
                    let popped = consumer.pop_slice(&mut tmp);
                    for (i, sample) in data.iter_mut().enumerate() {
                        let v = if i < popped { tmp[i] } else { 0.0 };
                        let scaled = ((v * 0.5 + 0.5) * u16::MAX as f32) as u16;
                        *sample = scaled;
                    }
                };
                device.build_output_stream(&stream_config, cb, err_fn, None)?
            }
            _ => return Err(anyhow!("unsupported sample format")),
        };

        stream.play()?;
        self.stream = Some(stream);
        self.producer_tx = Some(tx);
        self.writer_handle = Some(writer);
        self.pool = Some(pool);
        self.format = Some(format);
        Ok(self.format.clone().unwrap())
    }
    pub fn new() -> Self {
        CpalOutput {
            stream: None,
            producer_tx: None,
            writer_handle: None,
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

    // Default impl will be provided at module scope below.

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
        // drop stream to stop
        self.stream = None;
        // drop the sender to signal writer to exit, then join the thread
        if let Some(tx) = self.producer_tx.take() {
            drop(tx);
        }
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
