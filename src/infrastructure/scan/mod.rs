pub mod io_worker;
pub mod file_scanner;
pub mod scan_coordinator;
pub mod scan_worker;

use crate::infrastructure::persistence::MetaCache;
use anyhow::Result;

pub use self::io_worker::{Progress, ScanHandle, ScanOptions};

pub struct ScanCoordinator {
    worker: io_worker::IoWorkerHandle,
    _meta: MetaCache,
}

impl ScanCoordinator {
    pub fn new(meta: MetaCache) -> Self {
        // spawn io_worker with in-memory DB for now; integration with on-disk DB path can be added
        let worker = io_worker::spawn_io_worker(None);
        Self {
            worker,
            _meta: meta,
        }
    }

    /// Start a scan for given paths. Returns a `ScanHandle` to control/observe the scan.
    pub fn start_scan(&self, paths: Vec<std::path::PathBuf>, options: ScanOptions) -> ScanHandle {
        self.worker.start_scan(paths, options)
    }

    pub fn cancel_scan(&self, handle: &ScanHandle) -> Result<()> {
        self.worker.cancel(handle)
    }

    pub fn subscribe_progress(&self, handle: &ScanHandle) -> tokio::sync::mpsc::Receiver<Progress> {
        self.worker.subscribe(handle)
    }

    pub fn checkpoint_now(&self, _handle: &ScanHandle) -> Result<()> {
        // Placeholder: forward to worker for immediate checkpoint
        Ok(())
    }
}
