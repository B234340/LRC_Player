use crate::audio::ring_buffer::RingBuffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioOutputConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames_per_buffer: u32,
}

impl Default for AudioOutputConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            frames_per_buffer: 1_024,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioFrameBlock {
    pub channels: u16,
    pub frames: u32,
    pub samples: Box<[f32]>,
}

impl AudioFrameBlock {
    pub fn silence(channels: u16, frames: u32) -> Self {
        let sample_count = channels as usize * frames as usize;
        Self {
            channels,
            frames,
            samples: vec![0.0; sample_count].into_boxed_slice(),
        }
    }

    pub fn duration_ms(&self, sample_rate: u32) -> u32 {
        if sample_rate == 0 {
            return 0;
        }

        ((self.frames as u64 * 1_000) / sample_rate as u64) as u32
    }
}

#[derive(Debug)]
pub struct BufferedAudioOutput {
    config: AudioOutputConfig,
    queue: RingBuffer<AudioFrameBlock>,
}

impl BufferedAudioOutput {
    pub fn new(config: AudioOutputConfig, block_capacity: usize) -> Self {
        Self {
            config,
            queue: RingBuffer::new(block_capacity),
        }
    }

    pub fn config(&self) -> AudioOutputConfig {
        self.config
    }

    pub fn clear(&mut self) {
        while self.queue.pop().is_some() {}
    }

    pub fn try_enqueue(&mut self, block: AudioFrameBlock) -> Result<(), AudioFrameBlock> {
        self.queue.push(block)
    }

    pub fn try_enqueue_many<I>(&mut self, blocks: I) -> usize
    where
        I: IntoIterator<Item = AudioFrameBlock>,
    {
        let mut inserted = 0;
        for block in blocks {
            if self.try_enqueue(block).is_err() {
                break;
            }
            inserted += 1;
        }
        inserted
    }

    pub fn pop_or_silence(&mut self) -> AudioFrameBlock {
        self.queue.pop().unwrap_or_else(|| {
            AudioFrameBlock::silence(self.config.channels, self.config.frames_per_buffer)
        })
    }

    pub fn buffered_blocks(&self) -> usize {
        self.queue.len()
    }

    pub fn block_capacity(&self) -> usize {
        self.queue.capacity()
    }

    pub fn buffered_ms(&self) -> u32 {
        ((self.queue.len() as u64 * self.config.frames_per_buffer as u64 * 1_000)
            / self.config.sample_rate as u64) as u32
    }

    pub fn is_full(&self) -> bool {
        self.queue.is_full()
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioFrameBlock, AudioOutputConfig, BufferedAudioOutput};

    #[test]
    fn buffered_output_is_bounded() {
        let mut output = BufferedAudioOutput::new(AudioOutputConfig::default(), 2);

        assert!(output.try_enqueue(AudioFrameBlock::silence(2, 16)).is_ok());
        assert!(output.try_enqueue(AudioFrameBlock::silence(2, 16)).is_ok());
        assert!(output.try_enqueue(AudioFrameBlock::silence(2, 16)).is_err());
        assert_eq!(output.buffered_blocks(), 2);
        assert!(output.is_full());
    }
}