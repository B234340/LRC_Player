use lrc_player_shared::dto::LibraryDirectoryDto;
use anyhow::Context;
use rusqlite::{params, Connection, OpenFlags};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf, MAIN_SEPARATOR};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::Duration;

/// Bump when the DDL in `init_schema` changes. Opening a database whose
/// `PRAGMA user_version` is already at this value skips the DDL batch and the
/// `PRAGMA table_info` column checks, which makes `MetaCache::new` cheap.
const SCHEMA_VERSION: i64 = 2;

/// Busy timeout used by `MetaCache::new` (scanner / writer connections).
const WRITER_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Busy timeout used for the shared, UI-facing handle (`MetaCache::shared`).
/// In WAL mode readers practically never wait, so this only bounds the rare
/// case (WAL recovery, a write from the UI racing the scanner's commit). A
/// UI thread will never stall longer than this on a lock.
pub const UI_BUSY_TIMEOUT: Duration = Duration::from_millis(200);

/// Prepared-statement cache size per connection (`prepare_cached`).
const STATEMENT_CACHE_CAPACITY: usize = 64;

/// SQLite's default host-parameter limit is 32766 on the bundled build; stay
/// far below it so a batch lookup works on any build.
const BATCH_LOOKUP_CHUNK: usize = 500;

/// Display metadata for a track location, as returned by
/// [`MetaCache::track_metadata_many`]. Missing values are empty strings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackMetadataRow {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub composer: String,
}

pub struct MetaCache {
    conn: Connection,
    path: PathBuf,
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
        conn.busy_timeout(WRITER_BUSY_TIMEOUT)
            .context("set sqlite busy timeout")?;
        conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE_CAPACITY);

        let cache = MetaCache {
            conn,
            path: path.to_path_buf(),
        };
        // The DDL batch + 4x `PRAGMA table_info` used to run on every open.
        // Only run it when the schema version is behind.
        let user_version: i64 = cache
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .context("read schema user_version")?;
        if user_version < SCHEMA_VERSION {
            cache.init_schema()?;
            cache
                .conn
                .pragma_update(None, "user_version", SCHEMA_VERSION)
                .context("set schema user_version")?;
        }
        Ok(cache)
    }

    /// Change the busy timeout of this connection. UI-thread connections should
    /// use a short timeout (see [`UI_BUSY_TIMEOUT`]) so a scanner holding the
    /// write lock can never freeze the UI for seconds.
    pub fn set_busy_timeout(&self, timeout: Duration) -> anyhow::Result<()> {
        self.conn
            .busy_timeout(timeout)
            .context("set sqlite busy timeout")?;
        Ok(())
    }

    /// `PRAGMA data_version`: changes whenever *another* connection (e.g. the
    /// scanner) commits to the database. Cheap (no table access); use it to
    /// decide whether cached library data must be reloaded. Writes made through
    /// this same connection do NOT change it.
    pub fn data_version(&self) -> anyhow::Result<i64> {
        let version = self
            .conn
            .query_row("PRAGMA data_version", [], |row| row.get(0))
            .context("read data_version")?;
        Ok(version)
    }

    /// Return a process-wide shared handle to the default meta cache database.
    ///
    /// The connection is opened once (on first call) with a short busy timeout
    /// ([`UI_BUSY_TIMEOUT`]) and reused afterwards, so per-frame / per-row
    /// callers no longer pay for opening SQLite and running PRAGMAs. If opening
    /// fails, the error is returned and the next call retries.
    ///
    /// Recommended usage on the UI thread: keep the returned handle in the app
    /// struct, call [`SharedMetaCache::try_with`] (never blocks; returns `None`
    /// if a worker is using the connection) and keep showing the previous data
    /// when it returns `None`. Long-running work (scans, bulk writes) should
    /// open its own connection with [`MetaCache::new`] instead of holding the
    /// shared one.
    pub fn shared() -> anyhow::Result<SharedMetaCache> {
        static SHARED: OnceLock<Mutex<Option<SharedMetaCache>>> = OnceLock::new();
        let slot = SHARED.get_or_init(|| Mutex::new(None));
        let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(handle) = guard.as_ref() {
            return Ok(handle.clone());
        }
        let handle = SharedMetaCache::open(Self::default_path()?)?;
        *guard = Some(handle.clone());
        Ok(handle)
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
    album_artist TEXT,
    composer TEXT,
    album TEXT,
    canonical_location_id INTEGER,
    created_at INTEGER
);

