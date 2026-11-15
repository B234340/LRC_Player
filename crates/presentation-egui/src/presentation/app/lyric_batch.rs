//! Bulk download of lyrics from LRCLIB for every library track that has none
//! (Settings → 歌詞 → 「歌詞のない曲をまとめて取得」).
//!
//! - **Explicit action:** nothing is sent until the user confirms the dialog, which says what
//!   is sent (title, artist, album, duration of N tracks) to lrclib.net. It works even when
//!   the automatic lookup (`fetch_lyrics_online`) is off.
//! - **Count:** which library tracks have no lyrics is decided off the UI thread with the same
//!   discovery the player uses (`Lyrics::discover_for_track`: same-name `.lrc`, lyrics found
//!   by the lyrics-folder scan) plus attached files (`lyrics_overrides`). It is recomputed when
//!   the library, the attachments or the lyrics folders change, or after a run.
//! - **Worker:** ONE background thread asks LRCLIB for one track at a time, at most one lookup
//!   per [`MIN_INTERVAL`] (a lookup is at most two HTTP requests, so about 2 requests/s). HTTP
//!   429 / 5xx back off exponentially and retry; after [`MAX_CONSECUTIVE_FAILURES`] failed
//!   lookups in a row the run stops with the last error. Each track is checked again right
//!   before its lookup and skipped when it got lyrics meanwhile. Found lyrics are saved with
//!   [`save_lrc`] (beside the audio, never replacing a file; else the data folder + attached)
//!   and installed like the single-track lookup does (the playing track reloads its lyrics).
//! - **Cancel** wakes the worker from any wait at once; a request already on the wire is
//!   finished (bounded by the client timeout) and its result discarded.
//! - **Misses:** "not found", "instrumental" and "unsynced only" are remembered in
//!   `data/lyrics_fetch_misses.json` ([`MissCache`]) and skipped for [`MISS_TTL`] on later runs.
//!
//! The run itself ([`run_batch`]) only talks to a [`BatchIo`], so tests drive it without
//! network or timing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use eframe::egui::{self, Sense, Ui, Vec2};
use lrc_player_application::infrastructure::lyrics::{lrclib_fetch, Lyrics, LrclibQuery, LrclibResult};
use serde::{Deserialize, Serialize};

use crate::presentation::app::lyric_fetch::{save_lrc, track_query};
use crate::presentation::app::GuiApp;
use crate::presentation::ui::{
    format::format_count,
    i18n::Tr,
    icons::paint_icon,
    theme,
    views::{settings_row, TrackRow},
    widgets::{button, dialog, dialog_text, ButtonKind, DialogButton, DialogResult, DialogSpec},
    Icon, Page,
};

/// Minimum time between the starts of two lookups.
pub(crate) const MIN_INTERVAL: Duration = Duration::from_millis(1000);
/// Lookups that failed in a row (network / server) before the run gives up.
pub(crate) const MAX_CONSECUTIVE_FAILURES: u32 = 5;
/// Retries of one track after HTTP 429 / 5xx, waiting `BUSY_BACKOFF * 2^n` (capped).
const BUSY_RETRIES: u32 = 3;
const BUSY_BACKOFF: Duration = Duration::from_secs(5);
const BUSY_BACKOFF_MAX: Duration = Duration::from_secs(60);
/// Pause after a network failure (times the failures in a row).
const NETWORK_BACKOFF: Duration = Duration::from_secs(2);
/// How long a miss is remembered.
pub(crate) const MISS_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// Entries kept in the miss cache at most (oldest dropped first).
const MISS_CACHE_MAX: usize = 20_000;
/// The worker writes the miss cache after this many new entries (and at the end).
const MISS_SAVE_EVERY: usize = 25;
const MISS_CACHE_FILE: &str = "lyrics_fetch_misses.json";
const MISS_CACHE_VERSION: u32 = 1;

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------------------------
// Miss cache
// ---------------------------------------------------------------------------------------------

/// Why LRCLIB had nothing usable for a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MissKind {
    NotFound,
    Instrumental,
    PlainOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct MissEntry {
    result: MissKind,
    /// Unix time (s) of the lookup.
    time: u64,
}

#[derive(Serialize, Deserialize)]
struct MissFile {
    version: u32,
    misses: HashMap<String, MissEntry>,
}

/// `data/lyrics_fetch_misses.json`: audio path -> last unsuccessful lookup. Loaded lazily by a
/// worker; a missing, unreadable or corrupt file counts as empty (and is replaced on the next
/// save). Without a path (no persistence) it lives in memory only.
#[derive(Debug, Default)]
pub(crate) struct MissCache {
    path: Option<PathBuf>,
    entries: HashMap<String, MissEntry>,
    loaded: bool,
    dirty: bool,
}

