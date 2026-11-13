//! EBU R128 loudness analysis for files without ReplayGain tags.
//!
//! The engine normalizes tracks from their ReplayGain tags by itself. For untagged files it
//! uses a [`LoudnessHint`] that this module measures ([`measure_loudness`]) and hands over
//! with [`AudioEngine::set_loudness_hint`].
//!
//! - **One worker thread** at background priority
//!   ([`lower_current_thread_priority`]) works through a priority queue: the current track,
//!   then the gapless successor (`pending_next`), then everything else (library analysis).
//!   A current/next request preempts a running background measurement, which is requeued.
//!   Measuring costs about 0.12 s of CPU per minute of audio, so a new track normally gets its
//!   hint within a second; a hint that arrives while the track already plays is ramped in by
//!   the engine.
//! - **Tag check first:** files with ReplayGain tags are only recorded as "tagged" (the engine
//!   reads the tags itself) and never decoded.
//! - **Cache:** results are kept in `data/loudness.json` next to the executable, keyed by path
//!   and validated by file size + modification time, loaded on start, and written by the
//!   worker in the background. Nothing is read or written when persistence is off (tests).
//! - **Album gain** is added to a hint once every library track of the same album (album
//!   artist + album, the library index's album key) has a cached measurement; until then the
//!   hint carries the track gain only (the engine then uses the track gain in album mode).
//! - **Hooks:** `playback.rs` calls [`GuiApp::prepare_loudness`] before `AudioEngine::load`
//!   and before `set_next`, so a cached hint is set before the engine opens the file.
//!   [`GuiApp::poll_loudness`] applies worker results (and re-sends the current / next hints
//!   when an album becomes complete).
//!
//! # Control API for the settings UI
//!
//! | method | effect |
//! |---|---|
//! | [`GuiApp::set_normalization`] / [`GuiApp::normalization`] | mode, preamp, clipping guard (saved in `config.json`) |
//! | [`GuiApp::analyze_library_loudness`] | queue every library track (background priority) |
//! | [`GuiApp::cancel_loudness_analysis`] | drop the queued library work |
//! | [`GuiApp::loudness_analysis_progress`] | `Some((done, total))` while an analysis runs |

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use lrc_player_application::config::{Config, NormalizationModeSetting};
use lrc_player_application::infrastructure::audio::engine::loudness::{
    album_gain, has_replaygain_tags, lower_current_thread_priority, measure_loudness, MeasureCancelled,
    TrackLoudness,
};
use lrc_player_application::infrastructure::audio::engine::{
    AudioEngine, LoudnessHint, Normalization, NormalizationMode,
};
use serde::{Deserialize, Serialize};

use crate::presentation::app::library_index::LibraryIndex;
use crate::presentation::app::GuiApp;

/// Cache file name inside the data directory.
const CACHE_FILE: &str = "loudness.json";
const CACHE_VERSION: u32 = 1;
/// The worker writes the cache after this many new results (and whenever it goes idle).
const SAVE_EVERY: usize = 50;
/// Background results wake the UI at most this often (progress display).
const BACKGROUND_REPAINT: Duration = Duration::from_millis(250);
/// How long dropping the analyzer waits for the worker to stop and save.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(1);

/// `data/loudness.json` next to the executable.
pub(crate) fn default_cache_path() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("data").join(CACHE_FILE))
}

/// Engine normalization settings from the config.
pub(crate) fn normalization_from_config(cfg: &Config) -> Normalization {
    Normalization {
        mode: match cfg.normalization_mode {
            NormalizationModeSetting::Off => NormalizationMode::Off,
            NormalizationModeSetting::Track => NormalizationMode::Track,
            NormalizationModeSetting::Album => NormalizationMode::Album,
        },
        preamp_db: if cfg.normalization_preamp_db.is_finite() { cfg.normalization_preamp_db } else { 0.0 },
        prevent_clipping: cfg.prevent_clipping,
    }
}

/// Request priority (lower value = sooner).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Priority {
    /// The track that is (about to be) playing.
    Current = 0,
    /// The gapless successor.
    Next = 1,
    /// Library analysis and anything else.
    Background = 2,
}

const LEVELS: usize = 3;

/// Size + modification time (ms since the epoch) of a file.
fn file_stamp(path: &Path) -> Option<(u64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Some((meta.len(), mtime))
}

/// One cached file: a measurement, or "has ReplayGain tags", or "could not be decoded".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct CacheEntry {
    size: u64,
    mtime_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lufs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    peak: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    /// The file has ReplayGain tags (the engine uses them; not measured).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    tagged: bool,
    /// Decoding failed (retried when the file changes).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    failed: bool,
}

