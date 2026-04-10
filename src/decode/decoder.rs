use crate::audio::output::AudioFrameBlock;
use crate::decode::source::DecodedSource;

#[derive(Debug, Default)]
pub struct Decoder;

impl Decoder {
    pub fn decode_preview(&self, _source: &DecodedSource, channels: u16, frames: u32) -> AudioFrameBlock {
        AudioFrameBlock::silence(channels, frames)
    }
}