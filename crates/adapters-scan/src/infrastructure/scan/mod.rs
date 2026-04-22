pub mod file_scanner;
pub mod io_worker;
pub mod scan_coordinator;
pub mod scan_worker;

use lrc_player_adapters_persistence::persistence::MetaCache;
use anyhow::Result;

pub use self::io_worker::{Progress, ScanHandle, ScanOptions};
pub use self::scan_worker::ScanWorker;

pub struct ScanCoordinator {
    worker: ScanWorker,
    _meta: MetaCache,
}

impl ScanCoordinator {
    pub fn new(meta: MetaCache) -> Self {
        let worker = ScanWorker::new_with_db_path(Some(meta.path().to_path_buf()));
        Self { worker, _meta: meta }
    }

    /// Start a scan for given paths. Returns a `ScanHandle` to control/observe the scan.
    pub async fn start_scan(
        &self,
        paths: Vec<std::path::PathBuf>,
        options: ScanOptions,
    ) -> Result<ScanHandle> {
        self.worker.start_scan(paths, options).await
    }

    pub async fn cancel_scan(&self, handle: &ScanHandle) -> Result<()> {
        self.worker.cancel_scan(handle).await
    }

    pub fn subscribe_progress(
        &self,
        handle: &ScanHandle,
    ) -> tokio::sync::broadcast::Receiver<Progress> {
        self.worker.subscribe_progress(handle)
    }

    pub async fn checkpoint_now(&self) -> Result<()> {
        self.worker.checkpoint_now().await
    }
}
