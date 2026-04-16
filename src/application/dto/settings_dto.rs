use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsDto {
    pub preferred_output_device: Option<String>,
    pub resampler_taps: usize,
}

impl SettingsDto {
    pub fn new(preferred_output_device: Option<String>, resampler_taps: usize) -> Self {
        Self {
            preferred_output_device,
            resampler_taps,
        }
    }
}