CREATE TABLE IF NOT EXISTS locations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id TEXT REFERENCES tracks(content_id),
    path TEXT NOT NULL UNIQUE,
    device INTEGER,
    inode INTEGER,
    size INTEGER,
    mtime INTEGER,
    last_seen INTEGER,
    preferred BOOLEAN DEFAULT 0,
    available BOOLEAN DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_locations_content ON locations(content_id);
CREATE INDEX IF NOT EXISTS idx_locations_available_last_seen ON locations(available, last_seen);

CREATE TABLE IF NOT EXISTS hashes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id TEXT REFERENCES tracks(content_id),
    algo TEXT,
    full_hash TEXT,
    partial_hash TEXT,
    computed_at INTEGER
);

CREATE TABLE IF NOT EXISTS lyrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id TEXT REFERENCES tracks(content_id),
    path TEXT NOT NULL UNIQUE,
    size INTEGER,
    mtime INTEGER,
    last_seen INTEGER,
    available BOOLEAN DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_lyrics_content ON lyrics(content_id);

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
    album_artist TEXT,
    composer TEXT,
    album TEXT,
    canonical_location_id INTEGER,
    created_at INTEGER
);

CREATE TABLE IF NOT EXISTS locations_staging (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id TEXT,
    path TEXT NOT NULL UNIQUE,
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
    content_id TEXT,
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

        self.ensure_track_columns()?;

        Ok(())
    }

    fn ensure_track_columns(&self) -> anyhow::Result<()> {
        self.ensure_column("tracks", "album_artist", "TEXT")?;
        self.ensure_column("tracks", "composer", "TEXT")?;
        self.ensure_column("tracks_staging", "album_artist", "TEXT")?;
        self.ensure_column("tracks_staging", "composer", "TEXT")?;
        Ok(())
    }

    fn ensure_column(&self, table: &str, column: &str, column_def: &str) -> anyhow::Result<()> {
        let pragma = format!("PRAGMA table_info({})", table);
        let mut stmt = self.conn.prepare(&pragma)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        for name in rows {
            if name? == column {
                return Ok(());
            }
        }

        let alter = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, column_def);
        self.conn.execute_batch(&alter)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn default_path() -> anyhow::Result<PathBuf> {
        let exe_path = std::env::current_exe().context("unable to determine current executable path")?;
        let exe_dir = exe_path
            .parent()
            .context("current executable has no parent directory")?;
        let cache_dir = exe_dir.join("data").join("cache");
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("creating cache directory {}", cache_dir.display()))?;
        Ok(cache_dir.join("meta_cache.db"))
    }

    pub fn open_default() -> anyhow::Result<Self> {
        let path = Self::default_path()?;
        Self::new(path)
    }

    /// Example helper: insert or replace a `lyrics` row based on path
    pub fn upsert_lyrics_location(
        &self,
        content_id: Option<&str>,
        path: &str,
        size: i64,
        mtime: i64,
        last_seen: i64,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO lyrics (content_id, path, size, mtime, last_seen, available)
             VALUES (?1, ?2, ?3, ?4, ?5, 1)
             ON CONFLICT(path) DO UPDATE SET
               content_id = excluded.content_id,
               size = excluded.size,
               mtime = excluded.mtime,
               last_seen = excluded.last_seen,
               available = excluded.available",
            params![content_id, path, size, mtime, last_seen],
        )?;
        Ok(())
    }

    pub fn find_track_content_id_by_path(&self, file_path: &Path) -> anyhow::Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT content_id FROM locations WHERE path = ?1 LIMIT 1")?;
        let mut rows = stmt.query(params![file_path.to_string_lossy()])?;
        if let Some(row) = rows.next()? {
            Ok(row.get::<_, Option<String>>(0)?)
        } else {
            Ok(None)
        }
    }

    pub fn find_track_content_id_by_file_stem_and_parent(
        &self,
        file_stem: &str,
        parent_name: &str,
    ) -> anyhow::Result<Option<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT l.content_id, l.path FROM locations l JOIN tracks t ON t.content_id = l.content_id",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        for row in rows {
            let (content_id, path_str) = row?;
            let path = Path::new(&path_str);
            if path.file_stem().and_then(|s| s.to_str()) == Some(file_stem) {
                if let Some(parent) = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
                    if parent == parent_name {
                        return Ok(Some(content_id));
                    }
                }
            }
        }
        Ok(None)
    }

    pub fn lyrics_for_track_path(&self, file_path: &Path) -> anyhow::Result<Vec<String>> {
        if let Some(content_id) = self.find_track_content_id_by_path(file_path)? {
            let mut stmt = self.conn.prepare_cached(
                "SELECT path FROM lyrics WHERE content_id = ?1 AND available = 1",
            )?;
            let rows = stmt.query_map(params![content_id], |row| row.get(0))?;
            let mut paths = Vec::new();
            for path in rows {
                paths.push(path?);
            }
            Ok(paths)
        } else {
            Ok(Vec::new())
        }
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
            params![
                path,
                norm_path,
                size,
                mtime,
                device,
                inode,
                stable_key,
                now_ts,
                "enumerated"
            ],
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

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_track(
        &self,
        content_id: &str,
        title: Option<&str>,
        artist: Option<&str>,
        album_artist: Option<&str>,
        composer: Option<&str>,
        album: Option<&str>,
        canonical_location_id: Option<i64>,
        created_at: i64,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO tracks (content_id,title,artist,album_artist,composer,album,canonical_location_id,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(content_id) DO UPDATE SET
               title=excluded.title,
               artist=excluded.artist,
               album_artist=excluded.album_artist,
               composer=excluded.composer,
               album=excluded.album,
               canonical_location_id=excluded.canonical_location_id,
               created_at=excluded.created_at
            ",
            params![
                content_id,
                title,
                artist,
                album_artist,
                composer,
                album,
                canonical_location_id,
                created_at,
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_track_staging(
        &self,
        content_id: &str,
        title: Option<&str>,
        artist: Option<&str>,
        album_artist: Option<&str>,
        composer: Option<&str>,
        album: Option<&str>,
        canonical_location_id: Option<i64>,
        created_at: i64,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO tracks_staging (content_id,title,artist,album_artist,composer,album,canonical_location_id,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(content_id) DO UPDATE SET
               title=excluded.title,
               artist=excluded.artist,
               album_artist=excluded.album_artist,
               composer=excluded.composer,
               album=excluded.album,
               canonical_location_id=excluded.canonical_location_id,
               created_at=excluded.created_at
            ",
            params![
                content_id,
                title,
                artist,
                album_artist,
                composer,
                album,
                canonical_location_id,
                created_at,
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_location(
        &self,
        content_id: &str,
        path: &str,
        device: Option<i64>,
        inode: Option<i64>,
        size: i64,
        mtime: i64,
        last_seen: i64,
        preferred: bool,
        available: bool,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO locations (content_id,path,device,inode,size,mtime,last_seen,preferred,available)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(path) DO UPDATE SET
               content_id=excluded.content_id,
               device=excluded.device,
               inode=excluded.inode,
               size=excluded.size,
               mtime=excluded.mtime,
               last_seen=excluded.last_seen,
               preferred=excluded.preferred,
               available=excluded.available
            ",
            params![
                content_id,
                path,
                device,
                inode,
                size,
                mtime,
                last_seen,
                preferred,
                available,
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_location_staging(
        &self,
        content_id: &str,
        path: &str,
        device: Option<i64>,
        inode: Option<i64>,
        size: i64,
        mtime: i64,
        last_seen: i64,
        preferred: bool,
        available: bool,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO locations_staging (content_id,path,device,inode,size,mtime,last_seen,preferred,available)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(path) DO UPDATE SET
               content_id=excluded.content_id,
               device=excluded.device,
               inode=excluded.inode,
               size=excluded.size,
               mtime=excluded.mtime,
               last_seen=excluded.last_seen,
               preferred=excluded.preferred,
               available=excluded.available
            ",
            params![
                content_id,
                path,
                device,
                inode,
                size,
                mtime,
                last_seen,
                preferred,
                available,
            ],
        )?;
        Ok(())
    }

    pub fn upsert_hash(
        &self,
        content_id: &str,
        algo: &str,
        full_hash: Option<&str>,
        partial_hash: Option<&str>,
        computed_at: i64,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO hashes (content_id,algo,full_hash,partial_hash,computed_at)
             VALUES (?1,?2,?3,?4,?5)",
            params![content_id, algo, full_hash, partial_hash, computed_at],
        )?;
        Ok(())
    }

    pub fn upsert_hash_staging(
        &self,
        content_id: &str,
        algo: &str,
        full_hash: Option<&str>,
        partial_hash: Option<&str>,
        computed_at: i64,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO hashes_staging (content_id,algo,full_hash,partial_hash,computed_at)
             VALUES (?1,?2,?3,?4,?5)
             ",
            params![content_id, algo, full_hash, partial_hash, computed_at],
        )?;
        Ok(())
    }

    /// Checkpoint: atomically move staging tables into main tables and clear staging.
    pub fn checkpoint(&self) -> anyhow::Result<()> {
        // Use INSERT OR REPLACE for compatibility with older SQLite builds in test
        // environments. This will replace rows with the same primary key or unique key.
        let sql = r#"
    BEGIN TRANSACTION;
    INSERT OR REPLACE INTO files (path,norm_path,size,mtime,device,inode,stable_key,tag_title,tag_artist,tag_album,parsed_at,status)
    SELECT path,norm_path,size,mtime,device,inode,stable_key,tag_title,tag_artist,tag_album,parsed_at,status FROM files_staging;
    DELETE FROM files_staging;

    INSERT OR REPLACE INTO tracks (content_id,title,artist,album_artist,composer,album,canonical_location_id,created_at)
    SELECT content_id,title,artist,album_artist,composer,album,canonical_location_id,created_at FROM tracks_staging;
    DELETE FROM tracks_staging;

    INSERT OR REPLACE INTO locations (content_id,path,device,inode,size,mtime,last_seen,preferred,available)
    SELECT content_id,path,device,inode,size,mtime,last_seen,preferred,available FROM locations_staging;
    DELETE FROM locations_staging;

    INSERT INTO hashes (content_id,algo,full_hash,partial_hash,computed_at)
    SELECT content_id,algo,full_hash,partial_hash,computed_at FROM hashes_staging;
    DELETE FROM hashes_staging;
    COMMIT;
    "#;

        self.conn.execute_batch(sql)?;
        Ok(())
    }
    /// Basic query: get file id by path
    pub fn get_file_id(&self, path: &str) -> anyhow::Result<Option<i64>> {
        let mut stmt = self.conn.prepare_cached("SELECT id FROM files WHERE path = ?1")?;
        let mut rows = stmt.query(params![path])?;
        if let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    /// Return groups of duplicate files that share the same stable key.
    pub fn duplicate_groups(&self) -> anyhow::Result<Vec<(String, Vec<String>)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT stable_key, path FROM files
             WHERE stable_key IS NOT NULL
               AND stable_key IN (
                   SELECT stable_key FROM files WHERE stable_key IS NOT NULL GROUP BY stable_key HAVING COUNT(*) > 1
               )
             ORDER BY stable_key, path",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut groups = Vec::new();
        let mut current_key: Option<String> = None;
        let mut current_paths = Vec::new();

        for row in rows {
            let (key, path) = row?;
            if current_key.as_ref() != Some(&key) {
                if let Some(prev_key) = current_key.take() {
                    groups.push((prev_key, current_paths));
                    current_paths = Vec::new();
                }
                current_key = Some(key);
            }
            current_paths.push(path);
        }

        if let Some(prev_key) = current_key {
            groups.push((prev_key, current_paths));
        }

        Ok(groups)
    }

    /// Return aggregate library counts from the current meta cache.
    pub fn library_counts(&self) -> anyhow::Result<(usize, usize)> {
        let track_count: i64 = self.conn.query_row(
            "SELECT count(DISTINCT t.content_id)
             FROM tracks t
             JOIN locations l ON l.content_id = t.content_id
             WHERE l.available = 1",
            [],
            |r| r.get(0),
        )?;
        let album_count: i64 = self.conn.query_row(
            "SELECT count(DISTINCT t.album)
             FROM tracks t
             JOIN locations l ON l.content_id = t.content_id
             WHERE l.available = 1
               AND t.album IS NOT NULL AND t.album <> ''",
            [],
            |r| r.get(0),
        )?;
        Ok((album_count as usize, track_count as usize))
    }

    /// Return the most recently seen track locations from the library.
    pub fn recent_locations(&self, limit: usize) -> anyhow::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT path FROM locations WHERE available = 1 ORDER BY last_seen DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |row| row.get(0))?;
        let mut paths = Vec::new();
        for path in rows {
            paths.push(path?);
        }
        Ok(paths)
    }

    /// Return a list of recently seen library directories.
    pub fn recent_directories(&self, limit: usize) -> anyhow::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT path FROM locations WHERE available = 1 ORDER BY last_seen DESC")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut dirs = Vec::new();
        let mut seen = HashSet::new();

        for path in rows {
            let path: String = path?;
            if let Some(parent) = std::path::Path::new(&path).parent() {
                if let Some(parent_str) = parent.to_str() {
                    if seen.insert(parent_str.to_string()) {
                        dirs.push(parent_str.to_string());
                        if dirs.len() >= limit {
                            break;
                        }
                    }
                }
            }
        }

        Ok(dirs)
    }

    fn decode_album_identifier(album_id: &str) -> (&str, &str) {
        album_id.split_once('\0').unwrap_or((album_id, ""))
    }

    fn sort_location_paths(paths: &mut [String]) {
        paths.sort_by(|a, b| {
            let a_name = Path::new(a)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(a)
                .to_ascii_lowercase();
            let b_name = Path::new(b)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(b)
                .to_ascii_lowercase();
            a_name.cmp(&b_name)
        });
    }

    /// Return distinct album names in the library.
    pub fn library_albums(&self) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT t.album
             FROM tracks t
             JOIN locations l ON l.content_id = t.content_id
             WHERE l.available = 1
               AND t.album IS NOT NULL AND t.album <> ''
             ORDER BY t.album COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| Ok(row.get::<_, String>(0)?))?;
        let mut albums = Vec::new();
        for album in rows {
            albums.push(album?);
        }
        Ok(albums)
    }

    /// Return distinct artist names in the library.
    pub fn library_artists(&self) -> anyhow::Result<Vec<String>> {
        self.library_artists_by_kind("artist")
    }

    /// Return distinct names for the selected artist-like kind.
    /// kind: "artist" | "album_artist" | "composer"
    pub fn library_artists_by_kind(&self, kind: &str) -> anyhow::Result<Vec<String>> {
        let col = match kind {
            "artist" => "artist",
            "album_artist" => "album_artist",
            "composer" => "composer",
            _ => "artist",
        };
        let sql = format!(
            "SELECT DISTINCT t.{col} FROM tracks t JOIN locations l ON l.content_id = t.content_id WHERE l.available = 1 AND t.{col} IS NOT NULL AND t.{col} <> '' ORDER BY t.{col} COLLATE NOCASE"
        );
        let mut stmt = self.conn.prepare_cached(
            &sql,
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut artists = Vec::new();
        for artist in rows {
            artists.push(artist?);
        }
        Ok(artists)
    }

    /// Return track titles grouped by album.
    pub fn library_tracks_by_album(&self, album: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT COALESCE(t.title, '')
             FROM tracks t
             JOIN locations l ON l.content_id = t.content_id
             WHERE l.available = 1
               AND t.album = ?1
             ORDER BY COALESCE(t.title, '') COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![album], |row| row.get::<_, String>(0))?;
        let mut titles = Vec::new();
        for title in rows {
            titles.push(title?);
        }
        Ok(titles)
    }

    /// Return playable location paths grouped by a unique album identifier.
    pub fn library_tracks_by_album_id(&self, album_id: &str) -> anyhow::Result<Vec<String>> {
        let (album, album_owner) = Self::decode_album_identifier(album_id);
        if album_owner.is_empty() {
            let mut stmt = self.conn.prepare_cached(
                "SELECT l.path
                 FROM tracks t
                 JOIN locations l ON l.content_id = t.content_id
                 WHERE l.available = 1
                   AND t.album = ?1",
            )?;
            let rows = stmt.query_map(params![album], |row| row.get::<_, String>(0))?;
            let mut paths = Vec::new();
            for path in rows {
                paths.push(path?);
            }
            Self::sort_location_paths(&mut paths);
            return Ok(paths);
        }

        let mut stmt = self.conn.prepare_cached(
            "SELECT l.path
             FROM tracks t
             JOIN locations l ON l.content_id = t.content_id
             WHERE l.available = 1
               AND t.album = ?1
               AND COALESCE(NULLIF(t.album_artist, ''), NULLIF(t.artist, ''), '') = ?2",
        )?;
        let rows = stmt.query_map(params![album, album_owner], |row| row.get::<_, String>(0))?;
        let mut paths = Vec::new();
        for path in rows {
            paths.push(path?);
        }

        Self::sort_location_paths(&mut paths);

        Ok(paths)
    }

    /// Return track titles grouped by artist.
    pub fn library_tracks_by_artist(&self, artist: &str) -> anyhow::Result<Vec<String>> {
        self.library_track_titles_by_artist(artist)
    }

    /// Return location paths grouped by selected artist-like kind, sorted by
    /// album name then file name.
    pub fn library_track_locations_by_artist_kind(
        &self,
        kind: &str,
        artist_name: &str,
    ) -> anyhow::Result<Vec<String>> {
        let col = match kind {
            "artist" => "artist",
            "album_artist" => "album_artist",
            "composer" => "composer",
            _ => "artist",
        };
        let sql = format!(
            "SELECT COALESCE(t.album, ''), l.path
             FROM tracks t
             JOIN locations l ON l.content_id = t.content_id
             WHERE l.available = 1
               AND t.{col} = ?1"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![artist_name], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut items: Vec<(String, String)> = Vec::new();
        for row in rows {
            items.push(row?);
        }

        items.sort_by(|(album_a, path_a), (album_b, path_b)| {
            let a_key = album_a.to_ascii_lowercase();
            let b_key = album_b.to_ascii_lowercase();
            let ord = a_key.cmp(&b_key);
            if ord != std::cmp::Ordering::Equal {
                return ord;
            }
            let file_a = Path::new(path_a)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path_a)
                .to_ascii_lowercase();
            let file_b = Path::new(path_b)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path_b)
                .to_ascii_lowercase();
            file_a.cmp(&file_b)
        });

        Ok(items.into_iter().map(|(_, path)| path).collect())
    }

    /// Return track titles grouped by artist.
    pub fn library_track_titles_by_artist(&self, artist: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT COALESCE(t.title, '')
             FROM tracks t
             JOIN locations l ON l.content_id = t.content_id
             WHERE l.available = 1
               AND t.artist = ?1
             ORDER BY COALESCE(t.title, '') COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![artist], |row| row.get(0))?;
        let mut titles = Vec::new();
        for title in rows {
            titles.push(title?);
        }
        Ok(titles)
    }

    /// Return title/artist/album by an exact location path.
    pub fn track_metadata_by_location(
        &self,
        location_path: &str,
    ) -> anyhow::Result<Option<(String, String, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT COALESCE(t.title, ''), COALESCE(t.artist, ''), COALESCE(t.album, '')
             FROM locations l
             JOIN tracks t ON t.content_id = l.content_id
             WHERE l.path = ?1
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![location_path])?;
        if let Some(row) = rows.next()? {
            Ok(Some((row.get(0)?, row.get(1)?, row.get(2)?)))
        } else {
            Ok(None)
        }
    }

    /// Return one representative location path for the given album.
    pub fn album_representation_path(&self, album: &str) -> anyhow::Result<Option<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT l.path
             FROM locations l
             JOIN tracks t ON t.content_id = l.content_id
             WHERE t.album = ?1
             ORDER BY l.last_seen DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![album])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Return candidate location paths for an album ordered by availability/recency.
    pub fn album_location_paths(&self, album: &str, limit: usize) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT l.path
             FROM locations l
             JOIN tracks t ON t.content_id = l.content_id
             WHERE t.album = ?1
             ORDER BY l.available DESC, l.preferred DESC, l.last_seen DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![album, limit as i64], |row| row.get(0))?;
        let mut paths = Vec::new();
        for path in rows {
            paths.push(path?);
        }
        Ok(paths)
    }

    /// Clear all cached library metadata and locations from the meta cache.
    pub fn clear_library(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            "BEGIN TRANSACTION;
             DELETE FROM locations;
             DELETE FROM hashes;
             DELETE FROM lyrics;
             DELETE FROM files;
             DELETE FROM tracks;
             DELETE FROM locations_staging;
             DELETE FROM hashes_staging;
             DELETE FROM files_staging;
             DELETE FROM tracks_staging;
             COMMIT;",
        )?;
        Ok(())
    }
 
     /// Return a hierarchical tree of scanned library directories.
    pub fn library_directories(&self, limit: usize) -> anyhow::Result<Vec<LibraryDirectoryDto>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT DISTINCT path FROM locations WHERE available = 1")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut dirs = HashSet::new();

        for path in rows {
            let path: String = path?;
            let mut current = PathBuf::from(&path);
            while let Some(parent) = current.parent() {
                if parent.as_os_str().is_empty() {
                    break;
                }
                dirs.insert(parent.to_path_buf());
                current = parent.to_path_buf();
            }
        }

        let mut children: std::collections::HashMap<PathBuf, Vec<PathBuf>> = std::collections::HashMap::new();
        let mut roots = Vec::new();

        for dir in dirs.iter() {
            if let Some(parent) = dir.parent() {
                if dirs.contains(parent) {
                    children.entry(parent.to_path_buf()).or_default().push(dir.clone());
                } else {
                    roots.push(dir.clone());
                }
            }
        }

        let mut sorted_roots: Vec<_> = roots.into_iter().collect();
        sorted_roots.sort();

        fn build_tree(
            path: PathBuf,
            children: &std::collections::HashMap<PathBuf, Vec<PathBuf>>,
            limit: usize,
        ) -> LibraryDirectoryDto {
            let mut child_nodes = children
                .get(&path)
                .cloned()
                .unwrap_or_default();
            child_nodes.sort();
            let children = child_nodes
                .into_iter()
                .take(limit)
                .map(|child| build_tree(child, children, limit))
                .collect();
            LibraryDirectoryDto {
                path: path.to_string_lossy().to_string(),
                children,
            }
        }

        let mut result = Vec::new();
        for root in sorted_roots.into_iter().take(limit) {
            result.push(build_tree(root, &children, limit));
        }

        Ok(result)
    }

    /// Return playable location paths under the given directory path.
    pub fn library_tracks_by_directory(&self, directory: &str) -> anyhow::Result<Vec<String>> {
        let mut pattern = directory.to_string();
        if !pattern.ends_with(MAIN_SEPARATOR) {
            pattern.push(MAIN_SEPARATOR);
        }
        pattern.push('%');

        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT l.path FROM tracks t JOIN locations l ON l.content_id = t.content_id WHERE l.available = 1 AND (l.path = ?1 OR l.path LIKE ?2)",
        )?;
        let rows = stmt.query_map(params![directory, pattern], |row| row.get::<_, String>(0))?;
        let mut paths = Vec::new();
        for path in rows {
            paths.push(path?);
        }

        paths.sort_by(|a, b| {
            let a_name = Path::new(a)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(a)
                .to_ascii_lowercase();
            let b_name = Path::new(b)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(b)
                .to_ascii_lowercase();
            a_name.cmp(&b_name)
        });

        Ok(paths)
    }

    /// Batch version of [`MetaCache::track_metadata_by_location`]: resolve
    /// display metadata for many location paths with one query per 500 paths
    /// (instead of one query per row). Paths that are not in the library are
    /// simply absent from the returned map.
    ///
    /// Intended use: call once when a track list changes (not per frame) and
    /// keep the map next to the list.
    pub fn track_metadata_many<S: AsRef<str>>(
        &self,
        location_paths: &[S],
    ) -> anyhow::Result<HashMap<String, TrackMetadataRow>> {
        let mut result = HashMap::with_capacity(location_paths.len());
        for chunk in location_paths.chunks(BATCH_LOOKUP_CHUNK) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT l.path, COALESCE(t.title, ''), COALESCE(t.artist, ''), COALESCE(t.album, ''),
                        COALESCE(t.album_artist, ''), COALESCE(t.composer, '')
                 FROM locations l
                 JOIN tracks t ON t.content_id = l.content_id
                 WHERE l.path IN ({placeholders})"
            );
            // Full chunks share one cached statement; only the final partial
            // chunk prepares a statement of a new size.
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(
                rusqlite::params_from_iter(chunk.iter().map(|p| p.as_ref())),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        TrackMetadataRow {
                            title: row.get(1)?,
                            artist: row.get(2)?,
                            album: row.get(3)?,
                            album_artist: row.get(4)?,
                            composer: row.get(5)?,
                        },
                    ))
                },
            )?;
            for row in rows {
                let (path, meta) = row?;
                // A path is UNIQUE in `locations`; keep the first row anyway.
                result.entry(path).or_insert(meta);
            }
        }
        Ok(result)
    }

    /// Expose a raw connection reference for advanced ops (use carefully)
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

