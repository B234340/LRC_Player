use lrc_player_shared::dto::LibraryDirectoryDto;
use anyhow::Context;
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
            .prepare("SELECT content_id FROM locations WHERE path = ?1 LIMIT 1")?;
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
        let mut stmt = self.conn.prepare(
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
            let mut stmt = self.conn.prepare(
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

    fn encode_album_identifier(album: &str, album_artist: &str) -> String {
        let mut id = String::with_capacity(album.len() + 1 + album_artist.len());
        id.push_str(album);
        id.push('\0');
        id.push_str(album_artist);
        id
    }

    fn decode_album_identifier(album_id: &str) -> (&str, &str) {
        album_id.split_once('\0').unwrap_or((album_id, ""))
    }

    /// Return distinct album names in the library.
    pub fn library_albums(&self) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT album
             FROM tracks
             WHERE album IS NOT NULL AND album <> ''
             ORDER BY album COLLATE NOCASE",
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
            "SELECT DISTINCT {col} FROM tracks WHERE {col} IS NOT NULL AND {col} <> '' ORDER BY {col} COLLATE NOCASE"
        );
        let mut stmt = self.conn.prepare(
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
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(title, '') FROM tracks WHERE album = ?1 ORDER BY title COLLATE NOCASE",
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
            return self.library_tracks_by_album(album);
        }

        let mut stmt = self.conn.prepare(
            "SELECT l.path
             FROM tracks t
             JOIN locations l ON l.content_id = t.content_id
             WHERE t.album = ?1
               AND COALESCE(NULLIF(t.album_artist, ''), NULLIF(t.artist, ''), '') = ?2",
        )?;
        let rows = stmt.query_map(params![album, album_owner], |row| row.get::<_, String>(0))?;
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
             WHERE t.{col} = ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
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

    /// Return title/artist/album by an exact location path.
    pub fn track_metadata_by_location(
        &self,
        location_path: &str,
    ) -> anyhow::Result<Option<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
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
        let mut stmt = self.conn.prepare(
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
        let mut stmt = self.conn.prepare(
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
             DELETE FROM hashes;
             DELETE FROM files;
             DELETE FROM tracks;
             DELETE FROM locations;
             DELETE FROM hashes_staging;
             DELETE FROM files_staging;
             DELETE FROM tracks_staging;
             DELETE FROM locations_staging;
             COMMIT;",
        )?;
        Ok(())
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

    /// Return playable location paths under the given directory path.
    pub fn library_tracks_by_directory(&self, directory: &str) -> anyhow::Result<Vec<String>> {
        let mut pattern = directory.to_string();
        if !pattern.ends_with(MAIN_SEPARATOR) {
            pattern.push(MAIN_SEPARATOR);
        }
        pattern.push('%');

        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT l.path FROM tracks t JOIN locations l ON l.content_id = t.content_id WHERE l.path = ?1 OR l.path LIKE ?2",
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

    /// Expose a raw connection reference for advanced ops (use carefully)
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}


