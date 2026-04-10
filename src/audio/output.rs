#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioOutputConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames_per_buffer: u32,
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
}