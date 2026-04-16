use crate::application::dto::StateDto;

#[derive(Debug, Clone)]
pub struct ScanVm {
    pub last_scan_status: Option<String>,
}

impl From<&StateDto> for ScanVm {
    fn from(dto: &StateDto) -> Self {
        Self {
            last_scan_status: dto.last_tab.clone(),
        }
    }
}
