use crate::infrastructure::persistence::MetaCache;
use anyhow::Result;
use std::path::Path;

/// Application-level repository for library metadata and file records.
#[derive(Debug, Clone)]
pub struct LibraryFileRecord {
    pub path: String,
    pub norm_path: String,
    pub size: i64,
    pub mtime: i64,
    pub device: Option<i64>,
    pub inode: Option<i64>,
    pub stable_key: Option<String>,
}

impl LibraryFileRecord {
    pub fn new(
        path: impl Into<String>,
        norm_path: impl Into<String>,
        size: i64,
        mtime: i64,
        device: Option<i64>,
        inode: Option<i64>,
        stable_key: Option<String>,
    ) -> Self {
        Self {
            path: path.into(),
            norm_path: norm_path.into(),
            size,
            mtime,
            device,
            inode,
            stable_key,
        }
    }
}

pub trait LibraryRepository {
    fn upsert_file(&self, record: LibraryFileRecord) -> Result<()>;

    fn checkpoint(&self) -> Result<()>;

    fn get_file_id(&self, path: &str) -> Result<Option<i64>>;

    fn get_file_id_for_path<P: AsRef<Path>>(&self, path: P) -> Result<Option<i64>> {
        self.get_file_id(path.as_ref().to_string_lossy().as_ref())
    }
}

impl LibraryRepository for MetaCache {
    fn upsert_file(&self, record: LibraryFileRecord) -> Result<()> {
        self.upsert_file(
            &record.path,
            &record.norm_path,
            record.size,
            record.mtime,
            record.device,
            record.inode,
            record.stable_key.as_deref(),
        )
    }

    fn checkpoint(&self) -> Result<()> {
        self.checkpoint()
    }

    fn get_file_id(&self, path: &str) -> Result<Option<i64>> {
        self.get_file_id(path)
    }
}
