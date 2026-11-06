//! Cached library view data for the GUI.
//!
//! The GUI's `update_app_vm` currently rebuilds a `LibraryDto` on every tab
//! click / selection / queue edit by: `Config::load()` (file read + parse),
//! `MetaCache::open_default()` (open SQLite + PRAGMAs + schema DDL), and ~8-10
//! full-scan queries (`library_counts` x2, `recent_locations`,
//! `recent_directories`, `library_directories`, `library_albums`,
//! `library_artists_by_kind`, plus the selected album/artist/directory track
//! lists), then fsyncs state.json.
//!
//! [`LibrarySnapshotCache`] keeps the result of those queries and reloads only
//! when the library actually changed:
//!
//! * another connection (the scanner) committed — detected with the cheap
//!   `PRAGMA data_version` on the same connection, or
//! * the caller called [`LibrarySnapshotCache::invalidate`] (do this after
//!   writes made through the same connection, e.g. `clear_library`, and when a
//!   scan completes), or
//! * the artist view mode changed.
//!
//! Selected album / artist / directory track lists are cached per key too.
//! A call with nothing changed costs one `PRAGMA data_version` plus cloning
//! the vectors into the `LibraryDto`.
//!
//! Always pass the **same** connection (e.g. the handle from
//! `MetaCache::shared()`); `data_version` values are only comparable on one
//! connection. Passing a different connection simply forces a reload.

use crate::dto::{LibraryDirectoryDto, LibraryDto};
use crate::infrastructure::persistence::MetaCache;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;

/// Same limits the GUI's `library_dto()` uses today.
pub const RECENT_LOCATIONS_LIMIT: usize = 20;
pub const RECENT_DIRECTORIES_LIMIT: usize = 20;
pub const DIRECTORY_TREE_LIMIT: usize = 20;

/// Bound on cached per-selection track lists before they are dropped.
const MAX_CACHED_SELECTIONS: usize = 64;

/// Library-wide data that does not depend on the current selection.
#[derive(Debug, Clone, Default)]
pub struct LibrarySnapshot {
    pub album_count: usize,
    pub track_count: usize,
    pub recent_locations: Vec<String>,
    pub recent_directories: Vec<String>,
    pub directory_tree: Vec<LibraryDirectoryDto>,
    pub albums: Vec<String>,
    /// `ArtistViewMode::key()` the `artists` list was built for.
    pub artist_kind: String,
    pub artists: Vec<String>,
}

impl LibrarySnapshot {
    /// Run the library-wide queries once.
    pub fn load(cache: &MetaCache, artist_kind: &str) -> Result<Self> {
        let (album_count, track_count) = cache.library_counts()?;
        Ok(Self {
            album_count,
            track_count,
            recent_locations: cache.recent_locations(RECENT_LOCATIONS_LIMIT)?,
            recent_directories: cache.recent_directories(RECENT_DIRECTORIES_LIMIT)?,
            directory_tree: cache.library_directories(DIRECTORY_TREE_LIMIT)?,
            albums: cache.library_albums()?,
            artist_kind: artist_kind.to_string(),
            artists: cache.library_artists_by_kind(artist_kind)?,
        })
    }
}

/// Selection-dependent inputs of a `LibraryDto`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibrarySelection {
    pub favorite_directories: Vec<String>,
    /// Album identifier as used by `MetaCache::library_tracks_by_album_id`.
    pub selected_album: Option<String>,
    pub selected_artist: Option<String>,
    pub selected_directory: Option<String>,
}

#[derive(Debug, Default)]
pub struct LibrarySnapshotCache {
    snapshot: Option<Arc<LibrarySnapshot>>,
    data_version: Option<i64>,
    connection_id: Option<usize>,
    dirty: bool,
    reloads: u64,
    album_tracks: HashMap<String, Arc<Vec<String>>>,
    artist_tracks: HashMap<(String, String), Arc<Vec<String>>>,
    directory_tracks: HashMap<String, Arc<Vec<String>>>,
}

