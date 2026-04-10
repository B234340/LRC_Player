use crate::audio::output::AudioFrameBlock;

#[derive(Debug, Default)]
pub struct Mixer;

impl Mixer {
    pub fn mix(&self, blocks: &[AudioFrameBlock]) -> Option<AudioFrameBlock> {
        let first = blocks.first()?;
        let mut output = AudioFrameBlock::silence(first.channels, first.frames);

        for block in blocks {
            for (out, input) in output.samples.iter_mut().zip(block.samples.iter()) {
                *out += *input;
            }
        }

        Some(output)
    }
}