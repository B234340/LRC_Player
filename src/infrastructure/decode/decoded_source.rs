#[derive(Clone, Debug)]
pub struct DecodedSource {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

impl DecodedSource {
    pub fn new(samples: Vec<f32>, sample_rate: u32, channels: u16) -> Self {
        Self::with_metadata(samples, sample_rate, channels, None, None, None)
    }

    pub fn with_metadata(
        samples: Vec<f32>,
        sample_rate: u32,
        channels: u16,
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
    ) -> Self {
        Self {
            samples,
            sample_rate,
            channels,
            title,
            artist,
            album,
        }
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }
}