impl CacheEntry {
    fn stamp(&self) -> (u64, i64) {
        (self.size, self.mtime_ms)
    }

    fn measured(stamp: (u64, i64), l: &TrackLoudness) -> Self {
        Self {
            size: stamp.0,
            mtime_ms: stamp.1,
            lufs: Some(l.integrated_lufs),
            peak: Some(l.true_peak),
            duration_ms: Some(l.duration_ms),
            tagged: false,
            failed: false,
        }
    }

    fn flag(stamp: (u64, i64), tagged: bool) -> Self {
        Self {
            size: stamp.0,
            mtime_ms: stamp.1,
            lufs: None,
            peak: None,
            duration_ms: None,
            tagged,
            failed: !tagged,
        }
    }

    pub(crate) fn loudness(&self) -> Option<TrackLoudness> {
        Some(TrackLoudness {
            integrated_lufs: self.lufs.filter(|l| l.is_finite())?,
            true_peak: self.peak.unwrap_or(0.0),
            duration_ms: self.duration_ms.unwrap_or(0),
        })
    }
}

#[derive(Default, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    entries: HashMap<String, CacheEntry>,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<String, CacheEntry>,
    /// Results not yet written.
    unsaved: usize,
}

impl Cache {
    fn load(path: &Path) -> HashMap<String, CacheEntry> {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<CacheFile>(&b).ok())
            .filter(|f| f.version == CACHE_VERSION)
            .map(|f| f.entries)
            .unwrap_or_default()
    }

    fn valid(&self, path: &str, stamp: (u64, i64)) -> Option<&CacheEntry> {
        self.entries.get(path).filter(|e| e.stamp() == stamp)
    }
}

/// A background job in progress.
struct Running {
    path: String,
    priority: Priority,
    cancel: Arc<AtomicBool>,
    /// Cancelled for a higher-priority request: requeue instead of dropping.
    preempted: bool,
}

#[derive(Default)]
struct Work {
    queues: [VecDeque<String>; LEVELS],
    /// Current priority of each queued path (stale queue entries are skipped).
    queued: HashMap<String, Priority>,
    running: Option<Running>,
    /// Library analysis: paths still to do, and (done, total).
    batch: HashSet<String>,
    progress: Option<(usize, usize)>,
    shutdown: bool,
}

impl Work {
    fn push(&mut self, path: String, priority: Priority) {
        match self.queued.get(&path) {
            Some(&p) if p <= priority => return,
            _ => {}
        }
        if let Some(r) = &self.running {
            if r.path == path {
                return;
            }
        }
        self.queued.insert(path.clone(), priority);
        let q = &mut self.queues[priority as usize];
        if priority == Priority::Background {
            q.push_back(path);
        } else {
            // Newest current/next request first.
            q.push_front(path);
        }
        if priority < Priority::Background {
            if let Some(r) = self.running.as_mut() {
                if r.priority == Priority::Background {
                    r.preempted = true;
                    r.cancel.store(true, Ordering::Relaxed);
                }
            }
        }
    }

    fn pop(&mut self) -> Option<(String, Priority)> {
        for level in 0..LEVELS {
            while let Some(path) = self.queues[level].pop_front() {
                if self.queued.get(&path).is_some_and(|&p| p as usize == level) {
                    self.queued.remove(&path);
                    return Some((path, [Priority::Current, Priority::Next, Priority::Background][level]));
                }
            }
        }
        None
    }

    /// A job for `path` finished (or was dropped): library progress.
    fn finished(&mut self, path: &str) {
        if self.batch.remove(path) {
            if let Some((done, _)) = self.progress.as_mut() {
                *done += 1;
            }
        }
    }
}

