// Placeholder for sqlite_store implementation (optional future store)
// Will implement a Store trait and provide SQLite-backed persistence.

pub struct SqliteStore;

impl SqliteStore {
    pub fn open<P: AsRef<std::path::Path>>(_path: P) -> anyhow::Result<Self> {
        Ok(SqliteStore)
    }
}
