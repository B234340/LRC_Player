use anyhow::Result;
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::meta::MetadataOptions;
use symphonia::default::get_probe;

use crate::infrastructure::decode::DecodedSource;

#[derive(Default)]
pub struct SymphoniaDecoder;

impl SymphoniaDecoder {
    pub fn new() -> Self {
        Self
    }

    pub fn decode_file<P: AsRef<Path>>(&self, path: P) -> Result<DecodedSource> {
        let path = path.as_ref();
        let src = File::open(path)?;
        let mss = symphonia::core::io::MediaSourceStream::new(Box::new(src), Default::default());
        let probe = get_probe().format(
            &Default::default(),
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;
        let mut format = probe.format;

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
        let mut samples: Vec<f32> = Vec::new();

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
                        samples.extend_from_slice(sb.samples());
                    }
                    Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                    Err(e) => return Err(anyhow::anyhow!(e.to_string())),
                },
                Err(symphonia::core::errors::Error::IoError(_)) => break,
                Err(e) => return Err(anyhow::anyhow!(e.to_string())),
            }
        }

        Ok(DecodedSource::new(samples, sample_rate, channels))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
