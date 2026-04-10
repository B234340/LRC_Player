use std::io;
use std::fs::File;

use crate::audio::output::{AudioFrameBlock, AudioOutputConfig};
use crate::decode::source::DecodedSource;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, Default)]
pub struct Decoder;

impl Decoder {
    pub fn decode_preview(&self, _source: &DecodedSource, channels: u16, frames: u32) -> AudioFrameBlock {
        AudioFrameBlock::silence(channels, frames)
    }

    pub fn decode_blocks(
        &self,
        source: &DecodedSource,
        config: AudioOutputConfig,
        max_blocks: usize,
    ) -> io::Result<Vec<AudioFrameBlock>> {
        if max_blocks == 0 {
            return Ok(Vec::new());
        }

        let file = File::open(&source.path)?;
        let media_source = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(extension) = source.path.extension().and_then(|ext| ext.to_str()) {
            hint.with_extension(extension);
        }

        let probed = symphonia::default::get_probe()
            .format(&hint, media_source, &FormatOptions::default(), &MetadataOptions::default())
            .map_err(map_symphonia_error)?;
        let mut format = probed.format;
        let track = format
            .default_track()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no default audio track"))?;
        let track_id = track.id;
        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(map_symphonia_error)?;

        let frames_per_block = config.frames_per_buffer as usize;
        let output_channels = config.channels as usize;
        let samples_per_block = output_channels * frames_per_block;
        let mut pending_samples: Vec<f32> = Vec::new();
        let mut blocks = Vec::new();

        while blocks.len() < max_blocks {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(error) => return Err(map_symphonia_error(error)),
            };

            if packet.track_id() != track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(audio_buffer) => {
                    let spec = *audio_buffer.spec();
                    let duration = audio_buffer.capacity() as u64;
                    let mut sample_buffer = SampleBuffer::<f32>::new(duration, spec);
                    sample_buffer.copy_interleaved_ref(audio_buffer);

                    let decoded_channels = spec.channels.count();
                    if decoded_channels == 0 {
                        continue;
                    }

                    let samples = sample_buffer.samples();
                    let decoded_frames = samples.len() / decoded_channels;
                    for frame_index in 0..decoded_frames {
                        let frame_start = frame_index * decoded_channels;
                        match decoded_channels.cmp(&output_channels) {
                            std::cmp::Ordering::Equal => {
                                pending_samples.extend_from_slice(
                                    &samples[frame_start..frame_start + output_channels],
                                );
                            }
                            std::cmp::Ordering::Greater => {
                                pending_samples.extend_from_slice(
                                    &samples[frame_start..frame_start + output_channels],
                                );
                            }
                            std::cmp::Ordering::Less => {
                                let mono = samples[frame_start];
                                for _ in 0..output_channels {
                                    pending_samples.push(mono);
                                }
                            }
                        }

                        if pending_samples.len() >= samples_per_block {
                            let block_samples: Vec<f32> = pending_samples.drain(..samples_per_block).collect();
                            blocks.push(AudioFrameBlock {
                                channels: config.channels,
                                frames: config.frames_per_buffer,
                                samples: block_samples.into_boxed_slice(),
                            });
                            if blocks.len() >= max_blocks {
                                break;
                            }
                        }
                    }
                }
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(error) => return Err(map_symphonia_error(error)),
            }
        }

        if blocks.len() < max_blocks && !pending_samples.is_empty() {
            pending_samples.resize(samples_per_block, 0.0);
            blocks.push(AudioFrameBlock {
                channels: config.channels,
                frames: config.frames_per_buffer,
                samples: pending_samples.into_boxed_slice(),
            });
        }

        Ok(blocks)
    }
}

fn map_symphonia_error(error: SymphoniaError) -> io::Error {
    match error {
        SymphoniaError::IoError(error) => error,
        other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use hound::{SampleFormat, WavSpec, WavWriter};
    use tempfile::TempDir;

    use super::Decoder;
    use crate::audio::output::AudioOutputConfig;
    use crate::core::ids::TrackId;
    use crate::decode::source::DecodedSource;

    #[test]
    fn decoder_creates_frame_blocks_from_file_bytes() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let file_path = temp_dir.path().join("sample.wav");
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&file_path, spec).expect("wav file should be created");
        for index in 0..16u16 {
            let sample = (index as i16 * 256) - 2048;
            writer.write_sample(sample).expect("left sample should be written");
            writer.write_sample(sample).expect("right sample should be written");
        }
        writer.finalize().expect("wav file should be finalized");

        let decoder = Decoder::default();
        let blocks = decoder
            .decode_blocks(
                &DecodedSource {
                    track_id: TrackId(1),
                    path: file_path,
                },
                AudioOutputConfig {
                    sample_rate: 48_000,
                    channels: 2,
                    frames_per_buffer: 4,
                },
                4,
            )
            .expect("decode should succeed");

        assert!(blocks.len() >= 2);
        assert_eq!(blocks[0].samples.len(), 8);
        assert!(blocks[0].samples.iter().any(|sample| *sample != 0.0));
    }
}