#[cfg(test)]
mod tests {
    use lrc_player::infrastructure::persistence::MetaCache;
    use lrc_player::infrastructure::scan::io_worker::*;
    use std::fs::File;
    use tempfile::tempdir;
    use tokio::time::{sleep, timeout, Duration};

    #[tokio::test]
    async fn spawn_and_start() {
        let w = spawn_io_worker(None);
        let handle = w
            .start_scan(
                vec![],
                ScanOptions {
                    fast: true,
                    excludes: Vec::new(),
                },
            )
            .await
            .expect("start scan");
        let res = w.cancel(&handle).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn checkpoint_writes_scanned_file_to_meta_cache() {
        let db_dir = tempdir().unwrap();
        let db_path = db_dir.path().join("scan.db");
        let scan_dir = tempdir().unwrap();
        let test_file = scan_dir.path().join("track.mp3");
        let mut file = File::create(&test_file).unwrap();
        use std::io::Write;
        file.write_all(b"test audio content").unwrap();

        let worker = spawn_io_worker(Some(db_path.clone()));
        let handle = worker
            .start_scan(
                vec![scan_dir.path().to_path_buf()],
                ScanOptions {
                    fast: true,
                    excludes: Vec::new(),
                },
            )
            .await
            .expect("start scan");
        let mut progress_rx = worker.subscribe(&handle);

        let _ = timeout(Duration::from_secs(3), progress_rx.recv())
            .await
            .expect("expected at least one progress event");

        worker.checkpoint_now().await.expect("checkpoint send");
        sleep(Duration::from_secs(1)).await;

        let cache = MetaCache::new(db_path).expect("open db");
        let id = cache
            .get_file_id(test_file.to_string_lossy().as_ref())
            .expect("query file id");
        assert!(id.is_some(), "scanned file should appear in metadata cache");
    }
}
