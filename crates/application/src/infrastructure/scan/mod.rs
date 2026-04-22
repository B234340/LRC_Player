pub use lrc_player_adapters_scan::infrastructure::scan::*;

impl crate::repositories::scan_gateway::ScanGateway for ScanCoordinator {
    fn start_scan<'a>(
        &'a self,
        paths: Vec<std::path::PathBuf>,
        options: ScanOptions,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScanHandle>> + 'a>> {
        Box::pin(crate::infrastructure::scan::ScanCoordinator::start_scan(self, paths, options))
    }

    fn cancel_scan<'a>(
        &'a self,
        handle: &'a ScanHandle,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + 'a>> {
        Box::pin(crate::infrastructure::scan::ScanCoordinator::cancel_scan(self, handle))
    }

    fn subscribe_progress(&self, handle: &ScanHandle) -> tokio::sync::broadcast::Receiver<Progress> {
        crate::infrastructure::scan::ScanCoordinator::subscribe_progress(self, handle)
    }

    fn checkpoint_now<'a>(&'a self) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + 'a>> {
        Box::pin(crate::infrastructure::scan::ScanCoordinator::checkpoint_now(self))
    }
}
