use crate::core::models::TrackId;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use std::fmt::{Display, Formatter};
use std::fs::File;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use symphonia::core::audio::{AudioBufferRef, SampleBuffer};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone)]
pub struct PlaybackSnapshot {
    pub status: PlaybackStatus,
    pub current_track: Option<TrackId>,
    pub position_ms: u64,
}

impl Default for PlaybackSnapshot {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            current_track: None,
            position_ms: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum PlaybackCommand {
    PlayTrack { track_id: TrackId, file_path: PathBuf },
    Pause,
    Resume,
    Stop,
    Seek(u64),
}

#[derive(Debug)]
pub struct PlayerError {
    message: String,
}

impl PlayerError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for PlayerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PlayerError {}

pub trait PlayerBackend {
    fn dispatch(&mut self, command: PlaybackCommand) -> Result<(), PlayerError>;
    fn snapshot(&self) -> PlaybackSnapshot;
}

pub struct SymphoniaCpalPlayer {
    shared: Arc<Mutex<SharedPlaybackState>>,
    _stream: Stream,
    output_sample_rate: u32,
    output_channels: u16,
}

impl SymphoniaCpalPlayer {
    pub fn new() -> Result<Self, PlayerError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or_else(|| {
            PlayerError::new("No default audio output device is available.")
        })?;
        let supported_config = device
            .default_output_config()
            .map_err(|err| PlayerError::new(format!("Audio output configuration failed: {err}")))?;
        let stream_config = supported_config.config();
        let sample_format = supported_config.sample_format();
        let shared = Arc::new(Mutex::new(SharedPlaybackState::new(
            stream_config.sample_rate.0,
            stream_config.channels,
        )));
        let stream = build_output_stream(&device, &stream_config, sample_format, Arc::clone(&shared))?;

        stream
            .play()
            .map_err(|err| PlayerError::new(format!("Audio output stream failed to start: {err}")))?;

        Ok(Self {
            shared,
            _stream: stream,
            output_sample_rate: stream_config.sample_rate.0,
            output_channels: stream_config.channels,
        })
    }
}

impl PlayerBackend for SymphoniaCpalPlayer {
    fn dispatch(&mut self, command: PlaybackCommand) -> Result<(), PlayerError> {
        match command {
            PlaybackCommand::PlayTrack { track_id, file_path } => {
                let decoded = decode_audio_file(
                    &file_path,
                    self.output_sample_rate,
                    self.output_channels,
                )?;
                let mut state = self
                    .shared
                    .lock()
                    .map_err(|_| PlayerError::new("Playback state lock poisoned."))?;
                state.samples = decoded;
                state.position_samples = 0;
                state.status = PlaybackStatus::Playing;
                state.current_track = Some(track_id);
                state.sample_rate = self.output_sample_rate;
                state.channels = self.output_channels;
            }
            PlaybackCommand::Pause => {
                let mut state = self
                    .shared
                    .lock()
                    .map_err(|_| PlayerError::new("Playback state lock poisoned."))?;
                if state.status == PlaybackStatus::Playing {
                    state.status = PlaybackStatus::Paused;
                }
            }
            PlaybackCommand::Resume => {
                let mut state = self
                    .shared
                    .lock()
                    .map_err(|_| PlayerError::new("Playback state lock poisoned."))?;
                if !state.samples.is_empty() {
                    state.status = PlaybackStatus::Playing;
                }
            }
            PlaybackCommand::Stop => {
                let mut state = self
                    .shared
                    .lock()
                    .map_err(|_| PlayerError::new("Playback state lock poisoned."))?;
                state.status = PlaybackStatus::Stopped;
                state.position_samples = 0;
            }
            PlaybackCommand::Seek(position_ms) => {
                let mut state = self
                    .shared
                    .lock()
                    .map_err(|_| PlayerError::new("Playback state lock poisoned."))?;
                let channels = state.channels.max(1) as usize;
                let target_frames = ((position_ms as u128) * (state.sample_rate as u128) / 1_000) as usize;
                let target_samples = target_frames.saturating_mul(channels);
                state.position_samples = target_samples.min(state.samples.len());
            }
        }

        Ok(())
    }

