use crate::application::ports::{AppAudioFormat, AudioOutputPort, DecoderPort, SampleFormat};
use crate::infrastructure::audio::stream_adapter::StreamConverter;
use anyhow::Result;
use std::path::PathBuf;

/// Play a track by decoding it and sending PCM samples to the audio output.
pub fn play<D, O>(decoder: &D, output: &mut O, path: PathBuf, start_ms: Option<u64>) -> Result<()>
where
    D: DecoderPort,
    O: AudioOutputPort,
{
    let decoded = decoder.decode_file(&path)?;
    let requested_format = AppAudioFormat {
        sample_rate: decoded.sample_rate,
        channels: decoded.channels,
        sample_format: SampleFormat::F32,
    };

    let actual_format = output.start(requested_format)?;

    let samples = if decoded.sample_rate != actual_format.sample_rate
        || decoded.channels != actual_format.channels
    {
        let mut converted = Vec::with_capacity(
            (decoded.samples.len() / decoded.channels as usize)
                .saturating_mul(actual_format.channels as usize)
                .max(1024),
        );
        let mut conv = StreamConverter::new(
            decoded.sample_rate,
            decoded.channels,
            actual_format.sample_rate,
            actual_format.channels,
            32,
        );
        conv.process(
            &decoded.samples,
            decoded.channels,
            decoded.sample_rate,
            &mut converted,
        );
        converted
    } else {
        decoded.samples
    };
    let start_offset = if let Some(start_ms) = start_ms {
        let skip_frames = start_ms.saturating_mul(decoded.sample_rate as u64) / 1000;
        let skip_samples = skip_frames.saturating_mul(decoded.channels as u64) as usize;
        std::cmp::min(skip_samples, samples.len())
    } else {
        0
    };

    let mut offset = start_offset;
    while offset < samples.len() {
        let written = output.write_f32(&samples[offset..])?;
        if written == 0 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        } else {
            offset += written;
        }
    }

    Ok(())
}


