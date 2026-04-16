pub mod file_scanner;
pub mod io_worker;
pub mod scan_coordinator;
pub mod scan_worker;

use crate::infrastructure::persistence::MetaCache;
use anyhow::Result;

pub use self::io_worker::{Progress, ScanHandle, ScanOptions};
pub use self::scan_worker::ScanWorker;

pub struct ScanCoordinator {
    worker: ScanWorker,
    _meta: MetaCache,
}

impl ScanCoordinator {
    pub fn new(meta: MetaCache) -> Self {
        // spawn ScanWorker with an in-memory DB for now; integration with on-disk DB path can be added
        let worker = ScanWorker::new();
        Self {
            worker,
            _meta: meta,
        }
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

impl crate::application::repositories::scan_gateway::ScanGateway for ScanCoordinator {
    fn start_scan<'a>(
        &'a self,
        paths: Vec<std::path::PathBuf>,
        options: ScanOptions,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ScanHandle>> + 'a>> {
        Box::pin(crate::infrastructure::scan::ScanCoordinator::start_scan(
            self, paths, options,
        ))
    }

    fn cancel_scan<'a>(
        &'a self,
        handle: &'a ScanHandle,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(crate::infrastructure::scan::ScanCoordinator::cancel_scan(
            self, handle,
        ))
    }

    fn subscribe_progress(
        &self,
        handle: &ScanHandle,
    ) -> tokio::sync::broadcast::Receiver<Progress> {
        crate::infrastructure::scan::ScanCoordinator::subscribe_progress(self, handle)
    }

    fn checkpoint_now<'a>(
        &'a self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(crate::infrastructure::scan::ScanCoordinator::checkpoint_now(self))
    }
}
