use anyhow::Result;
use crossbeam_channel::Sender;
use std::path::PathBuf;
use std::sync::Arc;

/// Spawn a decoder thread that decodes `path`, converts/resamples to the target format,
/// and sends `Arc<Vec<f32>>` buffers directly to the provided `producer_tx`. Buffers are
/// obtained from `pool` when provided to avoid allocations.
pub fn spawn_decode_to_producer(
    path: PathBuf,
    chunk_frames: usize,
    target_sample_rate: u32,
    target_channels: u16,
    producer_tx: Sender<Arc<Vec<f32>>>,
    pool: Option<Arc<crate::infrastructure::audio::cpal_backend::BufferPool>>,
    taps: usize,
) -> std::thread::JoinHandle<()> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::default::get_probe;

    std::thread::spawn(move || {
        let res: Result<()> = (|| {
            let src = std::fs::File::open(&path)?;
            let mss =
                symphonia::core::io::MediaSourceStream::new(Box::new(src), Default::default());
            let probe = get_probe().format(
                &Default::default(),
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )?;
            let mut format = probe.format;

            // Find first audio track
            let track = format
                .tracks()
                .iter()
                .find(|t| t.codec_params.sample_rate.is_some())
                .ok_or_else(|| anyhow::anyhow!("no audio track found"))?;
            let codec_params = &track.codec_params;
            let sample_rate = codec_params
                .sample_rate
                .ok_or_else(|| anyhow::anyhow!("sample rate unknown"))?
                as u32;
            let channels = codec_params
                .channels
                .as_ref()
                .map(|c| c.count() as u16)
                .unwrap_or(2);

            let mut decoder =
                symphonia::default::get_codecs().make(codec_params, &DecoderOptions::default())?;

            // converter from src -> target
            let mut conv = crate::infrastructure::audio::stream_adapter::StreamConverter::new(
                sample_rate,
                channels,
                target_sample_rate,
                target_channels,
                taps,
            );

            // accumulator for interleaved samples at source format
            let mut acc: Vec<f32> = Vec::new();
            let src_target_len = chunk_frames.saturating_mul(channels as usize);

            loop {
                match format.next_packet() {
                    Ok(packet) => match decoder.decode(&packet) {
                        Ok(audio_buf) => {
                            let frames = audio_buf.frames();
                            let mut sb = SampleBuffer::<f32>::new(
                                frames as symphonia::core::units::Duration,
                                *audio_buf.spec(),
                            );
                            sb.copy_interleaved_ref(audio_buf);
                            acc.extend_from_slice(sb.samples());

                            while acc.len() >= src_target_len {
                                let chunk = acc.drain(0..src_target_len).collect::<Vec<_>>();

                                // get output buffer from pool or new
                                let mut out_buf = if let Some(p) = &pool {
                                    p.get((chunk_frames * target_channels as usize).max(1024))
                                } else {
                                    Vec::with_capacity(chunk_frames * target_channels as usize)
                                };

                                // convert in-place: stream_adapter writes into out_buf
                                conv.process(&chunk, channels, sample_rate, &mut out_buf);

                                // send to producer (blocking send to apply backpressure)
                                let _ = producer_tx.send(Arc::new(out_buf));
                            }
                        }
                        Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                        Err(e) => return Err(anyhow::anyhow!(e.to_string())),
                    },
                    Err(symphonia::core::errors::Error::IoError(_)) => break,
                    Err(e) => return Err(anyhow::anyhow!(e.to_string())),
                }
            }

            // send remaining samples if any
            if !acc.is_empty() {
                let chunk = acc;
                let mut out_buf = if let Some(p) = &pool {
                    p.get((chunk.len() / channels as usize) * target_channels as usize)
                } else {
                    Vec::with_capacity((chunk.len() / channels as usize) * target_channels as usize)
                };
                conv.process(&chunk, channels, sample_rate, &mut out_buf);
                let _ = producer_tx.send(Arc::new(out_buf));
            }

            Ok(())
        })();

        if let Err(e) = res {
            tracing::error!("decode thread error: {}", e);
        }
    })
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
/// or an error occurs.
pub fn spawn_decode_stream(
    path: PathBuf,
    chunk_frames: usize,
) -> tokio::sync::mpsc::UnboundedReceiver<Result<DecodedChunk>> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::default::get_probe;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    std::thread::spawn(move || {
        // All symphonia calls are done on this blocking thread; send chunks via the unbounded sender.
        let res: Result<()> = (|| {
            let src = std::fs::File::open(&path)?;
            let mss =
                symphonia::core::io::MediaSourceStream::new(Box::new(src), Default::default());
            let probe = get_probe().format(
                &Default::default(),
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )?;
            let mut format = probe.format;

            // Find first audio track with sample rate
            let track = format
                .tracks()
                .iter()
                .find(|t| t.codec_params.sample_rate.is_some())
                .ok_or_else(|| anyhow::anyhow!("no audio track found"))?;
            let codec_params = &track.codec_params;
            let sample_rate = codec_params
                .sample_rate
                .ok_or_else(|| anyhow::anyhow!("sample rate unknown"))?
                as u32;
            let channels = codec_params
                .channels
                .as_ref()
                .map(|c| c.count() as u16)
                .unwrap_or(2);

            let mut decoder =
                symphonia::default::get_codecs().make(codec_params, &DecoderOptions::default())?;

            // accumulator for interleaved samples
            let mut acc: Vec<f32> = Vec::new();
            let target_len = chunk_frames.saturating_mul(channels as usize);

            loop {
                match format.next_packet() {
                    Ok(packet) => match decoder.decode(&packet) {
                        Ok(audio_buf) => {
                            let frames = audio_buf.frames();
                            let mut sb = SampleBuffer::<f32>::new(
                                frames as symphonia::core::units::Duration,
                                *audio_buf.spec(),
                            );
                            sb.copy_interleaved_ref(audio_buf);
                            acc.extend_from_slice(sb.samples());

                            while acc.len() >= target_len {
                                let chunk = acc.drain(0..target_len).collect::<Vec<_>>();
                                let dc = DecodedChunk {
                                    samples: chunk,
                                    sample_rate,
                                    channels,
                                };
                                // ignore send error (receiver closed)
                                let _ = tx.send(Ok(dc));
                            }
                        }
                        Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                        Err(e) => return Err(anyhow::anyhow!(e.to_string())),
                    },
                    Err(symphonia::core::errors::Error::IoError(_)) => break,
                    Err(e) => return Err(anyhow::anyhow!(e.to_string())),
                }
            }

            // send remaining samples if any
            if !acc.is_empty() {
                let dc = DecodedChunk {
                    samples: acc,
                    sample_rate,
                    channels,
                };
                let _ = tx.send(Ok(dc));
            }

            Ok(())
        })();

        if let Err(e) = res {
            let _ = tx.send(Err(e));
        }

        // drop tx to close channel
    });

    rx
}
