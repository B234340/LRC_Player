use crate::infrastructure::persistence::MetaCache;
use anyhow::Result;
use std::path::Path;

/// Application-level repository for library metadata and file records.
pub trait LibraryRepository {
    fn upsert_file(
        &self,
        path: &str,
        norm_path: &str,
        size: i64,
        mtime: i64,
        device: Option<i64>,
        inode: Option<i64>,
        stable_key: Option<&str>,
    ) -> Result<()>;

    fn checkpoint(&self) -> Result<()>;

    fn get_file_id(&self, path: &str) -> Result<Option<i64>>;

    fn get_file_id_for_path<P: AsRef<Path>>(&self, path: P) -> Result<Option<i64>> {
        self.get_file_id(path.as_ref().to_string_lossy().as_ref())
    }
}

impl LibraryRepository for MetaCache {
    fn upsert_file(
        &self,
        path: &str,
        norm_path: &str,
        size: i64,
        mtime: i64,
        device: Option<i64>,
        inode: Option<i64>,
        stable_key: Option<&str>,
    ) -> Result<()> {
        self.upsert_file(path, norm_path, size, mtime, device, inode, stable_key)
    }

    fn checkpoint(&self) -> Result<()> {
        self.checkpoint()
    }

    fn get_file_id(&self, path: &str) -> Result<Option<i64>> {
        self.get_file_id(path)
    }
}