/// A cloneable, thread-safe handle to one long-lived [`MetaCache`] connection.
///
/// `rusqlite::Connection` is `Send` but not `Sync`, so the connection lives
/// behind a mutex. Obtain the process-wide default handle with
/// [`MetaCache::shared`], or wrap a specific database with
/// [`SharedMetaCache::open`] / [`SharedMetaCache::new`].
///
/// Do not call `with`/`try_with` re-entrantly from inside the closure (the
/// mutex is not re-entrant; `with` would deadlock, `try_with` returns `None`).
#[derive(Clone)]
pub struct SharedMetaCache {
    inner: Arc<Mutex<MetaCache>>,
    path: Arc<PathBuf>,
}

impl SharedMetaCache {
    /// Wrap an already-open cache. Its busy timeout is left unchanged.
    pub fn new(cache: MetaCache) -> Self {
        let path = Arc::new(cache.path().to_path_buf());
        Self {
            inner: Arc::new(Mutex::new(cache)),
            path,
        }
    }

    /// Open (or create) the database at `path` with [`UI_BUSY_TIMEOUT`].
    pub fn open<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let cache = MetaCache::new(path)?;
        cache.set_busy_timeout(UI_BUSY_TIMEOUT)?;
        Ok(Self::new(cache))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run `f` with the connection, waiting for other users of this handle.
    pub fn with<R>(&self, f: impl FnOnce(&MetaCache) -> R) -> R {
        let guard = self.lock();
        f(&guard)
    }

