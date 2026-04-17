use crate::infrastructure::scan::io_worker::{spawn_io_worker, Progress, ScanHandle, ScanOptions};
use anyhow::Result;
use tokio::sync::broadcast;

pub struct ScanWorker {
    inner: crate::infrastructure::scan::io_worker::IoWorkerHandle,
}

impl ScanWorker {
    pub fn new() -> Self {
        Self::new_with_db_path(None)
    }

    pub fn new_with_db_path(db_path: Option<std::path::PathBuf>) -> Self {
        Self {
            inner: spawn_io_worker(db_path),
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


