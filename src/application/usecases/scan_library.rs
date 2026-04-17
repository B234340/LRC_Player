use crate::application::repositories::ScanGateway;
use crate::infrastructure::scan::{ScanHandle, ScanOptions};
use anyhow::Result;
use std::path::PathBuf;

/// Start a library scan through the application scan gateway.
pub async fn scan_paths<G: ScanGateway>(
    gateway: &G,
    paths: Vec<PathBuf>,
    fast: bool,
) -> Result<ScanHandle> {
    gateway
        .start_scan(paths, ScanOptions { fast, excludes: Vec::new() })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::persistence::MetaCache;
    use crate::infrastructure::scan::ScanCoordinator;
    use std::fs::File;
    use tempfile::tempdir;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn scan_paths_via_gateway_starts_scan() {
        let coordinator = ScanCoordinator::new(MetaCache::new(":memory:").unwrap());
        let dir = tempdir().unwrap();
        File::create(dir.path().join("track.mp3")).unwrap();

        let handle = scan_paths(&coordinator, vec![dir.path().to_path_buf()], true)
            .await
            .expect("scan_paths");

        let mut progress_rx = coordinator.subscribe_progress(&handle);
        let progress = timeout(Duration::from_secs(3), progress_rx.recv())
            .await
            .expect("expected a progress event");

        assert!(progress.is_ok());
    }
}
