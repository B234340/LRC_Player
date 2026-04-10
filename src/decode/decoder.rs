use std::fs::File;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;

use crate::audio::output::{AudioFrameBlock, AudioOutputConfig, BufferedAudioOutput};
use crate::core::ids::TrackId;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeWorkerMessage {
    SourceExhausted { track_id: TrackId },
    Failed { track_id: TrackId, error: String },
}

pub struct DecodeWorkerHandle {
    cancel_requested: Arc<AtomicBool>,
    receiver: Receiver<DecodeWorkerMessage>,
}

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
        self.decode_blocks_inner(source, config, Some(max_blocks))
    }

    pub fn decode_all_blocks(
        &self,
        source: &DecodedSource,
        config: AudioOutputConfig,
    ) -> io::Result<Vec<AudioFrameBlock>> {
        self.decode_blocks_inner(source, config, None)
    }

    fn decode_blocks_inner(
        &self,
        source: &DecodedSource,
        config: AudioOutputConfig,
        max_blocks: Option<usize>,
    ) -> io::Result<Vec<AudioFrameBlock>> {
        if max_blocks == Some(0) {
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
        let max_output_frames = max_blocks
            .map(|value| frames_per_block.saturating_mul(value))
            .unwrap_or(usize::MAX);
        let mut source_sample_rate = None;
        let mut decoded_samples: Vec<f32> = Vec::new();

        while decoded_samples.len() / output_channels < max_required_input_frames(
            max_output_frames,
            source_sample_rate.unwrap_or(config.sample_rate),
            config.sample_rate,
        ) {
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
                    source_sample_rate.get_or_insert(spec.rate);
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
                        append_output_channels(
                            &mut decoded_samples,
                            &samples[frame_start..frame_start + decoded_channels],
                            output_channels,
                        );
                    }
                }
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(error) => return Err(map_symphonia_error(error)),
            }
        }

        let resampled = resample_interleaved(
            &decoded_samples,
            output_channels,
            source_sample_rate.unwrap_or(config.sample_rate),
            config.sample_rate,
            max_output_frames,
        );

        let mut blocks = Vec::new();
        for chunk in resampled.chunks(samples_per_block) {
            let mut block_samples = chunk.to_vec();
            if block_samples.len() < samples_per_block {
                block_samples.resize(samples_per_block, 0.0);
            }
            blocks.push(AudioFrameBlock {
                channels: config.channels,
                frames: config.frames_per_buffer,
                samples: block_samples.into_boxed_slice(),
            });
        }

        Ok(blocks)
    }
}

impl DecodeWorkerHandle {
    pub fn spawn(
        source: DecodedSource,
        config: AudioOutputConfig,
        output: BufferedAudioOutput,
        skip_blocks: usize,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel_requested);

        thread::spawn(move || {
            let decoder = Decoder::default();
            match decoder.decode_all_blocks(&source, config) {
                Ok(blocks) => {
                    for block in blocks.into_iter().skip(skip_blocks) {
                        let mut pending_block = Some(block);
                        while let Some(block) = pending_block.take() {
                            if worker_cancel.load(Ordering::Relaxed) {
                                return;
                            }

                            match output.try_enqueue(block) {
                                Ok(()) => {}
                                Err(block) => {
                                    pending_block = Some(block);
                                    thread::yield_now();
                                }
                            }
                        }
                    }

                    let _ = sender.send(DecodeWorkerMessage::SourceExhausted {
                        track_id: source.track_id,
                    });
                }
                Err(error) => {
                    let _ = sender.send(DecodeWorkerMessage::Failed {
                        track_id: source.track_id,
                        error: error.to_string(),
                    });
                }
            }
        });

        Self {
            cancel_requested,
            receiver,
        }
    }

    pub fn cancel(&self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
    }

    pub fn drain_messages(&self) -> Vec<DecodeWorkerMessage> {
        let mut messages = Vec::new();
        while let Ok(message) = self.receiver.try_recv() {
            messages.push(message);
        }
        messages
    }
}

fn append_output_channels(decoded_samples: &mut Vec<f32>, input_frame: &[f32], output_channels: usize) {
    if input_frame.is_empty() {
        return;
    }

    match input_frame.len().cmp(&output_channels) {
        std::cmp::Ordering::Equal => decoded_samples.extend_from_slice(input_frame),
        std::cmp::Ordering::Greater => decoded_samples.extend_from_slice(&input_frame[..output_channels]),
        std::cmp::Ordering::Less => {
            let fill = input_frame[0];
            decoded_samples.extend_from_slice(input_frame);
            for _ in input_frame.len()..output_channels {
                decoded_samples.push(fill);
            }
        }
    }
}