struct Shared {
    work: Mutex<Work>,
    cvar: Condvar,
    cache: Mutex<Cache>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// A finished job (the result, if any, is in the shared cache).
struct Done {
    path: String,
}

/// What the UI side handed to the engine.
#[derive(Default)]
struct Hints {
    /// Paths whose cache entry was validated (stat or worker) this session.
    verified: HashSet<String>,
    /// Last hint given to the engine per path (only current / next are kept).
    sent: HashMap<String, Option<LoudnessHint>>,
    current: Option<String>,
    next: Option<String>,
}

/// UI-side handle of the loudness worker. All methods take `&self` (the hint bookkeeping is
/// behind a mutex only the UI thread locks), so playback code can call them while it holds
/// a borrow of the engine.
pub(crate) struct LoudnessAnalyzer {
    shared: Arc<Shared>,
    rx: Receiver<Done>,
    exited: Receiver<()>,
    /// Where the worker keeps the cache (tests check that persist=false has none).
    #[cfg(test)]
    cache_path: Option<PathBuf>,
    hints: Mutex<Hints>,
}

impl LoudnessAnalyzer {
    /// Starts the worker. `cache_path` `None` = in-memory only (nothing read or written).
    pub(crate) fn new(ctx: egui::Context, cache_path: Option<PathBuf>) -> Self {
        let shared = Arc::new(Shared {
            work: Mutex::new(Work::default()),
            cvar: Condvar::new(),
            cache: Mutex::new(Cache::default()),
        });
        let (tx, rx) = crossbeam_channel::unbounded();
        let (exit_tx, exited) = crossbeam_channel::bounded(1);
        let worker_shared = Arc::clone(&shared);
        #[cfg(test)]
        let path = cache_path.clone();
        #[cfg(not(test))]
        let path = cache_path;
        let spawned = std::thread::Builder::new().name("loudness".into()).spawn(move || {
            worker_loop(&worker_shared, path.as_deref(), &tx, &ctx);
            let _ = exit_tx.send(());
        });
        if let Err(err) = spawned {
            tracing::warn!("loudness worker unavailable: {err}");
        }
        Self {
            shared,
            rx,
            exited,
            #[cfg(test)]
            cache_path,
            hints: Mutex::new(Hints::default()),
        }
    }

    #[cfg(test)]
    pub(crate) fn cache_path(&self) -> Option<&Path> {
        self.cache_path.as_deref()
    }

    fn request(&self, path: &str, priority: Priority) {
        let mut work = lock(&self.shared.work);
        work.push(path.to_string(), priority);
        self.shared.cvar.notify_one();
    }

    /// The hint for `path` from the cache, with album gain when its album is complete.
    /// The track's own entry must have been validated this session (`verified`).
    fn hint_for(&self, hints: &Hints, path: &str, library: &LibraryIndex) -> Option<LoudnessHint> {
        if !hints.verified.contains(path) {
            return None;
        }
        let track = lock(&self.shared.cache).entries.get(path).and_then(CacheEntry::loudness)?;
        let album = library.album_key_of(Path::new(path)).and_then(|key| {
            let paths = library.paths_of(library.album_rows(&key));
            let cache = lock(&self.shared.cache);
            let measured: Option<Vec<TrackLoudness>> =
                paths.iter().map(|p| cache.entries.get(p).and_then(CacheEntry::loudness)).collect();
            measured.filter(|m| !m.is_empty()).map(|m| album_gain(&m))
        });
        Some(LoudnessHint {
            track_gain_db: track.gain_db(),
            track_peak: track.true_peak,
            album_gain_db: album.map(|a| a.0),
            album_peak: album.map(|a| a.1),
        })
    }

    /// Sends the hint for `path` to the engine when it changed.
    fn send_hint(hints: &mut Hints, engine: Option<&AudioEngine>, path: &str, hint: Option<LoudnessHint>) {
        let prev = hints.sent.get(path).copied().flatten();
        if prev == hint {
            return;
        }
        if let Some(engine) = engine {
            engine.set_loudness_hint(PathBuf::from(path), hint);
        }
        hints.sent.insert(path.to_string(), hint);
    }

    /// The paths last prepared as current / next.
    pub(crate) fn current_and_next(&self) -> (Option<String>, Option<String>) {
        let hints = lock(&self.hints);
        (hints.current.clone(), hints.next.clone())
    }

    /// Forgets which tracks were prepared (the next poll prepares them again).
    pub(crate) fn reset_current_and_next(&self) {
        let mut hints = lock(&self.hints);
        hints.current = None;
        hints.next = None;
    }

