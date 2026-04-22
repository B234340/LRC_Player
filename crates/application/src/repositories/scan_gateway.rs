use crate::infrastructure::scan::{Progress, ScanHandle, ScanOptions};
use anyhow::Result;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use tokio::sync::broadcast;

/// Application port for scanning the library.
pub trait ScanGateway {
    fn start_scan<'a>(
        &'a self,
        paths: Vec<PathBuf>,
        options: ScanOptions,
    ) -> Pin<Box<dyn Future<Output = Result<ScanHandle>> + 'a>>;

    fn cancel_scan<'a>(
        &'a self,
        handle: &'a ScanHandle,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

    fn subscribe_progress(&self, handle: &ScanHandle) -> broadcast::Receiver<Progress>;

    fn checkpoint_now<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>>;
}