    fn snapshot(&self) -> PlaybackSnapshot {
        let state = match self.shared.lock() {
            Ok(state) => state,
            Err(_) => return PlaybackSnapshot::default(),
        };

        let channels = state.channels.max(1) as usize;
        let frames = state.position_samples / channels;
        let position_ms = ((frames as u128) * 1_000 / (state.sample_rate.max(1) as u128)) as u64;

        PlaybackSnapshot {
            status: state.status,
            current_track: state.current_track.clone(),
            position_ms,
        }
    }
}

#[derive(Debug, Default)]
pub struct NullPlayer {
    snapshot: PlaybackSnapshot,
}

impl PlayerBackend for NullPlayer {
    fn dispatch(&mut self, command: PlaybackCommand) -> Result<(), PlayerError> {
        match command {
            PlaybackCommand::PlayTrack { track_id, .. } => {
                self.snapshot.current_track = Some(track_id);
                self.snapshot.position_ms = 0;
                self.snapshot.status = PlaybackStatus::Playing;
            }
            PlaybackCommand::Pause => {
                self.snapshot.status = PlaybackStatus::Paused;
            }
            PlaybackCommand::Resume => {
                self.snapshot.status = PlaybackStatus::Playing;
            }
            PlaybackCommand::Stop => {
                self.snapshot.status = PlaybackStatus::Stopped;
                self.snapshot.position_ms = 0;
            }
            PlaybackCommand::Seek(position_ms) => {
                self.snapshot.position_ms = position_ms;
            }
        }

        Ok(())
    }

    fn snapshot(&self) -> PlaybackSnapshot {
        self.snapshot.clone()
    }
}

#[derive(Debug)]
struct SharedPlaybackState {
    status: PlaybackStatus,
    current_track: Option<TrackId>,
    sample_rate: u32,
    channels: u16,
    position_samples: usize,
    samples: Vec<i16>,
}

impl SharedPlaybackState {
    fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            current_track: None,
            sample_rate,
            channels,
            position_samples: 0,
            samples: Vec::new(),
        }
    }
}

fn build_output_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    shared: Arc<Mutex<SharedPlaybackState>>,
) -> Result<Stream, PlayerError> {
    let err_fn = |err| {
        eprintln!("audio stream error: {err}");
    };

    match sample_format {
        SampleFormat::F32 => device
            .build_output_stream(
                config,
                {
                    let shared = Arc::clone(&shared);
                    move |data: &mut [f32], _| write_output_data(data, &shared)
                },
                err_fn,
                None,
            )
            .map_err(|err| PlayerError::new(format!("Audio stream creation failed: {err}"))),
        SampleFormat::I16 => device
            .build_output_stream(
                config,
                {
                    let shared = Arc::clone(&shared);
                    move |data: &mut [i16], _| write_output_data(data, &shared)
                },
                err_fn,
                None,
            )
            .map_err(|err| PlayerError::new(format!("Audio stream creation failed: {err}"))),
        SampleFormat::U16 => device
            .build_output_stream(
                config,
                {
                    let shared = Arc::clone(&shared);
                    move |data: &mut [u16], _| write_output_data(data, &shared)
                },
                err_fn,
                None,
            )
            .map_err(|err| PlayerError::new(format!("Audio stream creation failed: {err}"))),
        _ => Err(PlayerError::new(format!(
            "Unsupported audio sample format: {sample_format:?}"
        ))),
    }
}

