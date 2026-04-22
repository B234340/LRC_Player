#[cfg(test)]
    mod tests {
        use lrc_player::shared::ipc::*;
        use std::time::Duration;
        use tempfile::tempdir;
        use tokio::runtime::Runtime;
        use tokio::sync::mpsc;

        #[test]
        fn server_receives_scan_requests() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("ipc_test.sock");
            let rt = Runtime::new().unwrap();

            rt.block_on(async {
                let (tx, mut rx) = mpsc::channel(1);
                start_server(&path, tx).await.unwrap();
                tokio::time::sleep(Duration::from_millis(10)).await;

                let request = ScanRequest {
                    paths: vec!["/tmp/scan1".into()],
                    lrc_paths: Vec::new(),
                    excludes: Vec::new(),
                    cancel: false,
                };
                send_scan_request(&path, &request).await.unwrap();

                let received = tokio::time::timeout(Duration::from_secs(1), rx.recv())
                    .await
                    .unwrap()
                    .unwrap();

                assert_eq!(received.paths, request.paths);
            });
        }
    }