fn max_required_input_frames(max_output_frames: usize, source_rate: u32, output_rate: u32) -> usize {
    if max_output_frames == 0 || source_rate == 0 || output_rate == 0 {
        return 0;
    }

    ((max_output_frames as u128 * source_rate as u128).div_ceil(output_rate as u128)) as usize
}

fn resample_interleaved(
    input: &[f32],
    channels: usize,
    source_rate: u32,
    output_rate: u32,
    max_output_frames: usize,
) -> Vec<f32> {
    if input.is_empty() || channels == 0 || max_output_frames == 0 {
        return Vec::new();
    }

    let input_frames = input.len() / channels;
    if input_frames == 0 {
        return Vec::new();
    }

    let target_frames = if source_rate == 0 || output_rate == 0 || source_rate == output_rate {
        input_frames.min(max_output_frames)
    } else {
        (((input_frames as u128 * output_rate as u128) + (source_rate as u128 / 2)) / source_rate as u128)
            as usize
    }
    .min(max_output_frames);

    let mut output = Vec::with_capacity(target_frames * channels);
    let step = if source_rate == 0 || output_rate == 0 {
        1.0
    } else {
        source_rate as f64 / output_rate as f64
    };

    for output_frame in 0..target_frames {
        let source_position = output_frame as f64 * step;
        let base_index = source_position.floor() as usize;
        let next_index = (base_index + 1).min(input_frames.saturating_sub(1));
        let fraction = (source_position - base_index as f64) as f32;

        for channel in 0..channels {
            let base_sample = input[base_index * channels + channel];
            let next_sample = input[next_index * channels + channel];
            output.push(base_sample + ((next_sample - base_sample) * fraction));
        }
    }

    output
}

fn map_symphonia_error(error: SymphoniaError) -> io::Error {
    match error {
        SymphoniaError::IoError(error) => error,
        other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use hound::{SampleFormat, WavSpec, WavWriter};
    use tempfile::TempDir;

    use super::{DecodeWorkerHandle, Decoder};
    use crate::audio::output::{AudioOutputConfig, BufferedAudioOutput};
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

    #[test]
    fn decoder_resamples_to_output_device_rate() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let file_path = temp_dir.path().join("resample.wav");
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&file_path, spec).expect("wav file should be created");
        for index in 0..48_000u32 {
            let phase = (index % 256) as i16;
            let sample = (phase * 128) - 16_384;
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
                    sample_rate: 44_100,
                    channels: 2,
                    frames_per_buffer: 441,
                },
                200,
            )
            .expect("decode should succeed");

        assert_eq!(blocks.len(), 100);
        assert!(blocks.iter().all(|block| block.frames == 441));
        assert!(blocks.iter().any(|block| block.samples.iter().any(|sample| *sample != 0.0)));
    }

    #[test]
    fn decode_worker_refills_bounded_output_after_consumption() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let file_path = temp_dir.path().join("stream.wav");
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&file_path, spec).expect("wav file should be created");
        for index in 0..48_000u32 {
            let phase = (index % 256) as i16;
            let sample = (phase * 128) - 16_384;
            writer.write_sample(sample).expect("left sample should be written");
            writer.write_sample(sample).expect("right sample should be written");
        }
        writer.finalize().expect("wav file should finalize");

        let config = AudioOutputConfig {
            sample_rate: 48_000,
            channels: 2,
            frames_per_buffer: 1_024,
        };
        let output = BufferedAudioOutput::new(config, 2);
        let source = DecodedSource {
            track_id: TrackId(7),
            path: file_path,
        };
        let decoder = Decoder::default();
        let initial_blocks = decoder
            .decode_blocks(&source, config, 2)
            .expect("initial prime should decode");
        let primed = output.try_enqueue_many(initial_blocks);
        assert_eq!(primed, 2);
        assert_eq!(output.buffered_blocks(), 2);

        let worker = DecodeWorkerHandle::spawn(source, config, output.clone(), primed);

        let _ = output.pop_or_silence();
        for _ in 0..10_000 {
            if output.buffered_blocks() == 2 {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }

        assert_eq!(output.buffered_blocks(), 2);
        worker.cancel();
    }
}