fn write_output_data<T>(output: &mut [T], shared: &Arc<Mutex<SharedPlaybackState>>)
where
    T: Sample + FromSample<f32> + SizedSample,
{
    let silence = T::from_sample(0.0f32);
    let mut state = match shared.lock() {
        Ok(state) => state,
        Err(_) => {
            output.fill(silence);
            return;
        }
    };

    if state.status != PlaybackStatus::Playing || state.samples.is_empty() {
        output.fill(silence);
        return;
    }

    for sample in output.iter_mut() {
        if state.position_samples >= state.samples.len() {
            state.status = PlaybackStatus::Stopped;
            *sample = silence;
            continue;
        }

        let normalized = state.samples[state.position_samples] as f32 / i16::MAX as f32;
        *sample = T::from_sample(normalized);
        state.position_samples += 1;
    }

    if state.position_samples >= state.samples.len() {
        state.status = PlaybackStatus::Stopped;
    }
}

fn decode_audio_file(
    file_path: &Path,
    output_sample_rate: u32,
    output_channels: u16,
) -> Result<Vec<i16>, PlayerError> {
    let file = File::open(file_path)
        .map_err(|err| PlayerError::new(format!("Audio file open failed: {err}")))?;
    let media_source = MediaSourceStream::new(Box::new(file), Default::default());
    let hint = Hint::new();

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            media_source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|err| PlayerError::new(format!("Audio format probe failed: {err}")))?;
    let mut format = probed.format;

    let track = format
        .default_track()
        .ok_or_else(|| PlayerError::new("No supported audio track was found in the selected file."))?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let source_sample_rate = codec_params
        .sample_rate
        .ok_or_else(|| PlayerError::new("The selected audio file does not report a sample rate."))?;
    let source_channels = codec_params
        .channels
        .ok_or_else(|| PlayerError::new("The selected audio file does not report channel information."))?
        .count();

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|err| PlayerError::new(format!("Audio decoder initialization failed: {err}")))?;
    let mut decoded_samples = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(err)) if err.kind() == ErrorKind::UnexpectedEof => break,
            Err(SymphoniaError::ResetRequired) => {
                return Err(PlayerError::new(
                    "Audio decoder reset was requested and is not handled by this player yet.",
                ))
            }
            Err(err) => {
                return Err(PlayerError::new(format!("Audio packet read failed: {err}")))
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(err)) if err.kind() == ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(PlayerError::new(format!("Audio decode failed: {err}"))),
        };

        append_decoded_samples(&mut decoded_samples, decoded);
    }

    if decoded_samples.is_empty() {
        return Err(PlayerError::new("The selected audio file produced no playable samples."));
    }

    Ok(quantize_samples(&resample_interleaved(
        &decoded_samples,
        source_sample_rate,
        source_channels,
        output_sample_rate,
        output_channels as usize,
    )))
}

