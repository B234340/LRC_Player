use anyhow::Context;
use rusqlite::{params, Connection, OpenFlags};
use std::path::Path;

pub struct MetaCache {
    conn: Connection,
}

impl MetaCache {
    /// Open or create a meta cache SQLite database at `path`.
    /// Use WAL mode and set pragmatic defaults suitable for this project.
    pub fn new<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX;

        let conn = Connection::open_with_flags(path, flags)
            .with_context(|| format!("opening sqlite db at {}", path.display()))?;

        // Recommended PRAGMAs for WAL and performance
        conn.pragma_update(None, "journal_mode", "WAL")
            .context("enable WAL journal_mode")?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .context("set synchronous to NORMAL")?;

        let cache = MetaCache { conn };
        cache.init_schema()?;
        Ok(cache)
    }

    /// Initialize schema (DDL). Safe to call multiple times.
    fn init_schema(&self) -> anyhow::Result<()> {
        const DDL: &str = r#"
BEGIN TRANSACTION;
CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    norm_path TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    device INTEGER,
    inode INTEGER,
    stable_key TEXT,
    tag_title TEXT,
    tag_artist TEXT,
    tag_album TEXT,
    parsed_at INTEGER,
    status TEXT DEFAULT 'enumerated'
);
CREATE INDEX IF NOT EXISTS idx_files_stable_key ON files(stable_key);

CREATE TABLE IF NOT EXISTS tracks (
    content_id TEXT PRIMARY KEY,
    title TEXT,
    artist TEXT,
    album TEXT,
    canonical_location_id INTEGER,
    created_at INTEGER
);

CREATE TABLE IF NOT EXISTS locations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id TEXT REFERENCES tracks(content_id),
    path TEXT NOT NULL,
    device INTEGER,
    inode INTEGER,
    size INTEGER,
    mtime INTEGER,
    last_seen INTEGER,
    preferred BOOLEAN DEFAULT 0,
    available BOOLEAN DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_locations_content ON locations(content_id);

CREATE TABLE IF NOT EXISTS hashes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id TEXT REFERENCES tracks(content_id),
    algo TEXT,
    full_hash TEXT,
    partial_hash TEXT,
    computed_at INTEGER
);

-- Staging tables (same schema as main but with UNIQUE path for upsert semantics)
CREATE TABLE IF NOT EXISTS files_staging (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    norm_path TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    device INTEGER,
    inode INTEGER,
    stable_key TEXT,
    tag_title TEXT,
    tag_artist TEXT,
    tag_album TEXT,
    parsed_at INTEGER,
    status TEXT DEFAULT 'enumerated'
);

CREATE TABLE IF NOT EXISTS tracks_staging (
    content_id TEXT PRIMARY KEY,
    title TEXT,
    artist TEXT,
    album TEXT,
    canonical_location_id INTEGER,
    created_at INTEGER
);

CREATE TABLE IF NOT EXISTS locations_staging (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id TEXT REFERENCES tracks(content_id),
    path TEXT NOT NULL,
    device INTEGER,
    inode INTEGER,
    size INTEGER,
    mtime INTEGER,
    last_seen INTEGER,
    preferred BOOLEAN DEFAULT 0,
    available BOOLEAN DEFAULT 1
);

CREATE TABLE IF NOT EXISTS hashes_staging (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id TEXT REFERENCES tracks(content_id),
    algo TEXT,
    full_hash TEXT,
    partial_hash TEXT,
    computed_at INTEGER
);

