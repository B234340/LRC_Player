use crate::dto::SettingsDto;

#[derive(Debug, Clone)]
pub struct SettingsVm {
    pub theme: Option<String>,
    pub preferred_output_device: Option<String>,
    pub resampler_taps: usize,
}

impl From<&SettingsDto> for SettingsVm {
    fn from(dto: &SettingsDto) -> Self {
        Self {
            theme: None,
            preferred_output_device: dto.preferred_output_device.clone(),
            resampler_taps: dto.resampler_taps,
        }
    }
}
