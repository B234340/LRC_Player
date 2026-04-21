#[cfg(test)]
mod tests {
    use std::path::Path;
    use lrc_player::infrastructure::decode::symphonia_decoder::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    fn write_wav<P: AsRef<Path>>(path: P, samples: &[i16], sample_rate: u32, channels: u16) {
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
    fn decode_wav_file_returns_pcm_f32() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.wav");
        write_wav(&path, &[0, 32767, -32768, 12345], 44100, 1);

        let decoder = SymphoniaDecoder::new();
        let decoded = decoder.decode_file(&path).expect("decode wav");

        assert_eq!(decoded.sample_rate, 44100);
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.frames(), 4);
        assert_eq!(decoded.samples.len(), 4);
        assert!(decoded.samples.iter().all(|s| s.is_finite()));
    }
}
