//! In-memory library index for the track table, album grid and artist pages.
//!
//! One worker thread (its own SQLite connection) loads every available track with its tags
//! ([`load_library_tracks`]), then fills in what the database does not store — duration,
//! year, genre, track/disc number — from a persistent [`TrackDetailsCache`], probing only
//! files that are new or changed (headers only, no decoding). Results arrive in batches and
//! are applied on the UI thread, which bumps [`LibraryIndex::revision`] so the track table
//! re-filters. The worker also probes paths outside the library on request (queue items,
//! dropped files) so they get titles and durations too.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use lrc_player_application::infrastructure::decode::probe_track_info;
use lrc_player_application::infrastructure::library_tracks::{
    file_stamp, load_library_tracks, LibraryTrackRow, TrackDetails, TrackDetailsCache,
};
use lrc_player_application::infrastructure::persistence::MetaCache;

use crate::presentation::ui::views::{AlbumItem, TrackRow};

/// Batch size / interval for detail updates sent to the UI.
const DETAIL_BATCH: usize = 400;
const DETAIL_INTERVAL: Duration = Duration::from_millis(700);

/// Separator between album title and owner in album keys.
const KEY_SEP: char = '\u{1f}';

pub(crate) fn album_key(album: &str, owner: &str) -> String {
    format!("{album}{KEY_SEP}{owner}")
}

