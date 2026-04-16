use crate::infrastructure::scan::io_worker::{spawn_io_worker, Progress, ScanHandle, ScanOptions};
use anyhow::Result;
use tokio::sync::broadcast;

pub struct ScanWorker {
    inner: crate::infrastructure::scan::io_worker::IoWorkerHandle,
}

impl ScanWorker {
    pub fn new() -> Self {
        Self {
            inner: spawn_io_worker(None),
        }
    }
}

impl Default for ScanWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl ScanWorker {
    pub async fn start_scan(
        &self,
        paths: Vec<std::path::PathBuf>,
        options: ScanOptions,
    ) -> Result<ScanHandle> {
        self.inner.start_scan(paths, options).await
    }

    pub async fn cancel_scan(&self, handle: &ScanHandle) -> Result<()> {
        self.inner.cancel(handle).await
    }

    pub fn subscribe_progress(&self, handle: &ScanHandle) -> broadcast::Receiver<Progress> {
        self.inner.subscribe(handle)
    }

    pub async fn checkpoint_now(&self) -> Result<()> {
        self.inner.checkpoint_now().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            .start_scan(vec![root.to_path_buf()], ScanOptions { fast: true })
            .await
            .expect("start scan");
        let mut rx = worker.subscribe_progress(&handle);

        let progress = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await;
        assert!(progress.is_ok(), "expected at least one progress event");
    }
}
