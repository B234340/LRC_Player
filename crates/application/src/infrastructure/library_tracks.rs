//! Whole-library track listing for the GUI's library table, plus a small persistent cache of
//! probed per-file details (duration, year, genre, track number) that the library database
//! does not store.
//!
//! Both are meant for a worker thread: [`load_library_tracks`] runs one query over all
//! available locations, and [`TrackDetailsCache`] is a JSON file next to the database keyed
//! by path and validated by size + mtime so files are only probed again when they change.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::infrastructure::persistence::MetaCache;

/// One playable location with its tags (missing tags are empty strings).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryTrackRow {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub composer: String,
}

/// Every available track location in the library, ordered by path.
pub fn load_library_tracks(cache: &MetaCache) -> Result<Vec<LibraryTrackRow>> {
    let mut stmt = cache.connection().prepare_cached(
        "SELECT l.path, COALESCE(t.title, ''), COALESCE(t.artist, ''), COALESCE(t.album, ''),
                COALESCE(t.album_artist, ''), COALESCE(t.composer, '')
         FROM locations l
         JOIN tracks t ON t.content_id = l.content_id
         WHERE l.available = 1
         ORDER BY l.path",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(LibraryTrackRow {
            path: row.get(0)?,
            title: row.get(1)?,
            artist: row.get(2)?,
            album: row.get(3)?,
            album_artist: row.get(4)?,
            composer: row.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Probed details of one audio file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackDetails {
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub track_no: Option<u32>,
    #[serde(default)]
    pub disc_no: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    size: u64,
    mtime: i64,
    #[serde(flatten)]
    details: TrackDetails,
}

/// Size + mtime (seconds) used to validate cache entries.
pub fn file_stamp(path: &Path) -> Option<(u64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some((meta.len(), mtime))
}

/// Persistent path -> [`TrackDetails`] cache.
#[derive(Debug, Default)]
pub struct TrackDetailsCache {
    path: Option<PathBuf>,
    entries: HashMap<String, Entry>,
    dirty: bool,
}

impl TrackDetailsCache {
    /// `track_details.json` next to the library database at `db_path`.
    pub fn path_for_db(db_path: &Path) -> PathBuf {
        db_path
            .parent()
            .map(|p| p.join("track_details.json"))
            .unwrap_or_else(|| PathBuf::from("track_details.json"))
    }

    /// Loads the cache (an unreadable or missing file gives an empty cache).
    pub fn load(path: PathBuf) -> Self {
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path: Some(path), entries, dirty: false }
    }

    /// In-memory cache that is never written.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Cached details when the file still has the given size + mtime.
    pub fn get(&self, path: &str, stamp: (u64, i64)) -> Option<&TrackDetails> {
        self.entries
            .get(path)
            .filter(|e| (e.size, e.mtime) == stamp)
            .map(|e| &e.details)
    }

    pub fn insert(&mut self, path: String, stamp: (u64, i64), details: TrackDetails) {
        self.entries.insert(path, Entry { size: stamp.0, mtime: stamp.1, details });
        self.dirty = true;
    }

    /// Drops entries whose path is not in `keep` (files removed from the library).
    pub fn retain_paths(&mut self, keep: &std::collections::HashSet<&str>) {
        let before = self.entries.len();
        self.entries.retain(|k, _| keep.contains(k.as_str()));
        self.dirty |= self.entries.len() != before;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Writes the file (atomic replace) when something changed.
    pub fn save(&mut self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec(&self.entries)?;
        std::fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        self.dirty = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn lists_available_tracks_with_tags() {
        let cache = MetaCache::new(":memory:").unwrap();
        for (i, (title, avail)) in [("B", 1), ("A", 1), ("Gone", 0)].iter().enumerate() {
            let cid = format!("c{i}");
            cache
                .connection()
                .execute(
                    "INSERT INTO tracks (content_id, title, artist, album, album_artist) VALUES (?1, ?2, 'Ar', 'Al', NULL)",
                    params![cid, title],
                )
                .unwrap();
            cache
                .connection()
                .execute(
                    "INSERT INTO locations (content_id, path, last_seen, available) VALUES (?1, ?2, 0, ?3)",
                    params![cid, format!("/m/{i}.flac"), avail],
                )
                .unwrap();
        }
        let rows = load_library_tracks(&cache).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, "/m/0.flac");
        assert_eq!(rows[0].title, "B");
        assert_eq!(rows[1].artist, "Ar");
        assert_eq!(rows[1].album_artist, "");
    }

    #[test]
    fn details_cache_roundtrip_and_stamp_validation() {
        let dir = tempfile_dir();
        let path = dir.join("track_details.json");
        let mut cache = TrackDetailsCache::load(path.clone());
        assert!(cache.is_empty());
        let details = TrackDetails { duration_ms: Some(1234), year: Some(2020), genre: Some("Pop".into()), track_no: Some(3), disc_no: None };
        cache.insert("/a.flac".into(), (10, 20), details.clone());
        cache.save().unwrap();
        let loaded = TrackDetailsCache::load(path);
        assert_eq!(loaded.get("/a.flac", (10, 20)), Some(&details));
        assert_eq!(loaded.get("/a.flac", (11, 20)), None, "changed file is re-probed");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lrc_details_{}_{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