impl MissCache {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self { path, ..Default::default() }
    }

    /// `data/lyrics_fetch_misses.json` next to the executable.
    pub(crate) fn default_path() -> Option<PathBuf> {
        Some(std::env::current_exe().ok()?.parent()?.join("data").join(MISS_CACHE_FILE))
    }

    /// Reads the file once (blocking).
    pub(crate) fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let Some(path) = &self.path else { return };
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::warn!("reading {}: {e}", path.display());
                return;
            }
        };
        match serde_json::from_str::<MissFile>(&text) {
            Ok(file) if file.version == MISS_CACHE_VERSION => {
                for (k, v) in file.misses {
                    self.entries.entry(k).or_insert(v);
                }
            }
            Ok(file) => tracing::info!("{}: unknown version {}, starting over", path.display(), file.version),
            Err(e) => tracing::warn!("{} is corrupt, starting over: {e}", path.display()),
        }
    }

    /// Whether `track` was looked up without success less than [`MISS_TTL`] before `now`.
    pub(crate) fn is_recent(&self, track: &str, now: u64) -> bool {
        self.entries.get(track).is_some_and(|e| now.saturating_sub(e.time) < MISS_TTL.as_secs() && e.time <= now + 86_400)
    }

    pub(crate) fn record(&mut self, track: &str, result: MissKind, now: u64) {
        self.entries.insert(track.to_string(), MissEntry { result, time: now });
        self.dirty = true;
        if self.entries.len() > MISS_CACHE_MAX + MISS_CACHE_MAX / 10 {
            self.prune(now);
        }
    }

    pub(crate) fn remove(&mut self, track: &str) {
        if self.entries.remove(track).is_some() {
            self.dirty = true;
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Drops expired entries, then the oldest beyond [`MISS_CACHE_MAX`].
    pub(crate) fn prune(&mut self, now: u64) {
        let before = self.entries.len();
        self.entries.retain(|_, e| now.saturating_sub(e.time) < MISS_TTL.as_secs());
        if self.entries.len() > MISS_CACHE_MAX {
            let mut times: Vec<u64> = self.entries.values().map(|e| e.time).collect();
            times.sort_unstable_by(|a, b| b.cmp(a));
            let cutoff = times[MISS_CACHE_MAX - 1];
            self.entries.retain(|_, e| e.time >= cutoff);
            // Ties at the cutoff: drop arbitrary extras.
            let extra = self.entries.len().saturating_sub(MISS_CACHE_MAX);
            let drop: Vec<String> = self.entries.iter().filter(|(_, e)| e.time == cutoff).take(extra).map(|(k, _)| k.clone()).collect();
            for k in drop {
                self.entries.remove(&k);
            }
        }
        self.dirty |= self.entries.len() != before;
    }

    /// Writes the file when something changed (atomically: temp file + rename).
    pub(crate) fn save(&mut self, now: u64) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        self.prune(now);
        self.dirty = false;
        let Some(path) = &self.path else { return Ok(()) };
        let file = MissFile { version: MISS_CACHE_VERSION, misses: self.entries.clone() };
        let json = serde_json::to_string(&file).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

// ---------------------------------------------------------------------------------------------
// The run (worker side)
// ---------------------------------------------------------------------------------------------

/// One track to look up.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BatchItem {
    pub path: String,
    pub query: LrclibQuery,
}

/// Running totals of a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct BatchCounts {
    /// Tracks handled so far (any outcome) / in the run.
    pub done: usize,
    pub total: usize,
    pub saved: usize,
    pub not_found: usize,
    pub instrumental: usize,
    pub plain_only: usize,
    pub errors: usize,
    /// Skipped: looked up without success within [`MISS_TTL`].
    pub recent: usize,
    /// Skipped: got lyrics since the count.
    pub has_lyrics: usize,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BatchEnd {
    Completed,
    Cancelled,
    /// Stopped after repeated failures (last error).
    Aborted(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BatchEvent {
    Progress(BatchCounts),
    Saved { track: String, path: PathBuf, attached: bool },
    Finished(BatchCounts, BatchEnd),
}

/// A failed lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FetchError {
    /// HTTP 429 / 5xx: the server asks to slow down or has trouble (retried after a pause).
    Busy(String),
    /// No connection, timeout, ... (counts towards stopping the run).
    Network(String),
    /// Anything else about this one track (unexpected status, bad response).
    Other(String),
}

impl FetchError {
    fn message(&self) -> &str {
        match self {
            FetchError::Busy(m) | FetchError::Network(m) | FetchError::Other(m) => m,
        }
    }
}

/// The HTTP status in an LRCLIB error message ("LRCLIB get: HTTP 429").
fn http_status(msg: &str) -> Option<u16> {
    let at = msg.find("HTTP ")? + 5;
    let digits: String = msg[at..].chars().take_while(char::is_ascii_digit).collect();
    (digits.len() == 3).then(|| digits.parse().ok()).flatten()
}

pub(crate) fn classify_error(err: &anyhow::Error) -> FetchError {
    let msg = format!("{err:#}");
    match http_status(&msg) {
        Some(s) if s == 429 || (500..600).contains(&s) => FetchError::Busy(msg),
        Some(_) => FetchError::Other(msg),
        None if msg.contains("invalid JSON") => FetchError::Other(msg),
        None => FetchError::Network(msg),
    }
}

/// Everything a run does outside itself.
pub(crate) trait BatchIo {
    /// Whether `track` has lyrics now (local discovery).
    fn has_lyrics(&mut self, track: &str) -> bool;
    fn fetch(&mut self, query: &LrclibQuery) -> Result<LrclibResult, FetchError>;
    /// Writes found lyrics for `track` (see [`save_lrc`]): (file, attached).
    fn save(&mut self, track: &str, lrc: &str) -> Result<(PathBuf, bool), String>;
    /// Waits `d`; false when the run was cancelled meanwhile (returns at once then).
    fn wait(&mut self, d: Duration) -> bool;
    fn cancelled(&mut self) -> bool;
    /// Unix time (s).
    fn now(&self) -> u64 {
        unix_now()
    }
    fn min_interval(&self) -> Duration {
        MIN_INTERVAL
    }
}

/// Looks `items` up one after the other (see the module docs). `emit` gets progress and saved
/// files; the result is also sent as [`BatchEvent::Finished`].
pub(crate) fn run_batch(items: &[BatchItem], io: &mut dyn BatchIo, misses: &Mutex<MissCache>, emit: &mut dyn FnMut(BatchEvent)) -> (BatchCounts, BatchEnd) {
    let mut c = BatchCounts { total: items.len(), ..Default::default() };
    lock(misses).ensure_loaded();
    let mut last_fetch: Option<Instant> = None;
    let mut failures = 0u32;
    let mut unsaved = 0usize;
    let mut end = BatchEnd::Completed;

    'items: for item in items {
        if io.cancelled() {
            end = BatchEnd::Cancelled;
            break;
        }
        if io.has_lyrics(&item.path) {
            c.has_lyrics += 1;
        } else if lock(misses).is_recent(&item.path, io.now()) {
            c.recent += 1;
        } else {
            let mut busy_tries = 0u32;
            let result = loop {
                if let Some(t) = last_fetch {
                    let pause = io.min_interval().saturating_sub(t.elapsed());
                    if !pause.is_zero() && !io.wait(pause) {
                        end = BatchEnd::Cancelled;
                        break 'items;
                    }
                }
                last_fetch = Some(Instant::now());
                match io.fetch(&item.query) {
                    Err(FetchError::Busy(msg)) if busy_tries < BUSY_RETRIES => {
                        let pause = BUSY_BACKOFF.saturating_mul(1 << busy_tries).min(BUSY_BACKOFF_MAX);
                        busy_tries += 1;
                        tracing::info!("LRCLIB busy ({msg}), retrying in {pause:?}");
                        if !io.wait(pause) {
                            end = BatchEnd::Cancelled;
                            break 'items;
                        }
                    }
                    other => break other,
                }
            };
            if io.cancelled() {
                // A late answer after "Stop": not saved.
                end = BatchEnd::Cancelled;
                break;
            }
            let now = io.now();
            let miss = |kind: MissKind, unsaved: &mut usize| {
                lock(misses).record(&item.path, kind, now);
                *unsaved += 1;
            };
            match result {
                Ok(LrclibResult::Synced { lrc, .. }) => {
                    failures = 0;
                    match io.save(&item.path, &lrc) {
                        Ok((path, attached)) => {
                            c.saved += 1;
                            lock(misses).remove(&item.path);
                            emit(BatchEvent::Saved { track: item.path.clone(), path, attached });
                        }
                        Err(e) => {
                            tracing::warn!("saving lyrics for {}: {e}", item.path);
                            c.errors += 1;
                        }
                    }
                }
                Ok(LrclibResult::NotFound) => {
                    failures = 0;
                    c.not_found += 1;
                    miss(MissKind::NotFound, &mut unsaved);
                }
                Ok(LrclibResult::Instrumental) => {
                    failures = 0;
                    c.instrumental += 1;
                    miss(MissKind::Instrumental, &mut unsaved);
                }
                Ok(LrclibResult::PlainOnly) => {
                    failures = 0;
                    c.plain_only += 1;
                    miss(MissKind::PlainOnly, &mut unsaved);
                }
                Err(err) => {
                    tracing::warn!("LRCLIB lookup for {} failed: {}", item.path, err.message());
                    c.errors += 1;
                    if !matches!(err, FetchError::Other(_)) {
                        failures += 1;
                        if failures >= MAX_CONSECUTIVE_FAILURES {
                            c.done += 1;
                            end = BatchEnd::Aborted(err.message().to_string());
                            break;
                        }
                        if matches!(err, FetchError::Network(_)) && !io.wait(NETWORK_BACKOFF.saturating_mul(failures)) {
                            c.done += 1;
                            end = BatchEnd::Cancelled;
                            break;
                        }
                    }
                }
            }
            if unsaved >= MISS_SAVE_EVERY {
                unsaved = 0;
                if let Err(e) = lock(misses).save(now) {
                    tracing::warn!("saving the lyrics miss cache: {e}");
                }
            }
        }
        c.done += 1;
        emit(BatchEvent::Progress(c));
    }
    if let Err(e) = lock(misses).save(io.now()) {
        tracing::warn!("saving the lyrics miss cache: {e}");
    }
    emit(BatchEvent::Finished(c, end.clone()));
    (c, end)
}

/// Whether `track` has lyrics the player would find: an attached file, a same-name `.lrc` or
/// lyrics registered by the lyrics-folder scan (blocking; worker threads only).
pub(crate) fn has_local_lyrics(track: &str, overrides: &HashMap<String, String>, lrc_targets: &[String]) -> bool {
    overrides.get(track).is_some_and(|p| Path::new(p).is_file()) || !Lyrics::discover_for_track(track, lrc_targets).is_empty()
}

pub(crate) type FetchFn = Box<dyn FnMut(&LrclibQuery) -> Result<LrclibResult, FetchError> + Send>;

/// The real [`BatchIo`]: disk + `fetch` (LRCLIB), cancelled through a channel.
struct LiveIo {
    cancel: Receiver<()>,
    cancelled: bool,
    overrides: HashMap<String, String>,
    lrc_targets: Vec<String>,
    fallback: Option<PathBuf>,
    fetch: FetchFn,
    interval: Duration,
}

impl BatchIo for LiveIo {
    fn has_lyrics(&mut self, track: &str) -> bool {
        has_local_lyrics(track, &self.overrides, &self.lrc_targets)
    }

    fn fetch(&mut self, query: &LrclibQuery) -> Result<LrclibResult, FetchError> {
        (self.fetch)(query)
    }

    fn save(&mut self, track: &str, lrc: &str) -> Result<(PathBuf, bool), String> {
        save_lrc(Path::new(track), lrc, self.fallback.as_deref())
    }

    fn wait(&mut self, d: Duration) -> bool {
        if self.cancelled {
            return false;
        }
        match self.cancel.recv_timeout(d) {
            Err(RecvTimeoutError::Timeout) => true,
            _ => {
                self.cancelled = true;
                false
            }
        }
    }

    fn cancelled(&mut self) -> bool {
        if !self.cancelled && !matches!(self.cancel.try_recv(), Err(TryRecvError::Empty)) {
            self.cancelled = true;
        }
        self.cancelled
    }

    fn min_interval(&self) -> Duration {
        self.interval
    }
}

// ---------------------------------------------------------------------------------------------
// UI side
// ---------------------------------------------------------------------------------------------

/// What the count depends on.
#[derive(Debug, Clone, PartialEq)]
struct CountKey {
    /// Hash of the library's paths (detail updates that only fill in tags do not count).
    library_paths: u64,
    overrides: usize,
    lrc_targets: Vec<String>,
    generation: u64,
}

/// Library tracks without lyrics.
#[derive(Debug, Clone, Default)]
pub(crate) struct MissingLyrics {
    pub paths: Vec<String>,
    /// How many of them were looked up without success recently (skipped by a run).
    pub recent: usize,
}

impl MissingLyrics {
    /// Tracks a run would actually send.
    pub(crate) fn to_query(&self) -> usize {
        self.paths.len().saturating_sub(self.recent)
    }
}

struct BatchJob {
    /// Dropping it (or sending) cancels the worker.
    cancel: Option<Sender<()>>,
    rx: Receiver<BatchEvent>,
    counts: BatchCounts,
}

/// State of the bulk download (in `UiState`).
#[derive(Default)]
pub(crate) struct LyricBatch {
    misses: Option<Arc<Mutex<MissCache>>>,
    count: Option<(CountKey, MissingLyrics)>,
    counting: Option<(CountKey, Receiver<MissingLyrics>)>,
    generation: u64,
    /// `library_paths` of [`CountKey`] and the library revision it was computed for.
    paths_hash: (u64, u64),
    job: Option<BatchJob>,
    /// The confirmation dialog is open.
    pub(crate) confirm: bool,
    /// Result of the last run (shown under the row).
    last: Option<(BatchCounts, BatchEnd)>,
    /// Frames left to scroll the row into view (screenshots).
    pub(crate) scroll_into_view: u8,
}

impl LyricBatch {
    /// A count is there and not being recomputed.
    #[cfg(test)]
    pub(crate) fn has_count(&self) -> bool {
        self.count.is_some() && self.counting.is_none()
    }
}

impl GuiApp {
    pub(crate) fn lyric_misses(&mut self) -> Arc<Mutex<MissCache>> {
        let persist = self.persist;
        self.ui
            .lyric_batch
            .misses
            .get_or_insert_with(|| Arc::new(Mutex::new(MissCache::new(if persist { MissCache::default_path() } else { None }))))
            .clone()
    }

    fn lyric_count_key(&mut self) -> CountKey {
        let revision = self.library.revision;
        if self.ui.lyric_batch.paths_hash.1 != revision {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for r in self.library.rows.iter() {
                r.path.hash(&mut h);
            }
            self.ui.lyric_batch.paths_hash = (h.finish(), revision);
        }
        CountKey {
            library_paths: self.ui.lyric_batch.paths_hash.0,
            overrides: self.lyrics_overrides.len(),
            lrc_targets: self.scan_lrc_targets.clone(),
            generation: self.ui.lyric_batch.generation,
        }
    }

    /// The current count of library tracks without lyrics (`None` while it is computed).
    /// Starts the computation on a worker when the inputs changed (call while it is shown).
    pub(crate) fn missing_lyrics(&mut self, ctx: &egui::Context) -> Option<&MissingLyrics> {
        let key = self.lyric_count_key();
        let b = &self.ui.lyric_batch;
        let stale = b.count.as_ref().is_none_or(|(k, _)| *k != key);
        if stale && b.counting.is_none() && b.job.is_none() && self.library.loaded {
            let rows: Arc<Vec<TrackRow>> = self.library.rows.clone();
            let overrides = self.lyrics_overrides.clone();
            let targets = self.scan_lrc_targets.clone();
            let misses = self.lyric_misses();
            let (tx, rx) = crossbeam_channel::bounded(1);
            let wake = ctx.clone();
            let spawned = std::thread::Builder::new().name("lyrics-count".into()).spawn(move || {
                let paths: Vec<String> = rows
                    .iter()
                    .map(|r| r.path.to_string_lossy().into_owned())
                    .filter(|p| !has_local_lyrics(p, &overrides, &targets))
                    .collect();
                let now = unix_now();
                let recent = {
                    let mut m = lock(&misses);
                    m.ensure_loaded();
                    paths.iter().filter(|p| m.is_recent(p, now)).count()
                };
                let _ = tx.send(MissingLyrics { paths, recent });
                wake.request_repaint();
            });
            if spawned.is_ok() {
                self.ui.lyric_batch.counting = Some((key, rx));
            }
        }
        // While counting again, the previous count stays (a run re-checks every track anyway).
        self.ui.lyric_batch.count.as_ref().map(|(_, m)| m)
    }

    /// `Some((counts, stopping))` while a run is going.
    pub(crate) fn lyrics_batch_progress(&self) -> Option<(BatchCounts, bool)> {
        self.ui.lyric_batch.job.as_ref().map(|j| (j.counts, j.cancel.is_none()))
    }

    /// Starts the run for the counted tracks (after the confirmation).
    pub(crate) fn start_lyrics_batch(&mut self, ctx: &egui::Context) {
        let fetch: FetchFn = Box::new(|q: &LrclibQuery| lrclib_fetch(q).map_err(|e| classify_error(&e)));
        self.start_lyrics_batch_with(ctx, fetch, MIN_INTERVAL);
    }

    pub(crate) fn start_lyrics_batch_with(&mut self, ctx: &egui::Context, fetch: FetchFn, interval: Duration) {
        if self.ui.lyric_batch.job.is_some() {
            return;
        }
        let Some((_, missing)) = &self.ui.lyric_batch.count else { return };
        let items: Vec<BatchItem> = missing
            .paths
            .iter()
            .filter_map(|p| {
                let row = &self.library.rows[self.library.row_index(Path::new(p))?];
                let query = track_query(p, Some(&row.title), Some(&row.artist), Some(&row.album), row.duration_ms);
                Some(BatchItem { path: p.clone(), query })
            })
            .collect();
        if items.is_empty() {
            return;
        }
        let misses = self.lyric_misses();
        let (cancel_tx, cancel_rx) = crossbeam_channel::bounded(1);
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut io = LiveIo {
            cancel: cancel_rx,
            cancelled: false,
            overrides: self.lyrics_overrides.clone(),
            lrc_targets: self.scan_lrc_targets.clone(),
            fallback: self.lyrics_fallback_dir(),
            fetch,
            interval,
        };
        let total = items.len();
        let wake = ctx.clone();
        let spawned = std::thread::Builder::new().name("lyrics-batch".into()).spawn(move || {
            let mut emit = |e: BatchEvent| {
                let _ = tx.send(e);
                wake.request_repaint();
            };
            run_batch(&items, &mut io, &misses, &mut emit);
        });
        match spawned {
            Ok(_) => {
                let b = &mut self.ui.lyric_batch;
                b.last = None;
                b.job = Some(BatchJob { cancel: Some(cancel_tx), rx, counts: BatchCounts { total, ..Default::default() } });
            }
            Err(e) => {
                let tr = self.tr();
                self.toast_error(tr.f("toast.lyrics_fetch_failed", &[&e]));
            }
        }
    }

    /// Asks the worker to stop (it finishes at once unless a request is on the wire).
    pub(crate) fn cancel_lyrics_batch(&mut self) {
        if let Some(job) = &mut self.ui.lyric_batch.job {
            if let Some(tx) = job.cancel.take() {
                let _ = tx.try_send(());
            }
        }
    }

    /// Applies worker results (called every frame).
    pub(crate) fn poll_lyric_batch(&mut self) {
        if self.ui.lyric_batch.confirm && self.ui.page != Page::Settings {
            self.ui.lyric_batch.confirm = false;
        }
        if let Some((key, rx)) = &self.ui.lyric_batch.counting {
            match rx.try_recv() {
                Ok(m) => {
                    let key = key.clone();
                    let b = &mut self.ui.lyric_batch;
                    b.count = Some((key, m));
                    b.counting = None;
                }
                Err(TryRecvError::Disconnected) => self.ui.lyric_batch.counting = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        let mut finished = None;
        let mut saved = Vec::new();
        if let Some(job) = &mut self.ui.lyric_batch.job {
            loop {
                match job.rx.try_recv() {
                    Ok(BatchEvent::Progress(c)) => job.counts = c,
                    Ok(BatchEvent::Saved { track, path, attached }) => saved.push((track, path, attached)),
                    Ok(BatchEvent::Finished(c, end)) => {
                        finished = Some((c, end));
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished = Some((job.counts, BatchEnd::Aborted("worker stopped".into())));
                        break;
                    }
                }
            }
        }
        for (track, path, attached) in saved {
            self.install_fetched_lyrics(&track, &path, attached);
        }
        if let Some((counts, end)) = finished {
            let b = &mut self.ui.lyric_batch;
            b.job = None;
            b.generation += 1;
            b.last = Some((counts, end.clone()));
            let tr = self.tr();
            let summary = batch_summary(&counts, tr, false);
            match end {
                BatchEnd::Completed => self.toast_success(tr.f("toast.lyrics_batch_done", &[&summary])),
                BatchEnd::Cancelled => self.toast_info(tr.f("toast.lyrics_batch_cancelled", &[&summary])),
                BatchEnd::Aborted(err) => self.toast_error(tr.f("toast.lyrics_batch_aborted", &[&summary, &err])),
            }
        }
    }
}

/// "保存 3・見つからない 2・…" (`all`: every count, else only those above zero; "saved" always).
pub(crate) fn batch_summary(c: &BatchCounts, tr: Tr, all: bool) -> String {
    let parts = [
        ("lyrics_batch.saved", c.saved, true),
        ("lyrics_batch.not_found", c.not_found, all),
        ("lyrics_batch.instrumental", c.instrumental, all),
        ("lyrics_batch.plain_only", c.plain_only, all),
        ("lyrics_batch.errors", c.errors, all),
        ("lyrics_batch.recent", c.recent, false),
        ("lyrics_batch.has_lyrics", c.has_lyrics, false),
    ];
    parts
        .iter()
        .filter(|(_, n, always)| *always || *n > 0)
        .map(|(key, n, _)| tr.f(key, &[&format_count(*n)]))
        .collect::<Vec<_>>()
        .join(tr.t("lyrics_batch.sep"))
}

/// Rough duration of a run of `n` lookups.
fn estimate(n: usize, tr: Tr) -> String {
    let minutes = (n as u64 * MIN_INTERVAL.as_millis() as u64).div_ceil(60_000);
    if minutes <= 1 { tr.t("lyrics_batch.about_a_minute").to_string() } else { tr.f("lyrics_batch.minutes", &[&format_count(minutes as usize)]) }
}

/// Settings → 歌詞: the bulk download row, its status line and the confirmation dialog.
pub(crate) fn lyrics_batch_row(app: &mut GuiApp, ui: &mut Ui, tr: Tr) {
    let pal = theme::palette(ui);
    let ctx = ui.ctx().clone();
    if app.ui.lyric_batch.scroll_into_view > 0 {
        app.ui.lyric_batch.scroll_into_view -= 1;
        ui.scroll_to_cursor(Some(egui::Align::Center));
    }
    let missing = app.missing_lyrics(&ctx).cloned();
    let progress = app.lyrics_batch_progress();
    settings_row(ui, false, tr.t("settings.lyrics_batch"), Some(tr.t("settings.lyrics_batch_hint")), |ui| {
        let rtl = ui.layout().prefer_right_to_left();
        // Digits ("9 / 19") in monospace so they do not jitter while counting up.
        // Spinner left of the text in either layout direction.
        let status = |ui: &mut Ui, text: String, spin: bool, mono: bool| {
            let text = egui::RichText::new(text).color(pal.text_muted);
            let text = if mono { text.monospace() } else { text };
            if spin && !rtl {
                ui.add(egui::Spinner::new().size(14.0));
            }
            ui.label(text);
            if spin && rtl {
                ui.add(egui::Spinner::new().size(14.0));
            }
        };
        match progress {
            Some((c, stopping)) => {
                let text = if stopping { tr.t("settings.lyrics_batch_stopping").to_string() } else { format!("{} / {}", format_count(c.done), format_count(c.total)) };
                let mut cancel = |ui: &mut Ui| {
                    if !stopping && button(ui, ButtonKind::Ghost, Some(Icon::Close), tr.t("settings.lyrics_batch_cancel")).clicked() {
                        app.cancel_lyrics_batch();
                    }
                };
                if rtl {
                    cancel(ui);
                    status(ui, text, true, !stopping);
                } else {
                    status(ui, text, true, !stopping);
                    cancel(ui);
                }
            }
            None => {
                let can_start = missing.as_ref().is_some_and(|m| m.to_query() > 0);
                let text = match &missing {
                    None => tr.t("settings.lyrics_batch_counting").to_string(),
                    Some(m) => tr.f("settings.lyrics_batch_count", &[&format_count(m.paths.len())]),
                };
                let mut start = |ui: &mut Ui| {
                    let r = ui.add_enabled_ui(can_start, |ui| button(ui, ButtonKind::Secondary, Some(Icon::ArrowDownToLine), tr.t("settings.lyrics_batch_start"))).inner;
                    let r = match &missing {
                        Some(m) if m.paths.is_empty() => r.on_disabled_hover_text(tr.t("settings.lyrics_batch_none")),
                        Some(m) if m.to_query() == 0 => r.on_disabled_hover_text(tr.t("settings.lyrics_batch_all_recent")),
                        _ => r,
                    };
                    if r.clicked() {
                        app.ui.lyric_batch.confirm = true;
                    }
                };
                if rtl {
                    start(ui);
                    status(ui, text, missing.is_none(), false);
                } else {
                    status(ui, text, missing.is_none(), false);
                    start(ui);
                }
            }
        }
    });

    // Status line: live counts, the last result, or what a run would skip.
    let line = match (progress, &app.ui.lyric_batch.last, &missing) {
        (Some((c, _)), _, _) => Some((batch_summary(&c, tr, true), None)),
        (None, Some((c, end)), _) => {
            let icon = match end {
                BatchEnd::Completed => (Icon::Success, pal.success),
                BatchEnd::Cancelled => (Icon::Info, pal.text_muted),
                BatchEnd::Aborted(_) => (Icon::Warning, pal.warning),
            };
            let summary = batch_summary(c, tr, false);
            let text = match end {
                BatchEnd::Completed => tr.f("settings.lyrics_batch_last", &[&summary]),
                BatchEnd::Cancelled => tr.f("settings.lyrics_batch_last_cancelled", &[&summary]),
                BatchEnd::Aborted(err) => tr.f("settings.lyrics_batch_last_aborted", &[&summary, err]),
            };
            Some((text, Some(icon)))
        }
        (None, None, Some(m)) if m.recent > 0 => Some((tr.f("settings.lyrics_batch_recent", &[&format_count(m.recent)]), None)),
        _ => None,
    };
    if let Some((text, icon)) = line {
        // Tucked under the row it belongs to.
        ui.add_space(-8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if let Some((icon, color)) = icon {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                paint_icon(ui.painter(), r, icon, color);
            }
            ui.add(egui::Label::new(theme::caption(ui, text)).wrap());
        });
        ui.add_space(12.0);
    }

    if app.ui.lyric_batch.confirm {
        lyrics_batch_dialog(app, &ctx, tr, missing.as_ref());
    }
}

fn lyrics_batch_dialog(app: &mut GuiApp, ctx: &egui::Context, tr: Tr, missing: Option<&MissingLyrics>) {
    let Some(m) = missing.filter(|m| m.to_query() > 0) else {
        app.ui.lyric_batch.confirm = false;
        return;
    };
    let n = m.to_query();
    let spec = DialogSpec {
        id: egui::Id::new("lyrics_batch_confirm"),
        title: tr.t("dialog.lyrics_batch_title"),
        width: 460.0,
        confirm: DialogButton { kind: ButtonKind::Primary, icon: Some(Icon::ArrowDownToLine), label: tr.t("settings.lyrics_batch_start") },
        cancel: Some(tr.t("dialog.cancel")),
        extra: None,
    };
    let result = dialog(ctx, &spec, |ui, _| {
        ui.spacing_mut().item_spacing.y = 10.0;
        dialog_text(ui, &tr.f("dialog.lyrics_batch_body", &[&format_count(n)]));
        dialog_text(ui, tr.t("dialog.lyrics_batch_save"));
        dialog_text(ui, &tr.f("dialog.lyrics_batch_time", &[&estimate(n, tr)]));
        if m.recent > 0 {
            dialog_text(ui, &tr.f("dialog.lyrics_batch_recent", &[&format_count(m.recent)]));
        }
        true
    });
    match result {
        DialogResult::Confirm => {
            app.ui.lyric_batch.confirm = false;
            app.start_lyrics_batch(ctx);
        }
        DialogResult::Cancel | DialogResult::Extra => app.ui.lyric_batch.confirm = false,
        DialogResult::Open => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashSet, VecDeque};

    const DAY: u64 = 86_400;

    #[test]
    fn misses_expire_after_thirty_days() {
        let mut m = MissCache::new(None);
        m.ensure_loaded();
        let t0 = 1_700_000_000;
        m.record("/a.flac", MissKind::NotFound, t0);
        assert!(m.is_recent("/a.flac", t0 + 29 * DAY));
        assert!(!m.is_recent("/a.flac", t0 + 30 * DAY));
        assert!(!m.is_recent("/b.flac", t0));
        // A clock that jumped back far: not trusted.
        assert!(!m.is_recent("/a.flac", t0 - 3 * DAY));
        m.prune(t0 + 31 * DAY);
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn miss_cache_round_trips_and_tolerates_a_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data").join(MISS_CACHE_FILE);
        let now = unix_now();
        let mut m = MissCache::new(Some(path.clone()));
        m.ensure_loaded();
        m.record("/a.flac", MissKind::Instrumental, now);
        m.record("/old.flac", MissKind::NotFound, now - 40 * DAY);
        m.save(now).unwrap();
        let mut back = MissCache::new(Some(path.clone()));
        back.ensure_loaded();
        assert!(back.is_recent("/a.flac", now));
        assert_eq!(back.len(), 1, "expired entries are not written");
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"instrumental\""));

        std::fs::write(&path, "{ not json").unwrap();
        let mut corrupt = MissCache::new(Some(path.clone()));
        corrupt.ensure_loaded();
        assert_eq!(corrupt.len(), 0);
        corrupt.record("/b.flac", MissKind::PlainOnly, now);
        corrupt.save(now).unwrap();
        let mut again = MissCache::new(Some(path));
        again.ensure_loaded();
        assert!(again.is_recent("/b.flac", now) && !again.is_recent("/a.flac", now));
    }

    #[test]
    fn miss_cache_is_bounded() {
        let mut m = MissCache::new(None);
        m.ensure_loaded();
        let t0 = 1_700_000_000;
        for i in 0..(MISS_CACHE_MAX + MISS_CACHE_MAX / 10 + 5) {
            m.record(&format!("/{i}.flac"), MissKind::NotFound, t0 + i as u64);
        }
        assert!(m.len() <= MISS_CACHE_MAX + MISS_CACHE_MAX / 10);
        m.prune(t0 + 100_000);
        assert_eq!(m.len(), MISS_CACHE_MAX);
        // The newest survive.
        let newest = MISS_CACHE_MAX + MISS_CACHE_MAX / 10 + 4;
        assert!(m.is_recent(&format!("/{newest}.flac"), t0 + 100_000));
        assert!(!m.is_recent("/0.flac", t0 + 100_000));
    }

    #[test]
    fn http_errors_are_classified() {
        let e = |s: &str| classify_error(&anyhow::anyhow!(s.to_string()));
        assert!(matches!(e("LRCLIB get: HTTP 429"), FetchError::Busy(_)));
        assert!(matches!(e("LRCLIB search: HTTP 503"), FetchError::Busy(_)));
        assert!(matches!(e("LRCLIB search: HTTP 400"), FetchError::Other(_)));
        assert!(matches!(e("LRCLIB get: invalid JSON: eof"), FetchError::Other(_)));
        assert!(matches!(e("LRCLIB get: io: Connection refused"), FetchError::Network(_)));
    }

    /// Scripted [`BatchIo`]: answers per title, records waits; `cancel_after` cancels once
    /// that many lookups were made.
    #[derive(Default)]
    struct FakeIo {
        answers: HashMap<String, VecDeque<Result<LrclibResult, FetchError>>>,
        with_lyrics: HashSet<String>,
        fetched: Vec<String>,
        saved: Vec<String>,
        waits: Vec<Duration>,
        cancel_after: Option<usize>,
        cancel_in_wait: bool,
        cancelled: bool,
        now: u64,
    }

    impl FakeIo {
        fn answer(mut self, title: &str, r: Result<LrclibResult, FetchError>) -> Self {
            self.answers.entry(title.into()).or_default().push_back(r);
            self
        }
    }

    impl BatchIo for FakeIo {
        fn has_lyrics(&mut self, track: &str) -> bool {
            self.with_lyrics.contains(track)
        }
        fn fetch(&mut self, q: &LrclibQuery) -> Result<LrclibResult, FetchError> {
            self.fetched.push(q.title.clone());
            if self.cancel_after.is_some_and(|n| self.fetched.len() >= n) {
                self.cancelled = true;
            }
            self.answers.get_mut(&q.title).and_then(VecDeque::pop_front).unwrap_or(Ok(LrclibResult::NotFound))
        }
        fn save(&mut self, track: &str, _lrc: &str) -> Result<(PathBuf, bool), String> {
            self.saved.push(track.into());
            Ok((Path::new(track).with_extension("lrc"), false))
        }
        fn wait(&mut self, d: Duration) -> bool {
            self.waits.push(d);
            self.cancelled |= self.cancel_in_wait;
            !self.cancelled
        }
        fn cancelled(&mut self) -> bool {
            self.cancelled
        }
        fn now(&self) -> u64 {
            self.now
        }
        fn min_interval(&self) -> Duration {
            Duration::from_secs(3600)
        }
    }

    fn items(names: &[&str]) -> Vec<BatchItem> {
        names
            .iter()
            .map(|n| BatchItem { path: format!("/m/{n}.flac"), query: LrclibQuery { title: n.to_string(), ..Default::default() } })
            .collect()
    }

    fn synced() -> Result<LrclibResult, FetchError> {
        Ok(LrclibResult::Synced { lrc: "[00:01.00]x\n".into(), word_sync: false })
    }

    fn run(io: &mut FakeIo, misses: &Mutex<MissCache>, names: &[&str]) -> (BatchCounts, BatchEnd, Vec<BatchEvent>) {
        let mut events = Vec::new();
        let (c, end) = run_batch(&items(names), io, misses, &mut |e| events.push(e));
        (c, end, events)
    }

    #[test]
    fn batch_counts_outcomes_skips_tracks_with_lyrics_and_remembers_misses() {
        let t0 = 1_700_000_000;
        let misses = Mutex::new(MissCache::new(None));
        lock(&misses).record("/m/old.flac", MissKind::NotFound, t0 - DAY);
        let mut io = FakeIo { now: t0, ..Default::default() }
            .answer("a", synced())
            .answer("b", Ok(LrclibResult::NotFound))
            .answer("c", Ok(LrclibResult::Instrumental))
            .answer("d", Ok(LrclibResult::PlainOnly))
            .answer("e", Err(FetchError::Other("HTTP 400".into())));
        io.with_lyrics.insert("/m/has.flac".into());
        let (c, end, events) = run(&mut io, &misses, &["a", "has", "b", "old", "c", "d", "e"]);
        assert_eq!(end, BatchEnd::Completed);
        assert_eq!(
            c,
            BatchCounts { done: 7, total: 7, saved: 1, not_found: 1, instrumental: 1, plain_only: 1, errors: 1, recent: 1, has_lyrics: 1 }
        );
        assert_eq!(io.fetched, ["a", "b", "c", "d", "e"], "skipped tracks are not sent");
        assert_eq!(io.saved, ["/m/a.flac"]);
        assert!(events.contains(&BatchEvent::Saved { track: "/m/a.flac".into(), path: "/m/a.lrc".into(), attached: false }));
        assert_eq!(events.iter().filter(|e| matches!(e, BatchEvent::Progress(_))).count(), 7);
        assert_eq!(events.last(), Some(&BatchEvent::Finished(c, BatchEnd::Completed)));
        // Polite: a pause before every lookup but the first.
        assert_eq!(io.waits.len(), 4);
        let m = lock(&misses);
        for t in ["b", "c", "d"] {
            assert!(m.is_recent(&format!("/m/{t}.flac"), t0), "{t} remembered");
        }
        assert!(!m.is_recent("/m/a.flac", t0) && !m.is_recent("/m/e.flac", t0), "hits and errors are not");
        drop(m);

        // A later run skips the remembered misses; after 30 days they are tried again.
        let mut io2 = FakeIo { now: t0 + DAY, ..Default::default() };
        let (c2, _, _) = run(&mut io2, &misses, &["b", "c", "d"]);
        assert_eq!((c2.recent, io2.fetched.len()), (3, 0));
        let mut io3 = FakeIo { now: t0 + 31 * DAY, ..Default::default() };
        let (c3, _, _) = run(&mut io3, &misses, &["b", "c", "d"]);
        assert_eq!((c3.recent, c3.not_found), (0, 3));
    }

    #[test]
    fn cancel_stops_without_saving_a_late_answer() {
        let misses = Mutex::new(MissCache::new(None));
        let mut io = FakeIo { cancel_after: Some(2), ..Default::default() }.answer("a", Ok(LrclibResult::NotFound)).answer("b", synced());
        let (c, end, events) = run(&mut io, &misses, &["a", "b", "c", "d"]);
        assert_eq!(end, BatchEnd::Cancelled);
        assert_eq!(io.fetched, ["a", "b"]);
        assert!(io.saved.is_empty(), "the answer that arrived after Stop is dropped");
        assert_eq!((c.done, c.not_found, c.saved), (1, 1, 0));
        assert_eq!(events.last(), Some(&BatchEvent::Finished(c, BatchEnd::Cancelled)));

        // Cancelled during the pause between lookups: the next track is not sent.
        let mut io = FakeIo { cancel_in_wait: true, ..Default::default() };
        let (c, end, _) = run(&mut io, &misses, &["x", "y", "z"]);
        assert_eq!((end, io.fetched.len(), io.waits.len(), c.done), (BatchEnd::Cancelled, 1, 1, 1));
    }

    #[test]
    fn busy_server_is_retried_with_backoff_and_failures_stop_the_run() {
        let misses = Mutex::new(MissCache::new(None));
        let mut io = FakeIo::default()
            .answer("a", Err(FetchError::Busy("HTTP 429".into())))
            .answer("a", Err(FetchError::Busy("HTTP 503".into())))
            .answer("a", synced());
        let (c, end, _) = run(&mut io, &misses, &["a"]);
        assert_eq!((end, c.saved, c.errors), (BatchEnd::Completed, 1, 0));
        assert_eq!(io.fetched.len(), 3);
        assert!(io.waits.contains(&BUSY_BACKOFF) && io.waits.contains(&(BUSY_BACKOFF * 2)));

        let mut io = FakeIo::default();
        for n in ["a", "b", "c", "d", "e", "f", "g"] {
            io = io.answer(n, Err(FetchError::Network("connection refused".into())));
        }
        let (c, end, _) = run(&mut io, &misses, &["a", "b", "c", "d", "e", "f", "g"]);
        assert_eq!(end, BatchEnd::Aborted("connection refused".into()));
        assert_eq!((c.errors, c.done, io.fetched.len()), (5, 5, 5));

        // A success in between resets the streak.
        let mut io = FakeIo::default();
        for n in ["a", "b", "c", "d", "f", "g", "h", "i"] {
            io = io.answer(n, Err(FetchError::Network("timeout".into())));
        }
        let (c, end, _) = run(&mut io, &misses, &["a", "b", "c", "d", "ok", "f", "g", "h", "i"]);
        assert_eq!((end, c.errors, c.not_found), (BatchEnd::Completed, 8, 1));
    }

    #[test]
    fn summary_lists_the_counts() {
        let c = BatchCounts { saved: 3, not_found: 2, recent: 4, ..Default::default() };
        assert_eq!(batch_summary(&c, Tr::JA, false), "保存 3・見つからない 2・最近確認済み 4");
        let all = batch_summary(&c, Tr::JA, true);
        assert!(all.contains("エラー 0") && all.contains("テキストのみ 0"));
        assert_eq!(batch_summary(&BatchCounts::default(), Tr::EN, false), "0 saved");
    }

    /// The whole flow on the app: count (off the UI thread), run with a fake LRCLIB, save
    /// beside the audio, reload the playing track, toast, recount.
    #[test]
    fn app_counts_runs_and_reloads_the_playing_track() {
        use crate::presentation::app::tests::test_app;
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx, dir.path(), false);
        let track = |n: &str| dir.path().join(format!("{n}.flac")).to_string_lossy().into_owned();
        std::fs::write(dir.path().join("has.lrc"), "[00:01.00]here\n").unwrap();
        {
            let rows = Arc::make_mut(&mut app.library.rows);
            for n in ["has", "song", "missing", "old"] {
                rows.push(TrackRow { path: track(n).into(), title: n.into(), artist: "A".into(), ..Default::default() });
            }
        }
        app.library.loaded = true;
        app.library.rebuild();
        app.library.revision += 1;
        lock(&app.lyric_misses()).record(&track("old"), MissKind::NotFound, unix_now());
        app.play_path = track("song");

        let wait_for = |app: &mut GuiApp, done: &dyn Fn(&mut GuiApp) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !done(app) {
                assert!(Instant::now() < deadline, "timed out");
                app.poll_lyric_batch();
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait_for(&mut app, &|a| a.missing_lyrics(&ctx).is_some());
        let m = app.missing_lyrics(&ctx).cloned().unwrap();
        let mut names: Vec<_> = m.paths.iter().map(|p| Path::new(p).file_stem().unwrap().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!((names, m.recent, m.to_query()), (vec!["missing".to_string(), "old".into(), "song".into()], 1, 2));

        let sent = Arc::new(Mutex::new(Vec::new()));
        let log = sent.clone();
        let fetch: FetchFn = Box::new(move |q: &LrclibQuery| {
            lock(&log).push((q.title.clone(), q.artist.clone()));
            Ok(if q.title == "song" { LrclibResult::Synced { lrc: "[00:01.00]fetched\n".into(), word_sync: false } } else { LrclibResult::NotFound })
        });
        app.start_lyrics_batch_with(&ctx, fetch, Duration::ZERO);
        assert!(app.lyrics_batch_progress().is_some());
        wait_for(&mut app, &|a| a.lyrics_batch_progress().is_none());

        let mut sent = lock(&sent).clone();
        sent.sort();
        assert_eq!(sent, [("missing".to_string(), "A".to_string()), ("song".into(), "A".into())], "only title/artist/album/duration, recent misses skipped");
        assert_eq!(std::fs::read_to_string(dir.path().join("song.lrc")).unwrap(), "[00:01.00]fetched\n");
        assert_eq!(app.loaded_timeline.as_ref().unwrap().lines()[0].text, "fetched", "playing track reloaded");
        let (c, end) = app.ui.lyric_batch.last.clone().unwrap();
        assert_eq!(end, BatchEnd::Completed);
        assert_eq!((c.total, c.saved, c.not_found, c.recent), (3, 1, 1, 1));
        assert!(!app.ui.toasts.is_empty());
        assert!(lock(&app.lyric_misses()).is_recent(&track("missing"), unix_now()));

        // Recounted after the run: "song" has lyrics now.
        wait_for(&mut app, &|a| a.missing_lyrics(&ctx).is_some_and(|m| m.paths.len() == 2));
    }
}
