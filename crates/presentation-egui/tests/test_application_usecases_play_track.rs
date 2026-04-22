#[cfg(test)]
mod tests {
    use lrc_player::application::ports::AppAudioFormat;
    use anyhow::Result;
    use lrc_player::application::usecases::play_track::*;
    use lrc_player::infrastructure::decode::SymphoniaDecoder;
    use lrc_player::ports::audio_output::AudioOutput;
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
            self.format = Some(format);
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