/// Title shown for a path that has no tags.
pub(crate) fn file_title(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

fn row_from_db(r: LibraryTrackRow) -> (TrackRow, String) {
    let owner = if r.album_artist.trim().is_empty() { r.artist.clone() } else { r.album_artist.clone() };
    let title = if r.title.trim().is_empty() { file_title(&r.path) } else { r.title };
    (
        TrackRow {
            path: PathBuf::from(r.path),
            title,
            artist: r.artist,
            album: r.album,
            ..Default::default()
        },
        owner,
    )
}

fn apply_details(row: &mut TrackRow, d: &TrackDetails) {
    if d.duration_ms.is_some() {
        row.duration_ms = d.duration_ms;
    }
    row.year = d.year.or(row.year);
    if let Some(g) = &d.genre {
        row.genre = g.clone();
    }
    row.track_no = d.track_no.or(row.track_no);
    row.disc_no = d.disc_no.or(row.disc_no);
}

/// An artist with the rows and albums that belong to them.
#[derive(Debug, Clone, Default)]
pub(crate) struct ArtistEntry {
    pub name: String,
    pub tracks: Vec<usize>,
    pub albums: Vec<String>,
}

enum Request {
    Reload(PathBuf),
    Probe(Vec<String>),
}

enum Done {
    Rows(Vec<LibraryTrackRow>),
    Details(Vec<(String, TrackDetails)>),
    Extra(Vec<TrackRow>),
    Failed(String),
}

#[derive(Default)]
struct Slot {
    reload: Option<PathBuf>,
    probes: Vec<String>,
    shutdown: bool,
    generation: u64,
}

struct Worker {
    slot: Arc<(Mutex<Slot>, Condvar)>,
    rx: Receiver<Done>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    fn new(ctx: egui::Context, persist: bool) -> Self {
        let slot: Arc<(Mutex<Slot>, Condvar)> = Arc::default();
        let (tx, rx) = crossbeam_channel::unbounded();
        let worker_slot = Arc::clone(&slot);
        let handle = std::thread::Builder::new()
            .name("library-index".into())
            .spawn(move || worker_loop(worker_slot, tx, ctx, persist))
            .ok();
        Self { slot, rx, handle }
    }

    fn send(&self, req: Request) {
        let (lock, cvar) = &*self.slot;
        if let Ok(mut slot) = lock.lock() {
            match req {
                Request::Reload(p) => {
                    slot.reload = Some(p);
                    slot.generation += 1;
                }
                Request::Probe(paths) => slot.probes.extend(paths),
            }
            cvar.notify_one();
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.slot;
        if let Ok(mut slot) = lock.lock() {
            slot.shutdown = true;
            cvar.notify_one();
        }
        // Not joined: a slow probe must not delay exit.
        drop(self.handle.take());
    }
}

fn worker_loop(slot: Arc<(Mutex<Slot>, Condvar)>, tx: Sender<Done>, ctx: egui::Context, persist: bool) {
    let (lock, cvar) = &*slot;
    let mut details: Option<(PathBuf, TrackDetailsCache)> = None;
    loop {
        let (reload, probes, generation) = {
            let Ok(mut guard) = lock.lock() else { return };
            loop {
                if guard.shutdown {
                    if let Some((_, cache)) = details.as_mut() {
                        let _ = cache.save();
                    }
                    return;
                }
                if guard.reload.is_some() || !guard.probes.is_empty() {
                    break (guard.reload.take(), std::mem::take(&mut guard.probes), guard.generation);
                }
                guard = match cvar.wait(guard) {
                    Ok(g) => g,
                    Err(_) => return,
                };
            }
        };
        let cancelled = || lock.lock().map(|g| g.shutdown || g.generation != generation).unwrap_or(true);

        if !probes.is_empty() {
            let rows: Vec<TrackRow> = probes
                .iter()
                .map(|p| {
                    let info = probe_track_info(p).ok().unwrap_or_default();
                    TrackRow {
                        path: PathBuf::from(p),
                        title: info.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| file_title(p)),
                        artist: info.artist.clone().unwrap_or_default(),
                        album: info.album.clone().unwrap_or_default(),
                        duration_ms: info.duration_ms,
                        track_no: info.track_number,
                        disc_no: info.disc_number,
                        year: info.year,
                        genre: info.genre.clone().unwrap_or_default(),
                    }
                })
                .collect();
            if tx.send(Done::Extra(rows)).is_err() {
                return;
            }
            ctx.request_repaint();
        }

        let Some(db_path) = reload else { continue };
        let rows = match MetaCache::new(&db_path).and_then(|db| load_library_tracks(&db)) {
            Ok(rows) => rows,
            Err(err) => {
                let _ = tx.send(Done::Failed(format!("{err:#}")));
                ctx.request_repaint();
                continue;
            }
        };
        let paths: Vec<String> = rows.iter().map(|r| r.path.clone()).collect();
        if tx.send(Done::Rows(rows)).is_err() {
            return;
        }
        ctx.request_repaint();

        let cache_path = TrackDetailsCache::path_for_db(&db_path);
        if details.as_ref().is_none_or(|(p, _)| *p != cache_path) {
            let cache = if persist { TrackDetailsCache::load(cache_path.clone()) } else { TrackDetailsCache::in_memory() };
            details = Some((cache_path, cache));
        }
        let Some((_, cache)) = details.as_mut() else { continue };
        {
            let keep: HashSet<&str> = paths.iter().map(String::as_str).collect();
            cache.retain_paths(&keep);
        }

        // Cached entries first (cheap), then probe the rest.
        let mut batch = Vec::new();
        let mut missing = Vec::new();
        for path in &paths {
            if let Some(stamp) = file_stamp(Path::new(path)) {
                match cache.get(path, stamp) {
                    Some(d) => batch.push((path.clone(), d.clone())),
                    None => missing.push((path.clone(), stamp)),
                }
            }
        }
        if !batch.is_empty() && tx.send(Done::Details(std::mem::take(&mut batch))).is_err() {
            return;
        }
        ctx.request_repaint();
        let mut last_send = Instant::now();
        let mut since_save = 0usize;
        for (path, stamp) in missing {
            if cancelled() {
                break;
            }
            let info = probe_track_info(&path).ok().unwrap_or_default();
            let d = TrackDetails {
                duration_ms: info.duration_ms,
                year: info.year,
                genre: info.genre.filter(|g| !g.trim().is_empty()),
                track_no: info.track_number,
                disc_no: info.disc_number,
            };
            cache.insert(path.clone(), stamp, d.clone());
            batch.push((path, d));
            since_save += 1;
            if batch.len() >= DETAIL_BATCH || last_send.elapsed() >= DETAIL_INTERVAL {
                if tx.send(Done::Details(std::mem::take(&mut batch))).is_err() {
                    return;
                }
                ctx.request_repaint();
                last_send = Instant::now();
            }
            if since_save >= 2_000 {
                let _ = cache.save();
                since_save = 0;
            }
        }
        if !batch.is_empty() {
            let _ = tx.send(Done::Details(batch));
            ctx.request_repaint();
        }
        if let Err(err) = cache.save() {
            tracing::debug!("saving track details cache failed: {err:#}");
        }
    }
}

/// UI-side library data.
pub(crate) struct LibraryIndex {
    pub rows: Arc<Vec<TrackRow>>,
    /// Album key of each row.
    owners: Vec<String>,
    /// Bumped whenever `rows` changes.
    pub revision: u64,
    pub albums: Vec<AlbumItem>,
    /// Album key -> rows (disc / track number / path order).
    album_rows: HashMap<String, Vec<usize>>,
    pub artists: Vec<ArtistEntry>,
    artist_pos: HashMap<String, usize>,
    by_path: HashMap<PathBuf, usize>,
    /// Probed rows for paths outside the library.
    extra: HashMap<PathBuf, TrackRow>,
    requested_extra: HashSet<PathBuf>,
    pub loaded: bool,
    pub loading: bool,
    pub error: Option<String>,
    worker: Worker,
}

impl LibraryIndex {
    pub(crate) fn new(ctx: egui::Context, persist: bool) -> Self {
        Self {
            rows: Arc::new(Vec::new()),
            owners: Vec::new(),
            revision: 1,
            albums: Vec::new(),
            album_rows: HashMap::new(),
            artists: Vec::new(),
            artist_pos: HashMap::new(),
            by_path: HashMap::new(),
            extra: HashMap::new(),
            requested_extra: HashSet::new(),
            loaded: false,
            loading: false,
            error: None,
            worker: Worker::new(ctx, persist),
        }
    }

    /// (Re)loads the library from the database at `db_path` on the worker.
    pub(crate) fn reload(&mut self, db_path: &Path) {
        self.loading = true;
        self.worker.send(Request::Reload(db_path.to_path_buf()));
    }

    /// Applies finished worker results. Returns true when rows changed.
    pub(crate) fn poll(&mut self) -> bool {
        let mut changed = false;
        let mut rows_replaced = false;
        let mut details: Vec<(String, TrackDetails)> = Vec::new();
        while let Ok(done) = self.worker.rx.try_recv() {
            match done {
                Done::Rows(rows) => {
                    let (rows, owners): (Vec<TrackRow>, Vec<String>) = rows.into_iter().map(row_from_db).unzip();
                    self.rows = Arc::new(rows);
                    self.owners = owners;
                    self.loaded = true;
                    self.loading = false;
                    self.error = None;
                    details.clear();
                    rows_replaced = true;
                    changed = true;
                }
                Done::Details(batch) => details.extend(batch),
                Done::Extra(rows) => {
                    for row in rows {
                        self.extra.insert(row.path.clone(), row);
                    }
                    changed = true;
                }
                Done::Failed(err) => {
                    tracing::warn!("loading the library failed: {err}");
                    self.loading = false;
                    self.loaded = true;
                    self.error = Some(err);
                }
            }
        }
        if changed || !details.is_empty() {
            if !details.is_empty() {
                if rows_replaced {
                    self.by_path = self.rows.iter().enumerate().map(|(i, r)| (r.path.clone(), i)).collect();
                }
                let rows = Arc::make_mut(&mut self.rows);
                for (path, d) in &details {
                    if let Some(&i) = self.by_path.get(Path::new(path)) {
                        apply_details(&mut rows[i], d);
                    }
                }
            }
            self.rebuild();
            self.revision += 1;
            return true;
        }
        false
    }

    /// Recomputes lookups, albums and artists from `rows`.
    pub(crate) fn rebuild(&mut self) {
        self.by_path = self.rows.iter().enumerate().map(|(i, r)| (r.path.clone(), i)).collect();
        let mut album_rows: HashMap<String, Vec<usize>> = HashMap::new();
        let mut album_meta: HashMap<String, (String, String, Option<i32>)> = HashMap::new();
        let mut artist_map: HashMap<String, ArtistEntry> = HashMap::new();
        for (i, row) in self.rows.iter().enumerate() {
            let owner = self.owners.get(i).map(String::as_str).unwrap_or("");
            if !row.album.trim().is_empty() {
                let key = album_key(&row.album, owner);
                album_rows.entry(key.clone()).or_default().push(i);
                let meta = album_meta
                    .entry(key)
                    .or_insert_with(|| (row.album.clone(), owner.to_string(), None));
                if let Some(y) = row.year {
                    meta.2 = Some(meta.2.map_or(y, |m: i32| m.max(y)));
                }
            }
            for name in [row.artist.as_str(), owner] {
                if name.trim().is_empty() {
                    continue;
                }
                let entry = artist_map.entry(name.to_string()).or_insert_with(|| ArtistEntry { name: name.to_string(), ..Default::default() });
                if entry.tracks.last() != Some(&i) {
                    entry.tracks.push(i);
                }
                if !row.album.trim().is_empty() {
                    let key = album_key(&row.album, owner);
                    if !entry.albums.contains(&key) {
                        entry.albums.push(key);
                    }
                }
                if name == owner {
                    break;
                }
            }
        }
        for rows in album_rows.values_mut() {
            let r = &self.rows;
            rows.sort_by(|&a, &b| {
                let ka = (r[a].disc_no.unwrap_or(0), r[a].track_no.unwrap_or(u32::MAX));
                let kb = (r[b].disc_no.unwrap_or(0), r[b].track_no.unwrap_or(u32::MAX));
                ka.cmp(&kb).then_with(|| r[a].path.cmp(&r[b].path))
            });
        }
        let mut albums: Vec<AlbumItem> = album_meta
            .into_iter()
            .map(|(key, (title, artist, year))| AlbumItem {
                track_count: album_rows.get(&key).map_or(0, Vec::len),
                key,
                title,
                artist,
                year,
            })
            .collect();
        albums.sort_by(|a, b| {
            a.title
                .to_lowercase()
                .cmp(&b.title.to_lowercase())
                .then_with(|| a.artist.to_lowercase().cmp(&b.artist.to_lowercase()))
        });
        let mut artists: Vec<ArtistEntry> = artist_map.into_values().collect();
        artists.sort_by_key(|a| a.name.to_lowercase());
        self.artist_pos = artists.iter().enumerate().map(|(i, a)| (a.name.clone(), i)).collect();
        self.albums = albums;
        self.album_rows = album_rows;
        self.artists = artists;
    }

    pub(crate) fn row_index(&self, path: &Path) -> Option<usize> {
        self.by_path.get(path).copied()
    }

    /// Display row for any path: library row, probed extra row, or a file-name fallback.
    pub(crate) fn display_row(&self, path: &str) -> TrackRow {
        let p = Path::new(path);
        if let Some(&i) = self.by_path.get(p) {
            return self.rows[i].clone();
        }
        if let Some(row) = self.extra.get(p) {
            return row.clone();
        }
        TrackRow { path: PathBuf::from(path), title: file_title(path), ..Default::default() }
    }

    /// Asks the worker to probe paths that are neither in the library nor probed yet.
    pub(crate) fn ensure_probed<'a>(&mut self, paths: impl IntoIterator<Item = &'a String>) {
        let missing: Vec<String> = paths
            .into_iter()
            .filter(|p| {
                let pb = Path::new(p.as_str());
                !self.by_path.contains_key(pb) && !self.requested_extra.contains(pb)
            })
            .cloned()
            .collect();
        if missing.is_empty() || !self.loaded {
            return;
        }
        for p in &missing {
            self.requested_extra.insert(PathBuf::from(p));
        }
        self.worker.send(Request::Probe(missing));
    }

    pub(crate) fn album(&self, key: &str) -> Option<&AlbumItem> {
        self.albums.iter().find(|a| a.key == key)
    }

    pub(crate) fn album_rows(&self, key: &str) -> &[usize] {
        self.album_rows.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Key of the album `path` belongs to.
    pub(crate) fn album_key_of(&self, path: &Path) -> Option<String> {
        let i = self.row_index(path)?;
        let row = &self.rows[i];
        (!row.album.trim().is_empty()).then(|| album_key(&row.album, &self.owners[i]))
    }

    /// Best album for a title (+ artist hint) from a context menu / Now Playing link.
    pub(crate) fn find_album(&self, title: &str, artist: &str) -> Option<String> {
        let mut candidates = self.albums.iter().filter(|a| a.title == title);
        let first = candidates.next()?;
        if first.artist == artist {
            return Some(first.key.clone());
        }
        Some(
            candidates
                .find(|a| a.artist == artist)
                .unwrap_or(first)
                .key
                .clone(),
        )
    }

    pub(crate) fn artist(&self, name: &str) -> Option<&ArtistEntry> {
        self.artist_pos.get(name).map(|&i| &self.artists[i])
    }

    pub(crate) fn paths_of(&self, rows: &[usize]) -> Vec<String> {
        rows.iter().map(|&i| self.rows[i].path.to_string_lossy().into_owned()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_row(path: &str, title: &str, artist: &str, album: &str, album_artist: &str) -> LibraryTrackRow {
        LibraryTrackRow {
            path: path.into(),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            album_artist: album_artist.into(),
            composer: String::new(),
        }
    }

    #[test]
    fn groups_albums_and_artists_and_applies_details() {
        let ctx = egui::Context::default();
        let mut index = LibraryIndex::new(ctx, false);
        let rows = vec![
            db_row("/m/a/2.flac", "Two", "X", "A", ""),
            db_row("/m/a/1.flac", "", "X feat. Y", "A", "X"),
            db_row("/m/b/1.flac", "B1", "Y", "B", ""),
        ];
        let (rows, owners): (Vec<TrackRow>, Vec<String>) = rows.into_iter().map(row_from_db).unzip();
        index.rows = Arc::new(rows);
        index.owners = owners;
        index.rebuild();
        assert_eq!(index.albums.len(), 2);
        let a = index.find_album("A", "X").unwrap();
        assert_eq!(index.album(&a).unwrap().track_count, 2);
        assert_eq!(index.display_row("/m/a/1.flac").title, "1", "untagged title falls back to the file stem");
        assert!(index.artist("X").is_some());
        assert_eq!(index.artist("X").unwrap().tracks.len(), 2);
        assert_eq!(index.display_row("/elsewhere/song.mp3").title, "song");

        // Track numbers order the album.
        let rows = Arc::make_mut(&mut index.rows);
        apply_details(&mut rows[0], &TrackDetails { track_no: Some(2), duration_ms: Some(1000), ..Default::default() });
        apply_details(&mut rows[1], &TrackDetails { track_no: Some(1), year: Some(2001), ..Default::default() });
        index.rebuild();
        let order = index.paths_of(index.album_rows(&a));
        assert_eq!(order, vec!["/m/a/1.flac".to_string(), "/m/a/2.flac".to_string()]);
        assert_eq!(index.album(&a).unwrap().year, Some(2001));
    }
}
