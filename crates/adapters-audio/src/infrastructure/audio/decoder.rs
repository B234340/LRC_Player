//! Legacy decode helpers (kept for API compatibility with the pre-engine GUI path).
//!
//! Both functions now sit on the engine's `TrackSource`, so they get: container seeking
//! (`start_ms` no longer decodes from the start of the file), packet filtering by track id,
//! format taken from each decoded buffer, and gapless trimming.
//!
//! Stop handling (F2): a disconnected stop channel counts as "stop" (dropping the sender is a
//! broadcast), sends to the producer use timeouts so a stop is noticed even when the consumer
//! side is stalled, and a stop during the start-offset skip ends the thread instead of falling
//! through into playback.

use anyhow::Result;
use crossbeam_channel::{Receiver, SendTimeoutError, Sender, TryRecvError};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::engine::source::TrackSource;
use super::stream_adapter::StreamConverter;

fn stop_requested(stop_rx: &Receiver<()>) -> bool {
    !matches!(stop_rx.try_recv(), Err(TryRecvError::Empty))
}

/// Sends `buf`, waiting for space but giving up when a stop is requested or the receiver is
/// gone. Returns false if the caller should stop.
fn send_or_stop(
    tx: &Sender<Arc<Vec<f32>>>,
    mut buf: Arc<Vec<f32>>,
    stop_rx: &Receiver<()>,
) -> bool {
    loop {
        match tx.send_timeout(buf, Duration::from_millis(50)) {
            Ok(()) => return true,
            Err(SendTimeoutError::Disconnected(_)) => return false,
            Err(SendTimeoutError::Timeout(b)) => {
                if stop_requested(stop_rx) {
                    return false;
                }
                buf = b;
            }
        }
    }
}

/// Spawn a decoder thread that decodes `path`, converts/resamples to the target format,
/// and sends `Arc<Vec<f32>>` buffers directly to the provided `producer_tx`. Buffers are
/// obtained from `pool` when provided to avoid allocations.
#[allow(clippy::too_many_arguments)]
pub fn spawn_decode_to_producer(
    path: PathBuf,
    chunk_frames: usize,
    target_sample_rate: u32,
    target_channels: u16,
    producer_tx: Sender<Arc<Vec<f32>>>,
    pool: Option<Arc<crate::infrastructure::audio::cpal_backend::BufferPool>>,
    taps: usize,
    start_ms: Option<u64>,
    stop_rx: Receiver<()>,
) -> std::thread::JoinHandle<anyhow::Result<()>> {
    std::thread::Builder::new()
        .name("lrc-legacy-decoder".into())
        .spawn(move || -> Result<()> {
            let mut src = TrackSource::open(&path)?;
            if let Some(ms) = start_ms.filter(|&ms| ms > 0) {
                src.seek_ms(ms)?;
            }
            if stop_requested(&stop_rx) {
                return Ok(());
            }
            let chunk_frames = chunk_frames.max(64);
            let mut conv: Option<(StreamConverter, usize, u32)> = None;
            let mut acc: Vec<f32> = Vec::new();
            let get_buf = |n: usize| match &pool {
                Some(p) => p.get(n),
                None => Vec::with_capacity(n),
            };
            let out_cap = |frames: usize| (frames * target_channels as usize * 2).max(1024);

            loop {
                if stop_requested(&stop_rx) {
                    return Ok(());
                }
                let fmt = match src.decode_next()? {
                    Some(f) => f,
                    None => break,
                };
                let changed = conv
                    .as_ref()
                    .is_none_or(|(_, ch, rate)| *ch != fmt.channels || *rate != fmt.rate);
                if changed {
                    // Flush what is buffered with the old format first.
                    if let Some((mut c, ch, rate)) = conv.take() {
                        let mut out = get_buf(out_cap(acc.len() / ch.max(1)));
                        c.process(&acc, ch as u16, rate, &mut out);
                        let mut tail = Vec::new();
                        c.flush(&mut tail);
                        out.extend_from_slice(&tail);
                        acc.clear();
                        if !out.is_empty() && !send_or_stop(&producer_tx, Arc::new(out), &stop_rx) {
                            return Ok(());
                        }
                    }
                    conv = Some((
                        StreamConverter::new(
                            fmt.rate,
                            fmt.channels as u16,
                            target_sample_rate,
                            target_channels,
                            taps,
                        ),
                        fmt.channels,
                        fmt.rate,
                    ));
                }
                let (c, ch, rate) = conv.as_mut().expect("converter");
                acc.extend_from_slice(src.block());
                let chunk_len = chunk_frames * *ch;
                let mut start = 0;
                while acc.len() - start >= chunk_len {
                    let mut out = get_buf(out_cap(chunk_frames));
                    c.process(&acc[start..start + chunk_len], *ch as u16, *rate, &mut out);
                    start += chunk_len;
                    if !send_or_stop(&producer_tx, Arc::new(out), &stop_rx) {
                        return Ok(());
                    }
                }
                acc.drain(..start);
            }

            // Natural end: send the remainder and the converter tail.
            if let Some((mut c, ch, rate)) = conv {
                let mut out = get_buf(out_cap(acc.len() / ch.max(1) + 16));
                c.process(&acc, ch as u16, rate, &mut out);
                let mut tail = Vec::new();
                c.flush(&mut tail);
                out.extend_from_slice(&tail);
                if !out.is_empty() {
                    let _ = send_or_stop(&producer_tx, Arc::new(out), &stop_rx);
                }
            }
            Ok(())
        })
        .expect("spawn decoder thread")
}

/// Chunk of decoded interleaved f32 samples.
pub struct DecodedChunk {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Spawn a streaming decoder for `path` that sends interleaved f32 chunks of approximately
/// `chunk_frames` frames (per channel) over an unbounded `tokio` receiver. The returned
/// receiver yields `Result<DecodedChunk, anyhow::Error>` asynchronously until the stream ends
/// or an error occurs. The thread exits early when the receiver is dropped.
pub fn spawn_decode_stream(
    path: PathBuf,
    chunk_frames: usize,
) -> tokio::sync::mpsc::UnboundedReceiver<Result<DecodedChunk>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    std::thread::spawn(move || {
        let res: Result<()> = (|| {
            let mut src = TrackSource::open(&path)?;
            let mut acc: Vec<f32> = Vec::new();
            let mut cur: Option<(u32, usize)> = None;
            let chunk_frames = chunk_frames.max(1);
            loop {
                let fmt = src.decode_next()?;
                let flush_all = match (&fmt, cur) {
                    (None, _) => true,
                    (Some(f), Some((r, c))) => f.rate != r || f.channels != c,
                    (Some(_), None) => false,
                };
                if let Some((rate, ch)) = cur {
                    let target = chunk_frames * ch;
                    let mut start = 0;
                    while acc.len() - start >= target || (flush_all && start < acc.len()) {
                        let end = (start + target).min(acc.len());
                        let dc = DecodedChunk {
                            samples: acc[start..end].to_vec(),
                            sample_rate: rate,
                            channels: ch as u16,
                        };
                        if tx.send(Ok(dc)).is_err() {
                            return Ok(());
                        }
                        start = end;
                    }
                    acc.drain(..start);
                }
                match fmt {
                    Some(f) => {
                        cur = Some((f.rate, f.channels));
                        acc.extend_from_slice(src.block());
                    }
                    None => break,
                }
            }
            Ok(())
        })();
        if let Err(e) = res {
            let _ = tx.send(Err(e));
        }
    });

    rx
}