    /// Sends the cached hint for `path` (validated by size + mtime) or asks the worker.
    pub(crate) fn prepare(&self, engine: Option<&AudioEngine>, library: &LibraryIndex, path: &str, priority: Priority) {
        if path.trim().is_empty() {
            return;
        }
        let mut guard = lock(&self.hints);
        let hints = &mut *guard;
        match priority {
            Priority::Current => hints.current = Some(path.to_string()),
            Priority::Next => hints.next = Some(path.to_string()),
            Priority::Background => {}
        }
        // Keep `sent` to the current / next tracks.
        let (cur, next) = (hints.current.clone(), hints.next.clone());
        hints.sent.retain(|p, _| Some(p) == cur.as_ref() || Some(p) == next.as_ref());

        let stamp = file_stamp(Path::new(path));
        let cached = stamp.and_then(|s| lock(&self.shared.cache).valid(path, s).cloned());
        match cached {
            Some(entry) => {
                hints.verified.insert(path.to_string());
                let hint = if entry.loudness().is_some() {
                    self.hint_for(hints, path, library)
                } else {
                    // Tagged (the engine reads the tags) or undecodable.
                    None
                };
                Self::send_hint(hints, engine, path, hint);
            }
            None => {
                // Unknown or changed file: forget a stale hint, measure.
                hints.verified.remove(path);
                Self::send_hint(hints, engine, path, None);
                if stamp.is_some() {
                    self.request(path, priority);
                }
            }
        }
    }

    /// Applies finished jobs; re-sends the current / next hints (album completion).
    /// Returns true when something finished.
    pub(crate) fn poll(&self, engine: Option<&AudioEngine>, library: &LibraryIndex) -> bool {
        let mut guard = lock(&self.hints);
        let hints = &mut *guard;
        let mut any = false;
        while let Ok(done) = self.rx.try_recv() {
            // The worker validated this entry. Only current / next need the mark (any other
            // path is validated by its own `prepare`), which keeps the set small during a
            // library analysis.
            if Some(&done.path) == hints.current.as_ref() || Some(&done.path) == hints.next.as_ref() {
                hints.verified.insert(done.path);
            }
            any = true;
        }
        if any {
            for path in [hints.current.clone(), hints.next.clone()].into_iter().flatten() {
                if hints.verified.contains(&path) {
                    let hint = self.hint_for(hints, &path, library);
                    Self::send_hint(hints, engine, &path, hint);
                }
            }
        }
        any
    }

    /// Queues every path at background priority (library analysis).
    pub(crate) fn analyze(&self, paths: Vec<String>) {
        let mut work = lock(&self.shared.work);
        let mut added = 0;
        for path in paths {
            if work.batch.insert(path.clone()) {
                added += 1;
                work.push(path, Priority::Background);
            }
        }
        let (done, total) = work.progress.unwrap_or((0, 0));
        work.progress = Some((done, total + added));
        if work.batch.is_empty() {
            work.progress = None;
        }
        self.shared.cvar.notify_one();
    }

    /// Drops queued background work and stops a running background measurement.
    pub(crate) fn cancel_background(&self) {
        let mut guard = lock(&self.shared.work);
        let work = &mut *guard;
        for path in std::mem::take(&mut work.queues[Priority::Background as usize]) {
            // Entries promoted to current / next live on in their own queue.
            if work.queued.get(&path) == Some(&Priority::Background) {
                work.queued.remove(&path);
            }
        }
        if let Some(r) = work.running.as_mut() {
            if r.priority == Priority::Background {
                r.preempted = false;
                r.cancel.store(true, Ordering::Relaxed);
            }
        }
        work.batch.clear();
        work.progress = None;
    }

    /// `(done, total)` of the running library analysis.
    pub(crate) fn progress(&self) -> Option<(usize, usize)> {
        lock(&self.shared.work).progress
    }

    #[cfg(test)]
    pub(crate) fn sent_hint(&self, path: &str) -> Option<LoudnessHint> {
        lock(&self.hints).sent.get(path).copied().flatten()
    }

    #[cfg(test)]
    fn clear_sent(&self) {
        lock(&self.hints).sent.clear();
    }

