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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::decode::SymphoniaDecoder;
    use crate::ports::audio_output::AudioOutput;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    struct TestOutput {
        format: Option<AppAudioFormat>,
        written: Vec<f32>,
    }

    impl TestOutput {
        fn new() -> Self {
            Self {
                format: None,
                written: Vec::new(),
            }
        }
    }

    impl AudioOutput for TestOutput {
        fn start(&mut self, format: AppAudioFormat) -> Result<AppAudioFormat> {
            self.format = Some(format.clone());
            Ok(format)
        }

        fn stop(&mut self) -> Result<()> {
            Ok(())
        }

        fn write_f32(&mut self, data: &[f32]) -> Result<usize> {
            self.written.extend_from_slice(data);
            Ok(data.len())
        }
    }

    fn write_wav<P: AsRef<std::path::Path>>(
        path: P,
        samples: &[i16],
        sample_rate: u32,
        channels: u16,
    ) {
        let mut file = File::create(path).unwrap();
        let bits_per_sample = 16u16;
        let byte_rate = sample_rate * channels as u32 * (bits_per_sample / 8) as u32;
        let block_align = channels * (bits_per_sample / 8);
        let subchunk2_size = (samples.len() * 2) as u32;
        let chunk_size = 36 + subchunk2_size;

        file.write_all(b"RIFF").unwrap();
        file.write_all(&chunk_size.to_le_bytes()).unwrap();
        file.write_all(b"WAVE").unwrap();
        file.write_all(b"fmt ").unwrap();
        file.write_all(&16u32.to_le_bytes()).unwrap();
        file.write_all(&1u16.to_le_bytes()).unwrap();
        file.write_all(&channels.to_le_bytes()).unwrap();
        file.write_all(&sample_rate.to_le_bytes()).unwrap();
        file.write_all(&byte_rate.to_le_bytes()).unwrap();
        file.write_all(&block_align.to_le_bytes()).unwrap();
        file.write_all(&bits_per_sample.to_le_bytes()).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&subchunk2_size.to_le_bytes()).unwrap();

        for &sample in samples {
            file.write_all(&sample.to_le_bytes()).unwrap();
        }
    }

    #[test]
    fn play_decoded_file_writes_samples_to_output() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.wav");
        write_wav(&path, &[0, 32767, -32768, 12345], 44100, 1);

        let decoder = SymphoniaDecoder::new();
        let mut output = TestOutput::new();

        play(&decoder, &mut output, path.clone(), Some(0)).expect("play track");

        assert!(output.format.is_some());
        let format = output.format.unwrap();
        assert_eq!(format.sample_rate, 44100);
        assert_eq!(format.channels, 1);
        assert!(!output.written.is_empty());
        assert_eq!(output.written.len(), 4);
    }
}
