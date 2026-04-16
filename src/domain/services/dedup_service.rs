use crate::infrastructure::dedup::{
    compute_partial_hash, DedupService as InfrastructureDedupService,
};
use anyhow::Result;
use std::path::{Path, PathBuf};
use tokio::sync::oneshot;

#[derive(Clone, Debug)]
pub struct DedupService {
    inner: InfrastructureDedupService,
}

impl Default for DedupService {
    fn default() -> Self {
        Self::new()
    }
}

impl DedupService {
    pub fn new() -> Self {
        Self {
            inner: InfrastructureDedupService::spawn(),
        }
    }

    pub fn compute_stable_key(&self, path: &Path) -> Result<String> {
        compute_partial_hash(&path.to_path_buf(), 16)
    }

    pub async fn request_full_hash(&self, path: PathBuf) -> oneshot::Receiver<Result<String>> {
        self.inner.request_full_hash(path).await
    }
}