impl LibrarySnapshotCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Force a reload on next access. Call after a scan completes and after
    /// writes made through the same connection (they don't bump data_version).
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// How many times the library-wide queries actually ran (diagnostics/tests).
    pub fn reload_count(&self) -> u64 {
        self.reloads
    }

    /// Drop cached data if the database changed since it was loaded.
    fn check_validity(&mut self, cache: &MetaCache) {
        let connection_id = cache as *const MetaCache as usize;
        let version = cache.data_version().ok();
        let changed = self.dirty
            || version.is_none()
            || version != self.data_version
            || Some(connection_id) != self.connection_id;
        if changed {
            self.snapshot = None;
            self.album_tracks.clear();
            self.artist_tracks.clear();
            self.directory_tracks.clear();
            self.data_version = version;
            self.connection_id = Some(connection_id);
            self.dirty = false;
        }
    }

    /// Library-wide data, reloaded only when stale.
    pub fn snapshot(&mut self, cache: &MetaCache, artist_kind: &str) -> Result<Arc<LibrarySnapshot>> {
        self.check_validity(cache);
        if let Some(snapshot) = &self.snapshot {
            if snapshot.artist_kind == artist_kind {
                return Ok(Arc::clone(snapshot));
            }
            // Only the artist list depends on the view mode.
            let mut updated = LibrarySnapshot::clone(snapshot);
            updated.artists = cache.library_artists_by_kind(artist_kind)?;
            updated.artist_kind = artist_kind.to_string();
            let updated = Arc::new(updated);
            self.snapshot = Some(Arc::clone(&updated));
            return Ok(updated);
        }
        let snapshot = Arc::new(LibrarySnapshot::load(cache, artist_kind)?);
        self.reloads += 1;
        self.snapshot = Some(Arc::clone(&snapshot));
        Ok(snapshot)
    }

    pub fn album_tracks(&mut self, cache: &MetaCache, album_id: &str) -> Result<Arc<Vec<String>>> {
        self.check_validity(cache);
        cached(&mut self.album_tracks, album_id.to_string(), || {
            cache.library_tracks_by_album_id(album_id)
        })
    }

    pub fn artist_tracks(
        &mut self,
        cache: &MetaCache,
        artist_kind: &str,
        artist: &str,
    ) -> Result<Arc<Vec<String>>> {
        self.check_validity(cache);
        cached(
            &mut self.artist_tracks,
            (artist_kind.to_string(), artist.to_string()),
            || cache.library_track_locations_by_artist_kind(artist_kind, artist),
        )
    }

    pub fn directory_tracks(&mut self, cache: &MetaCache, directory: &str) -> Result<Arc<Vec<String>>> {
        self.check_validity(cache);
        cached(&mut self.directory_tracks, directory.to_string(), || {
            cache.library_tracks_by_directory(directory)
        })
    }

    /// Drop-in replacement for the GUI's `library_dto()` body: same queries,
    /// same limits, same result, but served from cache when nothing changed.
    pub fn library_dto(
        &mut self,
        cache: &MetaCache,
        artist_kind: &str,
        selection: &LibrarySelection,
    ) -> Result<LibraryDto> {
        let snapshot = self.snapshot(cache, artist_kind)?;
        let selected_album_tracks = match selection.selected_album.as_deref() {
            Some(album) => self.album_tracks(cache, album)?.as_ref().clone(),
            None => Vec::new(),
        };
        let selected_artist_tracks = match selection.selected_artist.as_deref() {
            Some(artist) => self.artist_tracks(cache, artist_kind, artist)?.as_ref().clone(),
            None => Vec::new(),
        };
        let selected_directory_tracks = match selection.selected_directory.as_deref() {
            Some(directory) => self.directory_tracks(cache, directory)?.as_ref().clone(),
            None => Vec::new(),
        };
        Ok(LibraryDto {
            album_count: snapshot.album_count,
            track_count: snapshot.track_count,
            recent_locations: snapshot.recent_locations.clone(),
            recent_directories: snapshot.recent_directories.clone(),
            directory_tree: snapshot.directory_tree.clone(),
            favorite_directories: selection.favorite_directories.clone(),
            albums: snapshot.albums.clone(),
            selected_album: selection.selected_album.clone(),
            selected_album_tracks,
            artists: snapshot.artists.clone(),
            selected_artist: selection.selected_artist.clone(),
            selected_artist_tracks,
            selected_directory: selection.selected_directory.clone(),
            selected_directory_tracks,
        })
    }
}

