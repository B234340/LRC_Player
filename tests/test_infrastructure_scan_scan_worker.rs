#[cfg(test)]
mod tests {
    use lrc_player::infrastructure::scan::io_worker::ScanOptions;
    use lrc_player::infrastructure::scan::scan_worker::*;
    use std::fs::File;
    use tempfile::tempdir;
    use tokio::time::Duration;

    #[tokio::test]
    async fn scan_worker_can_start_and_report_progress() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        File::create(root.join("track.mp3")).unwrap();

        let worker = ScanWorker::new();
        let handle = worker
            .start_scan(
                vec![root.to_path_buf()],
                ScanOptions {
                    fast: true,
                    excludes: Vec::new(),
                },
            )
            .await
            .expect("start scan");
        let mut rx = worker.subscribe_progress(&handle);

        let progress = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await;
        assert!(progress.is_ok(), "expected at least one progress event");
    }
}
