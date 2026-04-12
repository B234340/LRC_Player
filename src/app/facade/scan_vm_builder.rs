// Scan VM builder: extracted from `AppFacade` to reduce facade size.
use crate::app::view_models::ScanTabVm;

pub fn build_scan_tab(facade: &super::AppFacade) -> ScanTabVm {
    ScanTabVm {
        include_dirs: facade
            .scan
            .config
            .include_dirs
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        exclude_dirs: facade
            .scan
            .config
            .exclude_dirs
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        visible_status: facade.scan.visible_status(),
        cancel_visible: matches!(
            facade.scan.state,
            crate::core::scan::ScanState::Scanning | crate::core::scan::ScanState::Cancelling
        ),
    }
}
