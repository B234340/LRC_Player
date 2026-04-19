use crate::application::dto::StateDto;

#[derive(Debug, Clone)]
pub struct ScanVm {
    pub last_scan_status: Option<String>,
    pub scan_scanned: Option<u64>,
    pub scan_parsed: Option<u64>,
    pub scan_registered: Option<u64>,
    pub scan_targets: Vec<String>,
    pub scan_lrc_targets: Vec<String>,
    pub scan_excludes: Vec<String>,
}

impl From<&StateDto> for ScanVm {
    fn from(dto: &StateDto) -> Self {
        Self {
            last_scan_status: dto.last_scan_status.clone(),
            scan_scanned: dto.scan_scanned,
            scan_parsed: dto.scan_parsed,
            scan_registered: dto.scan_registered,
            scan_targets: dto.scan_targets.clone(),
            scan_lrc_targets: dto.scan_lrc_targets.clone(),
            scan_excludes: dto.scan_excludes.clone(),
        }
    }
}
