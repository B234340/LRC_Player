#[derive(Clone, Debug)]
pub struct DecodedSource {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl DecodedSource {
    pub fn new(samples: Vec<f32>, sample_rate: u32, channels: u16) -> Self {
        Self {
            samples,
            sample_rate,
            channels,
        }
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }
}