fn cached<K: std::hash::Hash + Eq>(
    map: &mut HashMap<K, Arc<Vec<String>>>,
    key: K,
    load: impl FnOnce() -> Result<Vec<String>>,
) -> Result<Arc<Vec<String>>> {
    if let Some(hit) = map.get(&key) {
        return Ok(Arc::clone(hit));
    }
    if map.len() >= MAX_CACHED_SELECTIONS {
        map.clear();
    }
    let value = Arc::new(load()?);
    map.insert(key, Arc::clone(&value));
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn add_track(cache: &MetaCache, i: usize, album: &str, artist: &str, dir: &str) {
        let cid = format!("cid{i}");
        cache
            .connection()
            .execute(
                "INSERT INTO tracks (content_id, title, artist, album, album_artist) VALUES (?1, ?2, ?3, ?4, ?3)",
                params![cid, format!("T{i}"), artist, album],
            )
            .unwrap();
        cache
            .connection()
            .execute(
                "INSERT INTO locations (content_id, path, last_seen, available) VALUES (?1, ?2, ?3, 1)",
                params![cid, format!("{dir}/t{i}.flac"), i as i64],
            )
            .unwrap();
    }

    fn uncached_dto(cache: &MetaCache, kind: &str, sel: &LibrarySelection) -> LibraryDto {
        let (album_count, track_count) = cache.library_counts().unwrap();
        LibraryDto {
            album_count,
            track_count,
            recent_locations: cache.recent_locations(20).unwrap(),
            recent_directories: cache.recent_directories(20).unwrap(),
            directory_tree: cache.library_directories(20).unwrap(),
            favorite_directories: sel.favorite_directories.clone(),
            albums: cache.library_albums().unwrap(),
            selected_album: sel.selected_album.clone(),
            selected_album_tracks: sel
                .selected_album
                .as_deref()
                .map(|a| cache.library_tracks_by_album_id(a).unwrap())
                .unwrap_or_default(),
            artists: cache.library_artists_by_kind(kind).unwrap(),
            selected_artist: sel.selected_artist.clone(),
            selected_artist_tracks: sel
                .selected_artist
                .as_deref()
                .map(|a| cache.library_track_locations_by_artist_kind(kind, a).unwrap())
                .unwrap_or_default(),
            selected_directory: sel.selected_directory.clone(),
            selected_directory_tracks: sel
                .selected_directory
                .as_deref()
                .map(|d| cache.library_tracks_by_directory(d).unwrap())
                .unwrap_or_default(),
        }
    }

    fn same(a: &LibraryDto, b: &LibraryDto) -> bool {
        serde_json::to_string(a).unwrap() == serde_json::to_string(b).unwrap()
    }

    #[test]
    fn cached_dto_matches_uncached_and_only_reloads_when_needed() {
        let cache = MetaCache::new(":memory:").unwrap();
        for i in 0..30 {
            add_track(&cache, i, &format!("Album {}", i % 4), &format!("Artist {}", i % 3), &format!("/m/d{}", i % 5));
        }
        let sel = LibrarySelection {
            favorite_directories: vec!["/m".into()],
            selected_album: Some("Album 1".into()),
            selected_artist: Some("Artist 2".into()),
            selected_directory: Some("/m/d3".into()),
        };
        let mut snap = LibrarySnapshotCache::new();
        let dto = snap.library_dto(&cache, "artist", &sel).unwrap();
        assert!(same(&dto, &uncached_dto(&cache, "artist", &sel)));
        assert_eq!(dto.track_count, 30);
        assert!(!dto.selected_album_tracks.is_empty());

        // Repeated calls (tab clicks) don't re-run the library queries.
        for _ in 0..10 {
            snap.library_dto(&cache, "artist", &sel).unwrap();
        }
        assert_eq!(snap.reload_count(), 1);

        // A write through the same connection is invisible until invalidate().
        add_track(&cache, 100, "New Album", "New Artist", "/m/new");
        assert_eq!(snap.library_dto(&cache, "artist", &sel).unwrap().track_count, 30);
        snap.invalidate();
        let dto = snap.library_dto(&cache, "artist", &sel).unwrap();
        assert_eq!(dto.track_count, 31);
        assert!(same(&dto, &uncached_dto(&cache, "artist", &sel)));
        assert_eq!(snap.reload_count(), 2);

        // Switching artist view mode only refreshes the artist list.
        let dto = snap.library_dto(&cache, "album_artist", &sel).unwrap();
        assert!(same(&dto, &uncached_dto(&cache, "album_artist", &sel)));
        assert_eq!(snap.reload_count(), 2);
    }

    #[test]
    fn commits_from_another_connection_trigger_reload() {
        let dir = std::env::temp_dir().join(format!("lrc_snapshot_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("lib.db");
        let _ = std::fs::remove_file(&db);
        let ui = MetaCache::new(&db).unwrap();
        let scanner = MetaCache::new(&db).unwrap();
        add_track(&scanner, 1, "A", "X", "/m");

        let mut snap = LibrarySnapshotCache::new();
        let sel = LibrarySelection::default();
        assert_eq!(snap.library_dto(&ui, "artist", &sel).unwrap().track_count, 1);
        assert_eq!(snap.library_dto(&ui, "artist", &sel).unwrap().track_count, 1);
        assert_eq!(snap.reload_count(), 1);

        add_track(&scanner, 2, "B", "Y", "/m");
        assert_eq!(snap.library_dto(&ui, "artist", &sel).unwrap().track_count, 2);
        assert_eq!(snap.reload_count(), 2);
        drop((ui, scanner));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
