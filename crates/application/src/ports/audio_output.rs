use lrc_player_ports::audio_output::{AudioFormat, AudioOutput};

/// Application-level port for audio output. This is a thin abstraction over the
/// lower-level cross-cutting audio output trait used by the infrastructure.
pub trait AudioOutputPort: AudioOutput {}

impl<T: AudioOutput> AudioOutputPort for T {}

pub type AppAudioFormat = AudioFormat;

pub use lrc_player_ports::audio_output::SampleFormat;