    #[cfg(test)]
    pub(crate) fn cached(&self, path: &str) -> Option<CacheEntry> {
        lock(&self.shared.cache).entries.get(path).cloned()
    }
}

impl Drop for LoudnessAnalyzer {
    fn drop(&mut self) {
        {
            let mut work = lock(&self.shared.work);
            work.shutdown = true;
            if let Some(r) = work.running.as_mut() {
                r.preempted = false;
                r.cancel.store(true, Ordering::Relaxed);
            }
            self.shared.cvar.notify_one();
        }
        // Cancellation is checked per decoded packet, so the worker stops (and saves the
        // cache) quickly; never hang the exit on a stalled file system, though.
        let _ = self.exited.recv_timeout(SHUTDOWN_WAIT);
    }
}

fn save_cache(shared: &Shared, path: &Path) {
    let json = {
        let mut cache = lock(&shared.cache);
        if cache.unsaved == 0 {
            return;
        }
        cache.unsaved = 0;
        let file = CacheFile { version: CACHE_VERSION, entries: cache.entries.clone() };
        match serde_json::to_vec(&file) {
            Ok(j) => j,
            Err(err) => {
                tracing::warn!("serializing the loudness cache failed: {err}");
                return;
            }
        }
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    let res = std::fs::write(&tmp, json).and_then(|_| std::fs::rename(&tmp, path));
    if let Err(err) = res {
        tracing::warn!("writing {} failed: {err}", path.display());
    }
}

/// Measures (or classifies) one file. `None` when it was cancelled or the file is missing.
fn process(path: &str, cancel: &AtomicBool) -> Result<Option<CacheEntry>, MeasureCancelled> {
    let p = Path::new(path);
    let Some(stamp) = file_stamp(p) else { return Ok(None) };
    if has_replaygain_tags(p) {
        return Ok(Some(CacheEntry::flag(stamp, true)));
    }
    match measure_loudness(p, cancel) {
        Ok(l) => Ok(Some(CacheEntry::measured(stamp, &l))),
        Err(err) if err.is::<MeasureCancelled>() => Err(MeasureCancelled),
        Err(err) => {
            tracing::debug!("loudness of {path}: {err:#}");
            Ok(Some(CacheEntry::flag(stamp, false)))
        }
    }
}

fn worker_loop(shared: &Shared, cache_path: Option<&Path>, tx: &Sender<Done>, ctx: &egui::Context) {
    lower_current_thread_priority();
    if let Some(path) = cache_path {
        let entries = Cache::load(path);
        let mut cache = lock(&shared.cache);
        // Results that raced the load are newer.
        for (k, v) in entries {
            cache.entries.entry(k).or_insert(v);
        }
    }
    let mut last_repaint: Option<Instant> = None;
    loop {
        let (path, priority, cancel) = {
            let mut work = lock(&shared.work);
            loop {
                if work.shutdown {
                    drop(work);
                    if let Some(p) = cache_path {
                        save_cache(shared, p);
                    }
                    return;
                }
                if let Some((path, priority)) = work.pop() {
                    let cancel = Arc::new(AtomicBool::new(false));
                    work.running = Some(Running {
                        path: path.clone(),
                        priority,
                        cancel: Arc::clone(&cancel),
                        preempted: false,
                    });
                    break (path, priority, cancel);
                }
                // Idle: a good moment to write the cache.
                if cache_path.is_some() && lock(&shared.cache).unsaved > 0 {
                    drop(work);
                    if let Some(p) = cache_path {
                        save_cache(shared, p);
                    }
                    work = lock(&shared.work);
                    continue;
                }
                work = match shared.cvar.wait(work) {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
            }
        };

        // A still-valid cached result needs no work.
        let stamp = file_stamp(Path::new(&path));
        let cached = stamp.is_some_and(|s| lock(&shared.cache).valid(&path, s).is_some());
        let outcome = if cached { Ok(None) } else { process(&path, &cancel) };

        let mut work = lock(&shared.work);
        let running = work.running.take();
        match outcome {
            Err(MeasureCancelled) => {
                if running.as_ref().is_some_and(|r| r.preempted) && !work.shutdown {
                    // Resume later, ahead of the other background work.
                    if !work.queued.contains_key(&path) {
                        work.queued.insert(path.clone(), priority);
                        work.queues[priority as usize].push_front(path);
                    }
                } else {
                    work.finished(&path);
                }
                continue;
            }
            Ok(entry) => {
                work.finished(&path);
                if work.batch.is_empty() {
                    work.progress = None;
                }
                drop(work);
                if let Some(entry) = entry {
                    let mut cache = lock(&shared.cache);
                    cache.entries.insert(path.clone(), entry);
                    cache.unsaved += 1;
                    let unsaved = cache.unsaved;
                    drop(cache);
                    if unsaved >= SAVE_EVERY {
                        if let Some(p) = cache_path {
                            save_cache(shared, p);
                        }
                    }
                }
            }
        }
        if tx.send(Done { path }).is_err() {
            return;
        }
        let urgent = priority < Priority::Background;
        if urgent || last_repaint.is_none_or(|t| t.elapsed() >= BACKGROUND_REPAINT) {
            ctx.request_repaint();
            last_repaint = Some(Instant::now());
        } else {
            ctx.request_repaint_after(BACKGROUND_REPAINT);
        }
    }
}

impl GuiApp {
    fn normalization_active(&self) -> bool {
        self.config.normalization_mode != NormalizationModeSetting::Off
    }

    /// Hook for `playback.rs`: before `path` is loaded (`Current`) or queued gaplessly (`Next`),
    /// sends its cached hint to the engine or asks the worker to measure it. Cheap (one stat).
    pub(crate) fn prepare_loudness(&self, path: &str, priority: Priority) {
        if !self.normalization_active() {
            return;
        }
        self.loudness.prepare(self.engine.as_ref(), &self.library, path, priority);
    }

    /// Applies worker results and makes sure the current / next tracks were requested
    /// (covers the restored track and settings changes).
    pub(crate) fn poll_loudness(&mut self) {
        if self.normalization_active() {
            let (current, next) = self.loudness.current_and_next();
            if !self.play_path.is_empty() && current.as_deref() != Some(self.play_path.as_str()) {
                self.prepare_loudness(&self.play_path, Priority::Current);
            }
            if let Some((_, pending)) = &self.pending_next {
                if next.as_deref() != Some(pending.as_str()) {
                    self.prepare_loudness(pending, Priority::Next);
                }
            }
        }
        self.loudness.poll(self.engine.as_ref(), &self.library);
    }

    /// Current normalization settings.
    pub fn normalization(&self) -> Normalization {
        normalization_from_config(&self.config)
    }

    /// Changes the normalization settings (applied with a ramp, saved in `config.json`).
    /// The preamp is clamped to ±15 dB.
    pub fn set_normalization(&mut self, n: Normalization) {
        let mode = match n.mode {
            NormalizationMode::Off => NormalizationModeSetting::Off,
            NormalizationMode::Track => NormalizationModeSetting::Track,
            NormalizationMode::Album => NormalizationModeSetting::Album,
        };
        let preamp = if n.preamp_db.is_finite() { n.preamp_db.clamp(-15.0, 15.0) } else { 0.0 };
        let was_active = self.normalization_active();
        self.config.normalization_mode = mode;
        self.config.normalization_preamp_db = preamp;
        self.config.prevent_clipping = n.prevent_clipping;
        if let Some(engine) = &self.engine {
            engine.set_normalization(self.normalization());
        }
        if !was_active && self.normalization_active() {
            // Measure what is playing now (the next poll re-requests current / next).
            self.loudness.reset_current_and_next();
        }
        self.save_config();
    }

    /// Queues every library track for loudness analysis (background priority; tagged and
    /// already measured files are skipped quickly). Progress: [`Self::loudness_analysis_progress`].
    pub fn analyze_library_loudness(&mut self) {
        let paths: Vec<String> =
            self.library.rows.iter().map(|r| r.path.to_string_lossy().into_owned()).collect();
        self.loudness.analyze(paths);
    }

    /// Stops the library analysis (current / next track requests are kept).
    pub fn cancel_loudness_analysis(&mut self) {
        self.loudness.cancel_background();
    }

    /// `Some((done, total))` while a library analysis runs, `None` otherwise.
    pub fn loudness_analysis_progress(&self) -> Option<(usize, usize)> {
        self.loudness.progress()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::app::tests::test_app;
    use lrc_player_application::infrastructure::audio::engine::EngineState;
    use lrc_player_application::infrastructure::persistence::MetaCache;

    /// Stereo 16-bit WAV: 1 kHz sine at `dbfs` on both channels (EBU Tech 3341: -23 dBFS
    /// stereo = -23 LUFS, so the expected track gain is `-18 - dbfs`).
    fn write_sine(path: &Path, dbfs: f64, seconds: f64) {
        let rate = 44_100u32;
        let frames = (rate as f64 * seconds) as usize;
        let amp = 10f64.powf(dbfs / 20.0) * 32767.0;
        let mut data = Vec::with_capacity(frames * 4);
        for i in 0..frames {
            let v = (amp * (std::f64::consts::TAU * 1000.0 * i as f64 / rate as f64).sin()).round() as i16;
            data.extend_from_slice(&v.to_le_bytes());
            data.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 4).to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        std::fs::write(path, out).unwrap();
    }

    fn run_frame(ctx: &egui::Context, app: &mut GuiApp) {
        let _ = ctx.run_ui(egui::RawInput::default(), |_ui| {});
        app.poll_engine();
        app.poll_library();
        app.poll_loudness();
    }

    fn wait_for(ctx: &egui::Context, app: &mut GuiApp, what: &str, mut cond: impl FnMut(&GuiApp) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            run_frame(ctx, app);
            if cond(app) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what} (pending_next {:?}, prepared {:?}, state {:?}, snapshot {:?})",
                app.pending_next,
                app.loudness.current_and_next(),
                app.playback_state,
                app.engine_snapshot.as_ref().map(|s| (s.state, s.path.clone()))
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    fn add_library_track(db: &MetaCache, i: usize, path: &Path, album: &str) {
        let conn = db.connection();
        conn.execute(
            "INSERT INTO tracks (content_id, title, artist, album, album_artist) VALUES (?1, ?2, 'Ar', ?3, 'Ar')",
            rusqlite::params![format!("c{i}"), format!("T{i}"), album],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO locations (content_id, path, last_seen, available) VALUES (?1, ?2, 0, 1)",
            rusqlite::params![format!("c{i}"), s(path)],
        )
        .unwrap();
    }

    #[test]
    fn default_is_track_mode_with_clipping_guard() {
        let n = normalization_from_config(&Config::default());
        assert_eq!(n.mode, NormalizationMode::Track);
        assert_eq!(n.preamp_db, 0.0);
        assert!(n.prevent_clipping);
    }

    #[test]
    fn hints_for_current_and_next_cache_invalidation_and_album_gain() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (dir.path().join("a.wav"), dir.path().join("b.wav"), dir.path().join("c.wav"));
        write_sine(&a, -23.0, 3.0); // -23 LUFS -> +5 dB
        write_sine(&b, -13.0, 3.0); // -13 LUFS -> -5 dB
        write_sine(&c, -18.0, 3.0); // -18 LUFS -> 0 dB
        {
            // a and b form one album in the library; c is on another one.
            let db = MetaCache::new(dir.path().join("meta_cache.db")).unwrap();
            add_library_track(&db, 0, &a, "Album");
            add_library_track(&db, 1, &b, "Album");
            add_library_track(&db, 2, &c, "Other");
        }
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx, dir.path(), true);
        assert!(app.loudness.cache_path().is_none(), "persist=false keeps the cache in memory");
        wait_for(&ctx, &mut app, "library", |app| app.library.loaded && app.library.rows.len() == 3);

        // Play a with b queued next: both get measured and their hints sent. Without an output
        // device the engine may never report `TrackStarted` (which queues the successor), so
        // the gapless hook is driven directly here.
        app.set_shuffle(false);
        app.set_repeat(lrc_player_application::dto::RepeatMode::Off);
        app.play_now(vec![s(&a), s(&b)]);
        wait_for(&ctx, &mut app, "engine accepted the load", |app| {
            app.engine_snapshot.as_ref().is_some_and(|s| s.state != EngineState::Stopped)
        });
        app.update_gapless_next();
        assert_eq!(app.pending_next, Some((1, s(&b))));
        wait_for(&ctx, &mut app, "hints for current and next", |app| {
            app.pending_next.as_ref().is_some_and(|(_, p)| *p == s(&b))
                && app.loudness.sent_hint(&s(&a)).is_some()
                && app.loudness.sent_hint(&s(&b)).is_some()
        });
        let ha = app.loudness.sent_hint(&s(&a)).unwrap();
        let hb = app.loudness.sent_hint(&s(&b)).unwrap();
        assert!((ha.track_gain_db - 5.0).abs() < 0.15, "{ha:?}");
        assert!((hb.track_gain_db - -5.0).abs() < 0.15, "{hb:?}");
        assert!((ha.track_peak - 10f32.powf(-23.0 / 20.0)).abs() < 0.01, "{ha:?}");

        // Album gain once both album tracks are measured: equal durations, energy mean of
        // -23 and -13 LUFS = -13 + 10*log10((1 + 0.1)/2) ≈ -15.6 LUFS -> about -2.4 dB.
        wait_for(&ctx, &mut app, "album gain", |app| {
            app.loudness.sent_hint(&s(&a)).is_some_and(|h| h.album_gain_db.is_some())
        });
        let ha = app.loudness.sent_hint(&s(&a)).unwrap();
        let expected = -18.0 - (-13.0 + 10.0 * ((1.0f32 + 0.1) / 2.0).log10());
        assert!((ha.album_gain_db.unwrap() - expected).abs() < 0.15, "{ha:?} vs {expected}");
        assert_eq!(ha.album_peak, Some(hb.track_peak.max(ha.track_peak)));
        assert_eq!(app.loudness.sent_hint(&s(&b)).unwrap().album_gain_db, ha.album_gain_db);

        // A cached hint is sent synchronously before the engine loads the file.
        app.loudness.clear_sent();
        app.play_now(vec![s(&a)]);
        assert_eq!(app.loudness.sent_hint(&s(&a)).map(|h| h.track_gain_db), Some(ha.track_gain_db));

        // Cache invalidation: a changed file (new mtime) is not served from the cache.
        write_sine(&a, -33.0, 3.0); // same size, 10 dB quieter
        let f = std::fs::File::options().write(true).open(&a).unwrap();
        f.set_modified(std::time::SystemTime::now() + Duration::from_secs(5)).unwrap();
        drop(f);
        app.play_now(vec![s(&a)]);
        assert_eq!(app.loudness.sent_hint(&s(&a)), None, "stale hint must not be reused");
        wait_for(&ctx, &mut app, "re-measured hint", |app| app.loudness.sent_hint(&s(&a)).is_some());
        let ha2 = app.loudness.sent_hint(&s(&a)).unwrap();
        assert!((ha2.track_gain_db - 15.0).abs() < 0.15, "{ha2:?}");

        // Library analysis measures the rest (c) and reports progress.
        app.analyze_library_loudness();
        assert!(app.loudness_analysis_progress().is_some());
        wait_for(&ctx, &mut app, "library analysis", |app| app.loudness_analysis_progress().is_none());
        let cc = app.loudness.cached(&s(&c)).and_then(|e| e.loudness()).expect("c measured");
        assert!(cc.gain_db().abs() < 0.15, "{cc:?}");
    }

    #[test]
    fn library_analysis_reports_progress_and_can_be_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..30 {
            let p = dir.path().join(format!("t{i}.wav"));
            write_sine(&p, -20.0, 20.0);
            paths.push(s(&p));
        }
        let ctx = egui::Context::default();
        let analyzer = LoudnessAnalyzer::new(ctx, None);
        analyzer.analyze(paths.clone());
        let (_, total) = analyzer.progress().unwrap();
        assert_eq!(total, 30);
        analyzer.cancel_background();
        assert_eq!(analyzer.progress(), None);
        // Nothing left to do: the worker goes idle well before measuring all 30 files.
        std::thread::sleep(Duration::from_millis(300));
        let measured = paths.iter().filter(|p| analyzer.cached(p).is_some()).count();
        assert!(measured < 30, "cancel should stop the batch ({measured} measured)");
    }

    #[test]
    fn cache_round_trips_through_the_file_and_persist_false_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let track = dir.path().join("x.wav");
        write_sine(&track, -23.0, 2.0);
        let cache_file = dir.path().join("data").join("loudness.json");
        let ctx = egui::Context::default();
        {
            let analyzer = LoudnessAnalyzer::new(ctx.clone(), Some(cache_file.clone()));
            let library = LibraryIndex::new(ctx.clone(), false);
            analyzer.prepare(None, &library, &s(&track), Priority::Current);
            let deadline = Instant::now() + Duration::from_secs(20);
            while analyzer.sent_hint(&s(&track)).is_none() {
                analyzer.poll(None, &library);
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(10));
            }
        } // drop: the worker saves
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&cache_file).unwrap()).unwrap();
        let entry = &json["entries"][s(&track)];
        assert!(entry["size"].as_u64().is_some() && entry["mtime_ms"].as_i64().is_some(), "{json}");
        assert!((entry["lufs"].as_f64().unwrap() - -23.0).abs() < 0.15);

        // Loaded on start: the hint is served from the file without re-measuring.
        let analyzer = LoudnessAnalyzer::new(ctx.clone(), Some(cache_file.clone()));
        let library = LibraryIndex::new(ctx.clone(), false);
        let deadline = Instant::now() + Duration::from_secs(5);
        while analyzer.cached(&s(&track)).is_none() {
            assert!(Instant::now() < deadline, "cache not loaded");
            std::thread::sleep(Duration::from_millis(5));
        }
        analyzer.prepare(None, &library, &s(&track), Priority::Current);
        assert!(analyzer.sent_hint(&s(&track)).is_some(), "served from the loaded cache");
        drop(analyzer);

        // persist=false: no cache path, and the real data file is not touched.
        let real = default_cache_path().unwrap();
        let before = std::fs::metadata(&real).ok().and_then(|m| m.modified().ok());
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx, dir.path(), false);
        assert!(app.loudness.cache_path().is_none());
        app.prepare_loudness(&s(&track), Priority::Current);
        let deadline = Instant::now() + Duration::from_secs(20);
        while app.loudness.sent_hint(&s(&track)).is_none() {
            app.poll_loudness();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(app);
        let after = std::fs::metadata(&real).ok().and_then(|m| m.modified().ok());
        assert_eq!(before, after, "persist=false must not write {}", real.display());
    }
}
