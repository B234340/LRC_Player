use crate::repositories::ScanGateway;
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