    /// Run `f` only if the connection is free right now; never blocks.
    /// Returns `None` when another thread is currently using the handle.
    pub fn try_with<R>(&self, f: impl FnOnce(&MetaCache) -> R) -> Option<R> {
        match self.inner.try_lock() {
            Ok(guard) => Some(f(&guard)),
            Err(TryLockError::Poisoned(poisoned)) => Some(f(&poisoned.into_inner())),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    /// Lock the connection for several calls in a row.
    pub fn lock(&self) -> MutexGuard<'_, MetaCache> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// True when both handles share the same underlying connection.
    pub fn ptr_eq(&self, other: &SharedMetaCache) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl std::fmt::Debug for SharedMetaCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedMetaCache").field("path", &self.path).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(cache: &MetaCache, n: usize) {
        let conn = cache.connection();
        conn.execute_batch("BEGIN").unwrap();
        for i in 0..n {
            let cid = format!("c{i}");
            conn.execute(
                "INSERT INTO tracks (content_id, title, artist, album, album_artist, composer) VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                params![cid, format!("Title {i}"), format!("Artist {}", i % 7), format!("Album {}", i % 13), "AA"],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO locations (content_id, path, last_seen, available) VALUES (?1, ?2, ?3, 1)",
                params![cid, format!("/music/d{}/t{i}.flac", i % 10), i as i64],
            )
            .unwrap();
        }
        conn.execute_batch("COMMIT").unwrap();
    }

    #[test]
    fn track_metadata_many_matches_single_lookups_across_chunks() {
        let cache = MetaCache::new(":memory:").unwrap();
        seed(&cache, 1_234);
        let mut paths: Vec<String> = (0..1_234).map(|i| format!("/music/d{}/t{i}.flac", i % 10)).collect();
        paths.push("/not/in/library.mp3".to_string());

        let map = cache.track_metadata_many(&paths).unwrap();
        assert_eq!(map.len(), 1_234);
        assert!(!map.contains_key("/not/in/library.mp3"));
        for path in paths.iter().take(1_234).step_by(97) {
            let (title, artist, album) = cache.track_metadata_by_location(path).unwrap().unwrap();
            let row = &map[path];
            assert_eq!((row.title.as_str(), row.artist.as_str(), row.album.as_str()), (title.as_str(), artist.as_str(), album.as_str()));
            assert_eq!(row.album_artist, "AA");
            assert_eq!(row.composer, "");
        }
        let empty: [&str; 0] = [];
        assert!(cache.track_metadata_many(&empty).unwrap().is_empty());
    }

    #[test]
    fn reopening_skips_schema_ddl_and_keeps_data() {
        let dir = std::env::temp_dir().join(format!("lrc_meta_cache_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("reopen.db");
        let _ = std::fs::remove_file(&db);
        {
            let cache = MetaCache::new(&db).unwrap();
            seed(&cache, 3);
            let v: i64 = cache.connection().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
            assert_eq!(v, SCHEMA_VERSION);
        }
        let start = std::time::Instant::now();
        let cache = MetaCache::new(&db).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(cache.library_counts().unwrap().1, 3);
        // Index from schema v2 exists.
        let idx: i64 = cache
            .connection()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='idx_locations_available_last_seen'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1);
        assert!(elapsed < Duration::from_secs(1), "reopen took {elapsed:?}");
        drop(cache);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn data_version_changes_only_for_other_connections() {
        let dir = std::env::temp_dir().join(format!("lrc_meta_cache_dv_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("dv.db");
        let _ = std::fs::remove_file(&db);
        let reader = MetaCache::new(&db).unwrap();
        let writer = MetaCache::new(&db).unwrap();
        let v0 = reader.data_version().unwrap();
        seed(&reader, 1); // own write
        assert_eq!(reader.data_version().unwrap(), v0);
        writer.connection().execute("UPDATE locations SET last_seen = 99", []).unwrap();
        assert_ne!(reader.data_version().unwrap(), v0);
        drop((reader, writer));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shared_handle_is_reused_and_try_with_never_blocks() {
        let a = MetaCache::shared().unwrap();
        let b = MetaCache::shared().unwrap();
        assert!(a.ptr_eq(&b));

        let mem = SharedMetaCache::new(MetaCache::new(":memory:").unwrap());
        let guard = mem.lock();
        let other = mem.clone();
        let blocked = std::thread::spawn(move || other.try_with(|_| ()).is_none())
            .join()
            .unwrap();
        assert!(blocked, "try_with must return None while locked");
        drop(guard);
        assert_eq!(mem.try_with(|c| c.library_counts().unwrap()), Some((0, 0)));
    }
}


