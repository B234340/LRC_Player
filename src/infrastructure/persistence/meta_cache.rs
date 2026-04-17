use crate::application::dto::LibraryDirectoryDto;
use anyhow::Context;
use directories::ProjectDirs;
use rusqlite::{params, Connection, OpenFlags};
use std::collections::HashSet;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};

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

        let cache = MetaCache {
            conn,
            path: path.to_path_buf(),
        };
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

        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn default_path() -> anyhow::Result<PathBuf> {
        let dirs = ProjectDirs::from("com", "example", "lrc_player")
            .context("unable to determine project directories")?;
        let cache_dir = dirs.data_dir().join("cache");
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("creating cache directory {}", cache_dir.display()))?;
        Ok(cache_dir.join("meta_cache.db"))
    }

    pub fn open_default() -> anyhow::Result<Self> {
        let path = Self::default_path()?;
        Self::new(path)
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
        album: Option<&str>,
        canonical_location_id: Option<i64>,
        created_at: i64,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO tracks (content_id,title,artist,album,canonical_location_id,created_at)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(content_id) DO UPDATE SET
               title=excluded.title,
               artist=excluded.artist,
               album=excluded.album,
               canonical_location_id=excluded.canonical_location_id,
               created_at=excluded.created_at
            ",
            params![
                content_id,
                title,
                artist,
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
        album: Option<&str>,
        canonical_location_id: Option<i64>,
        created_at: i64,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO tracks_staging (content_id,title,artist,album,canonical_location_id,created_at)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(content_id) DO UPDATE SET
               title=excluded.title,
               artist=excluded.artist,
               album=excluded.album,
               canonical_location_id=excluded.canonical_location_id,
               created_at=excluded.created_at
            ",
            params![
                content_id,
                title,
                artist,
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

    INSERT OR REPLACE INTO tracks (content_id,title,artist,album,canonical_location_id,created_at)
    SELECT content_id,title,artist,album,canonical_location_id,created_at FROM tracks_staging;
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
        let mut stmt = self.conn.prepare("SELECT id FROM files WHERE path = ?1")?;
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
        let mut stmt = self.conn.prepare(
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
        let track_count: i64 = self
            .conn
            .query_row("SELECT count(*) FROM tracks", [], |r| r.get(0))?;
        let album_count: i64 = self.conn.query_row(
            "SELECT count(DISTINCT album) FROM tracks WHERE album IS NOT NULL AND album <> ''",
            [],
            |r| r.get(0),
        )?;
        Ok((album_count as usize, track_count as usize))
    }

    /// Return the most recently seen track locations from the library.
    pub fn recent_locations(&self, limit: usize) -> anyhow::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM locations ORDER BY last_seen DESC LIMIT ?1")?;
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
            .prepare("SELECT path FROM locations ORDER BY last_seen DESC")?;
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

    /// Return distinct album names in the library.
    pub fn library_albums(&self) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT album FROM tracks WHERE album IS NOT NULL AND album <> '' ORDER BY album COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut albums = Vec::new();
        for album in rows {
            albums.push(album?);
        }
        Ok(albums)
    }

    /// Return distinct artist names in the library.
    pub fn library_artists(&self) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT artist FROM tracks WHERE artist IS NOT NULL AND artist <> '' ORDER BY artist COLLATE NOCASE",
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
        let mut stmt = self.conn.prepare(
            "SELECT title FROM tracks WHERE album = ?1 ORDER BY title COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![album], |row| row.get(0))?;
        let mut titles = Vec::new();
        for title in rows {
            titles.push(title?);
        }
        Ok(titles)
    }

    /// Return track titles grouped by artist.
    pub fn library_tracks_by_artist(&self, artist: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT title FROM tracks WHERE artist = ?1 ORDER BY title COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![artist], |row| row.get(0))?;
        let mut titles = Vec::new();
        for title in rows {
            titles.push(title?);
        }
        Ok(titles)
    }

    /// Return a hierarchical tree of scanned library directories.
    pub fn library_directories(&self, limit: usize) -> anyhow::Result<Vec<LibraryDirectoryDto>> {
        let mut stmt = self.conn.prepare("SELECT DISTINCT path FROM locations")?;
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

    /// Return track titles under the given directory path.
    pub fn library_tracks_by_directory(&self, directory: &str) -> anyhow::Result<Vec<String>> {
        let mut pattern = directory.to_string();
        if !pattern.ends_with(MAIN_SEPARATOR) {
            pattern.push(MAIN_SEPARATOR);
        }
        pattern.push('%');

        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT t.title FROM tracks t JOIN locations l ON l.content_id = t.content_id WHERE l.path = ?1 OR l.path LIKE ?2 ORDER BY t.title COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![directory, pattern], |row| row.get(0))?;
        let mut titles = Vec::new();
        for title in rows {
            titles.push(title?);
        }
        Ok(titles)
    }

    /// Expose a raw connection reference for advanced ops (use carefully)
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}


