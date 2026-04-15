use anyhow::Result;
use std::fmt;

/// Simple sample format enum for the abstraction layer.
#[derive(Clone, Copy, Debug)]
pub enum SampleFormat {
    I16,
    F32,
}

#[derive(Clone, Debug)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: SampleFormat,
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} Hz, {} ch, {:?}",
            self.sample_rate, self.channels, self.sample_format
        )
    }
}

/// Cross-cutting audio output trait used by application layer.
/// Implementations will drive actual audio APIs (cpal, WASAPI, PipeWire, etc.).
pub trait AudioOutput {
    /// Start the output with the requested format. Implementations may adjust format.
    fn start(&mut self, format: AudioFormat) -> Result<AudioFormat>;

    /// Stop playback and release resources.
    fn stop(&mut self) -> Result<()>;

    /// Push interleaved PCM samples as 32-bit floats (native endianness).
    /// Implementations may accept other formats; conversion can be performed upstream.
    fn write_f32(&mut self, data: &[f32]) -> Result<usize>;
}