COMMIT;
"#;

        self.conn
            .execute_batch(DDL)
            .context("creating meta cache schema")?;

        Ok(())
    }

    /// Example helper: insert or replace a `files` row based on path
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_file(
        &self,
        path: &str,
        norm_path: &str,
        size: i64,
        mtime: i64,
        device: Option<i64>,
        inode: Option<i64>,
        stable_key: Option<&str>,
    ) -> anyhow::Result<()> {
        // compute current unix timestamp in seconds without adding a chrono dependency
        let now_ts: i64 = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(_) => 0,
        };

        self.conn.execute(
            "INSERT INTO files (path,norm_path,size,mtime,device,inode,stable_key,parsed_at,status)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(path) DO UPDATE SET
               norm_path=excluded.norm_path,
               size=excluded.size,
               mtime=excluded.mtime,
               device=excluded.device,
               inode=excluded.inode,
               stable_key=excluded.stable_key,
               parsed_at=excluded.parsed_at,
               status=excluded.status
            ",
            params![path, norm_path, size, mtime, device, inode, stable_key, now_ts, "enumerated"],
        )?;

        Ok(())
    }

    /// Upsert into staging table. Used during scanning; changes are later checkpointed to main tables.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_file_staging(
        &self,
        path: &str,
        norm_path: &str,
        size: i64,
        mtime: i64,
        device: Option<i64>,
        inode: Option<i64>,
        stable_key: Option<&str>,
    ) -> anyhow::Result<()> {
        // compute current unix timestamp in seconds without adding a chrono dependency
        let now_ts: i64 = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(_) => 0,
        };

        self.conn.execute(
            "INSERT INTO files_staging (path,norm_path,size,mtime,device,inode,stable_key,parsed_at,status)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(path) DO UPDATE SET
               norm_path=excluded.norm_path,
               size=excluded.size,
               mtime=excluded.mtime,
               device=excluded.device,
               inode=excluded.inode,
               stable_key=excluded.stable_key,
               parsed_at=excluded.parsed_at,
               status=excluded.status
            ",
            params![path, norm_path, size, mtime, device, inode, stable_key, now_ts, "enumerated"],
        )?;

        Ok(())
    }

    /// Checkpoint: atomically move staging tables into main tables and clear staging.
    pub fn checkpoint(&self) -> anyhow::Result<()> {
        // Use INSERT OR REPLACE for compatibility with older SQLite builds in test
        // environments. This will replace rows with the same `path` value.
        let sql = r#"
    BEGIN TRANSACTION;
    INSERT OR REPLACE INTO files (path,norm_path,size,mtime,device,inode,stable_key,tag_title,tag_artist,tag_album,parsed_at,status)
    SELECT path,norm_path,size,mtime,device,inode,stable_key,tag_title,tag_artist,tag_album,parsed_at,status FROM files_staging;
    DELETE FROM files_staging;
    COMMIT;
    "#;

        self.conn.execute_batch(sql)?;
        Ok(())
    }
    /// Basic query: get file id by path
    pub fn get_file_id(&self, path: &str) -> anyhow::Result<Option<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM files WHERE path = ?1")?;
        let mut rows = stmt.query(params![path])?;
        if let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    /// Expose a raw connection reference for advanced ops (use carefully)
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_in_memory() {
        let cache = MetaCache::new(":memory:").expect("create in-memory db");
        // schema should exist
        let row: i64 = cache
            .connection()
            .query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| r.get(0))
            .expect("query sqlite_master");
        assert!(row >= 1);
    }

    #[test]
    fn upsert_staging_and_checkpoint_moves_to_main() {
        let cache = MetaCache::new(":memory:").expect("create in-memory db");

        // insert into staging
        cache
            .upsert_file_staging("/tmp/song1.mp3", "/tmp/song1.mp3", 1234, 42, None, None, None)
            .expect("upsert staging");

        // ensure staging has 1 row
        let staging_count: i64 = cache
            .connection()
            .query_row("SELECT count(*) FROM files_staging", [], |r| r.get(0))
            .expect("count staging");
        assert_eq!(staging_count, 1);

        // ensure main files table is empty
        let main_count: i64 = cache
            .connection()
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
            .expect("count files");
        assert_eq!(main_count, 0);

        // checkpoint should move staging -> main and clear staging
        cache.checkpoint().expect("checkpoint");

        let main_count_after: i64 = cache
            .connection()
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
            .expect("count files after");
        assert_eq!(main_count_after, 1);

        let staging_count_after: i64 = cache
            .connection()
            .query_row("SELECT count(*) FROM files_staging", [], |r| r.get(0))
            .expect("count staging after");
        assert_eq!(staging_count_after, 0);

        // get_file_id should return Some
        let id = cache.get_file_id("/tmp/song1.mp3").expect("get id");
        assert!(id.is_some());
    }

    #[test]
    fn upsert_file_and_get_file_id() {
        let cache = MetaCache::new(":memory:").expect("create in-memory db");
        cache
            .upsert_file("/tmp/song2.mp3", "/tmp/song2.mp3", 4321, 43, None, None, None)
            .expect("upsert file");
        let id = cache.get_file_id("/tmp/song2.mp3").expect("get id");
        assert!(id.is_some());
    }
}