fn append_decoded_samples(output: &mut Vec<f32>, decoded: AudioBufferRef<'_>) {
    match decoded {
        AudioBufferRef::F32(buffer) => {
            let mut sample_buffer = SampleBuffer::<f32>::new(buffer.capacity() as u64, *buffer.spec());
            sample_buffer.copy_interleaved_ref(AudioBufferRef::F32(buffer));
            output.extend_from_slice(sample_buffer.samples());
        }
        AudioBufferRef::U8(buffer) => {
            let mut sample_buffer = SampleBuffer::<f32>::new(buffer.capacity() as u64, *buffer.spec());
            sample_buffer.copy_interleaved_ref(AudioBufferRef::U8(buffer));
            output.extend_from_slice(sample_buffer.samples());
        }
        AudioBufferRef::U16(buffer) => {
            let mut sample_buffer = SampleBuffer::<f32>::new(buffer.capacity() as u64, *buffer.spec());
            sample_buffer.copy_interleaved_ref(AudioBufferRef::U16(buffer));
            output.extend_from_slice(sample_buffer.samples());
        }
        AudioBufferRef::U24(buffer) => {
            let mut sample_buffer = SampleBuffer::<f32>::new(buffer.capacity() as u64, *buffer.spec());
            sample_buffer.copy_interleaved_ref(AudioBufferRef::U24(buffer));
            output.extend_from_slice(sample_buffer.samples());
        }
        AudioBufferRef::U32(buffer) => {
            let mut sample_buffer = SampleBuffer::<f32>::new(buffer.capacity() as u64, *buffer.spec());
            sample_buffer.copy_interleaved_ref(AudioBufferRef::U32(buffer));
            output.extend_from_slice(sample_buffer.samples());
        }
        AudioBufferRef::S8(buffer) => {
            let mut sample_buffer = SampleBuffer::<f32>::new(buffer.capacity() as u64, *buffer.spec());
            sample_buffer.copy_interleaved_ref(AudioBufferRef::S8(buffer));
            output.extend_from_slice(sample_buffer.samples());
        }
        AudioBufferRef::S16(buffer) => {
            let mut sample_buffer = SampleBuffer::<f32>::new(buffer.capacity() as u64, *buffer.spec());
            sample_buffer.copy_interleaved_ref(AudioBufferRef::S16(buffer));
            output.extend_from_slice(sample_buffer.samples());
        }
        AudioBufferRef::S24(buffer) => {
            let mut sample_buffer = SampleBuffer::<f32>::new(buffer.capacity() as u64, *buffer.spec());
            sample_buffer.copy_interleaved_ref(AudioBufferRef::S24(buffer));
            output.extend_from_slice(sample_buffer.samples());
        }
        AudioBufferRef::S32(buffer) => {
            let mut sample_buffer = SampleBuffer::<f32>::new(buffer.capacity() as u64, *buffer.spec());
            sample_buffer.copy_interleaved_ref(AudioBufferRef::S32(buffer));
            output.extend_from_slice(sample_buffer.samples());
        }
        AudioBufferRef::F64(buffer) => {
            let mut sample_buffer = SampleBuffer::<f32>::new(buffer.capacity() as u64, *buffer.spec());
            sample_buffer.copy_interleaved_ref(AudioBufferRef::F64(buffer));
            output.extend_from_slice(sample_buffer.samples());
        }
    }
}

fn resample_interleaved(
    input: &[f32],
    source_sample_rate: u32,
    source_channels: usize,
    output_sample_rate: u32,
    output_channels: usize,
) -> Vec<f32> {
    if input.is_empty() || source_channels == 0 || output_channels == 0 {
        return Vec::new();
    }

    let source_frames = input.len() / source_channels;
    if source_frames == 0 {
        return Vec::new();
    }

    let output_frames = if source_sample_rate == output_sample_rate {
        source_frames
    } else {
        ((source_frames as u128) * (output_sample_rate as u128) / (source_sample_rate as u128))
            .max(1) as usize
    };

    let mut output = Vec::with_capacity(output_frames.saturating_mul(output_channels));

    for output_frame in 0..output_frames {
        let source_position = (output_frame as f64) * (source_sample_rate as f64)
            / (output_sample_rate as f64);
        let base_frame = source_position.floor() as usize;
        let next_frame = (base_frame + 1).min(source_frames.saturating_sub(1));
        let fraction = (source_position - base_frame as f64) as f32;

        for channel in 0..output_channels {
            let start = base_frame.saturating_mul(source_channels);
            let next_start = next_frame.saturating_mul(source_channels);
            let sample_a = remap_channel(
                &input[start..start + source_channels],
                channel,
                output_channels,
            );
            let sample_b = remap_channel(
                &input[next_start..next_start + source_channels],
                channel,
                output_channels,
            );
            output.push(sample_a + ((sample_b - sample_a) * fraction));
        }
    }

    output
}

fn remap_channel(frame: &[f32], output_channel: usize, output_channels: usize) -> f32 {
    match (frame.len(), output_channels) {
        (0, _) => 0.0,
        (_, 1) => frame.iter().copied().sum::<f32>() / frame.len() as f32,
        (1, _) => frame[0],
        _ => frame[output_channel.min(frame.len() - 1)],
    }
}

fn quantize_samples(input: &[f32]) -> Vec<i16> {
    input
        .iter()
        .map(|sample| {
            let clamped = sample.clamp(-1.0, 1.0);
            (clamped * i16::MAX as f32) as i16
        })
        .collect()
}