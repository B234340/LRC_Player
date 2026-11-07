use lrc_player_domain::services::compute_partial_hash;
use lrc_player_adapters_lyrics::infrastructure::lyrics::Lyrics;
use lrc_player_adapters_persistence::persistence::MetaCache;
use crate::infrastructure::scan::file_scanner::FileScanner;
use anyhow::Result;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::default::get_probe;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

/// Progress updates are sent at most this often (plus once at the end).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

type AudioTagMeta = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub fast: bool,
    pub excludes: Vec<std::path::PathBuf>,
}

#[derive(Clone, Debug)]
pub struct ScanHandle {
    pub id: u64,
}

/// Cumulative counts for one scan job. The progress channel closes when the job has
/// finished (or was cancelled) and everything it found has been committed to the database.
#[derive(Clone, Debug)]
pub struct Progress {
    /// Supported files (audio + lyrics) found so far.
    pub scanned: u64,
    /// Files read successfully (hashed / parsed).
    pub parsed: u64,
    /// Audio tracks and lyrics files written to the database.
    pub registered: u64,
}

enum WorkerCommand {
    Start {
        id: u64,
        paths: Vec<PathBuf>,
        options: ScanOptions,
    },
    Cancel {
        id: u64,
    },
}

enum DBCommand {
    UpsertFile {
        path: String,
        norm_path: String,
        size: i64,
        mtime: i64,
        device: Option<i64>,
        inode: Option<i64>,
        stable_key: Option<String>,
    },
    UpsertTrack {
        content_id: String,
        title: Option<String>,
        artist: Option<String>,
        album_artist: Option<String>,
        composer: Option<String>,
        album: Option<String>,
        canonical_location_id: Option<i64>,
        created_at: i64,
    },
    UpsertHash {
        content_id: String,
        algo: String,
        full_hash: Option<String>,
        partial_hash: Option<String>,
        computed_at: i64,
    },
    UpsertLocation {
        content_id: String,
        path: String,
        device: Option<i64>,
        inode: Option<i64>,
        size: i64,
        mtime: i64,
        last_seen: i64,
        preferred: bool,
        available: bool,
    },
    UpsertLyricsLocation {
        content_id: Option<String>,
        path: String,
        size: i64,
        mtime: i64,
        last_seen: i64,
    },
    Checkpoint,
    /// Commit everything staged so far, then acknowledge.
    Flush(oneshot::Sender<()>),
}

/// Handle to control the IO worker. Cloneable.
#[derive(Clone)]
pub struct IoWorkerHandle {
    cmd_tx: mpsc::Sender<WorkerCommand>,
    progress_senders: ProgressSenders,
    db_tx: mpsc::Sender<DBCommand>,
    _inner: Arc<Mutex<()>>, // placeholder for possible shared state
}

fn extract_audio_tags(
    path: &PathBuf,
) -> AudioTagMeta {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return (None, None, None, None, None),
    };

    // Symphonia probe search has a hard 1 MiB limit. Large ID3v2 headers can exceed it,
    // causing noisy "reached probe limit" logs for valid files. Skip probing in that case.
    let mut header = [0_u8; 10];
    if file.read_exact(&mut header).is_ok() && &header[0..3] == b"ID3" {
        let id3_size = ((header[6] as u32 & 0x7F) << 21)
            | ((header[7] as u32 & 0x7F) << 14)
            | ((header[8] as u32 & 0x7F) << 7)
            | (header[9] as u32 & 0x7F);
        let total_id3_bytes = 10_u32.saturating_add(id3_size);
        if total_id3_bytes > 1_048_576 {
            return (None, None, None, None, None);
        }
    }

    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return (None, None, None, None, None),
    };

    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut probe = match get_probe().format(
        &Default::default(),
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) {
        Ok(p) => p,
        Err(_) => return (None, None, None, None, None),
    };

    let mut title = None;
    let mut artist = None;
    let mut album_artist = None;
    let mut composer = None;
    let mut album = None;

    if let Some(metadata) = probe.format.metadata().current() {
        for tag in metadata.tags() {
            let key = tag.key.to_string().to_lowercase();
            match key.as_str() {
                "title" | "tracktitle" | "track title" => {
                    title.get_or_insert_with(|| tag.value.to_string());
                }
                "artist" | "performer" => {
                    artist.get_or_insert_with(|| tag.value.to_string());
                }
                "album artist" => {
                    album_artist.get_or_insert_with(|| tag.value.to_string());
                }
                "albumartist" => {
                    album_artist.get_or_insert_with(|| tag.value.to_string());
                }
                "composer" => {
                    composer.get_or_insert_with(|| tag.value.to_string());
                }
                "album" | "album title" => {
                    album.get_or_insert_with(|| tag.value.to_string());
                }
                _ => {}
            }
        }
    }

    if album_artist.is_none() {
        album_artist = artist.clone();
    }

    (title, artist, album_artist, composer, album)
}

impl IoWorkerHandle {
    pub async fn start_scan(
        &self,
        paths: Vec<PathBuf>,
        options: ScanOptions,
    ) -> Result<ScanHandle> {
        let id = rand::random::<u64>();
        let (progress_tx, _progress_rx) = broadcast::channel(32);
        lock(&self.progress_senders).insert(id, progress_tx);

        let cmd = WorkerCommand::Start { id, paths, options };
        self.cmd_tx
            .send(cmd)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(ScanHandle { id })
    }

    pub async fn cancel(&self, handle: &ScanHandle) -> Result<()> {
        let cmd = WorkerCommand::Cancel { id: handle.id };
        self.cmd_tx
            .send(cmd)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(())
    }

    pub fn subscribe(&self, handle: &ScanHandle) -> broadcast::Receiver<Progress> {
        if let Some(sender) = lock(&self.progress_senders).get(&handle.id) {
            sender.subscribe()
        } else {
            // The job already finished: hand out a closed channel holding one empty update,
            // so the subscriber sees the end of the scan right away.
            let (tx, rx) = broadcast::channel(1);
            let _ = tx.send(Progress {
                scanned: 0,
                parsed: 0,
                registered: 0,
            });
            rx
        }
    }

    pub async fn checkpoint_now(&self) -> Result<()> {
        let cmd = DBCommand::Checkpoint;
        self.db_tx
            .send(cmd)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(())
    }
}

pub fn spawn_io_worker(db_path: Option<std::path::PathBuf>) -> IoWorkerHandle {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<WorkerCommand>(64);

    // DB command channel
    let (db_tx, mut db_rx) = mpsc::channel::<DBCommand>(64);

    // Progress sender map
    let progress_senders: ProgressSenders = Arc::new(StdMutex::new(HashMap::new()));

    // Active job cancellation senders
    let active_jobs: ActiveJobs = Arc::new(StdMutex::new(HashMap::new()));
    let job_db_path = db_path.clone();

    // Spawn DB worker
    tokio::spawn(async move {
        // Open MetaCache (in-memory if None)
        let meta = match db_path {
            Some(p) => MetaCache::new(p).ok(),
            None => MetaCache::new(":memory:").ok(),
        };

        // checkpoint policy
        let mut staged_count: usize = 0;
        const CHECKPOINT_N: usize = 500;
        const CHECKPOINT_SEC: u64 = 5;

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(CHECKPOINT_SEC));

        loop {
            tokio::select! {
                biased;
                maybe_cmd = db_rx.recv() => {
                    match maybe_cmd {
                        Some(db_cmd) => {
                            let db_cmd = match (db_cmd, meta.as_ref()) {
                                (DBCommand::Flush(ack), None) => {
                                    let _ = ack.send(());
                                    continue;
                                }
                                (cmd, _) => cmd,
                            };
                            if let Some(m) = meta.as_ref() {
                                match db_cmd {
                                    DBCommand::UpsertFile { path, norm_path, size, mtime, device, inode, stable_key } => {
                                        let _ = m.upsert_file_staging(&path, &norm_path, size, mtime, device, inode, stable_key.as_deref());
                                        staged_count += 1;
                                    }
                                    DBCommand::UpsertTrack {
                                        content_id,
                                        title,
                                        artist,
                                        album_artist,
                                        composer,
                                        album,
                                        canonical_location_id,
                                        created_at,
                                    } => {
                                        let _ = m.upsert_track_staging(
                                            &content_id,
                                            title.as_deref(),
                                            artist.as_deref(),
                                            album_artist.as_deref(),
                                            composer.as_deref(),
                                            album.as_deref(),
                                            canonical_location_id,
                                            created_at,
                                        );
                                        staged_count += 1;
                                    }
                                    DBCommand::UpsertHash { content_id, algo, full_hash, partial_hash, computed_at } => {
                                        let _ = m.upsert_hash_staging(&content_id, &algo, full_hash.as_deref(), partial_hash.as_deref(), computed_at);
                                        staged_count += 1;
                                    }
                                    DBCommand::UpsertLocation { content_id, path, device, inode, size, mtime, last_seen, preferred, available } => {
                                        let _ = m.upsert_location_staging(
                                            &content_id,
                                            &path,
                                            device,
                                            inode,
                                            size,
                                            mtime,
                                            last_seen,
                                            preferred,
                                            available,
                                        );
                                        staged_count += 1;
                                    }
                                    DBCommand::UpsertLyricsLocation { content_id, path, size, mtime, last_seen } => {
                                        let _ = m.upsert_lyrics_location(
                                            content_id.as_deref(),
                                            &path,
                                            size,
                                            mtime,
                                            last_seen,
                                        );
                                        staged_count += 1;
                                    }
                                    DBCommand::Checkpoint => {
                                        let _ = m.checkpoint();
                                        staged_count = 0;
                                    }
                                    DBCommand::Flush(ack) => {
                                        if let Err(err) = m.checkpoint() {
                                            log_warn(&format!("scan checkpoint failed: {err:#}"));
                                        }
                                        staged_count = 0;
                                        let _ = ack.send(());
                                    }
                                }
                                if staged_count >= CHECKPOINT_N {
                                    let _ = m.checkpoint();
                                    staged_count = 0;
                                }
                            }
                        }
                        None => break, // channel closed
                    }
                }
                _ = interval.tick() => {
                    if staged_count > 0 {
                        if let Some(m) = meta.as_ref() {
                            let _ = m.checkpoint();
                            staged_count = 0;
                        }
                    }
                }
            }
        }
    });

    let jobs = active_jobs.clone();
    let progress_senders_clone = progress_senders.clone();
    let db_tx_clone = db_tx.clone();
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                WorkerCommand::Start { id, paths, options } => {
                    let (cancel_tx, cancel_rx) = oneshot::channel();
                    lock(&jobs).insert(id, cancel_tx);
                    let sender = lock(&progress_senders_clone)
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| broadcast::channel(1).0);
                    let job = ScanJob {
                        id,
                        paths,
                        excludes: options.excludes,
                        cancel_rx,
                        cancelled: false,
                        sender,
                        db_tx: db_tx_clone.clone(),
                        db_path: job_db_path.clone(),
                        progress_senders: progress_senders_clone.clone(),
                        jobs: jobs.clone(),
                    };
                    // The job is blocking file I/O from start to end; keep it off the runtime.
                    tokio::task::spawn_blocking(move || job.run());
                }
                WorkerCommand::Cancel { id } => {
                    if let Some(tx) = lock(&jobs).remove(&id) {
                        let _ = tx.send(());
                    }
                }
            }
        }
    });

    IoWorkerHandle {
        cmd_tx,
        progress_senders,
        db_tx,
        _inner: Arc::new(Mutex::new(())),
    }
}



type ProgressSenders = Arc<StdMutex<HashMap<u64, broadcast::Sender<Progress>>>>;
type ActiveJobs = Arc<StdMutex<HashMap<u64, oneshot::Sender<()>>>>;

fn lock<T>(m: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn log_warn(msg: &str) {
    eprintln!("[scan] {msg}");
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn size_and_mtime(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some((meta.len() as i64, mtime))
}

/// One scan job, run on a blocking thread.
struct ScanJob {
    id: u64,
    paths: Vec<PathBuf>,
    excludes: Vec<PathBuf>,
    cancel_rx: oneshot::Receiver<()>,
    cancelled: bool,
    sender: broadcast::Sender<Progress>,
    db_tx: mpsc::Sender<DBCommand>,
    db_path: Option<PathBuf>,
    progress_senders: ProgressSenders,
    jobs: ActiveJobs,
}

impl ScanJob {
    fn is_cancelled(&mut self) -> bool {
        if !self.cancelled && matches!(self.cancel_rx.try_recv(), Ok(())) {
            self.cancelled = true;
        }
        self.cancelled
    }

    fn db(&self, cmd: DBCommand) {
        let _ = self.db_tx.blocking_send(cmd);
    }

    /// Commits everything staged so far and waits until the DB worker has done it.
    fn flush(&self) {
        let (ack_tx, ack_rx) = oneshot::channel();
        if self.db_tx.blocking_send(DBCommand::Flush(ack_tx)).is_ok() {
            let _ = ack_rx.blocking_recv();
        }
    }

    fn run(mut self) {
        let scanner = FileScanner::new();
        let mut counts = Progress { scanned: 0, parsed: 0, registered: 0 };
        let mut last_emit: Option<Instant> = None;
        let emit = |counts: &Progress, last: &mut Option<Instant>, sender: &broadcast::Sender<Progress>, force: bool| {
            if force || last.is_none_or(|t| t.elapsed() >= PROGRESS_INTERVAL) {
                let _ = sender.send(counts.clone());
                *last = Some(Instant::now());
            }
        };

        // 1. Find every supported file under every root.
        let mut audio = Vec::new();
        let mut lyrics = Vec::new();
        for root in self.paths.clone() {
            if self.is_cancelled() {
                break;
            }
            for file in scanner.enumerate_paths(&root, &self.excludes) {
                if scanner.is_audio_file(&file) {
                    audio.push(file);
                } else if scanner.is_lyrics_file(&file) {
                    lyrics.push(file);
                }
            }
            counts.scanned = (audio.len() + lyrics.len()) as u64;
            emit(&counts, &mut last_emit, &self.sender, false);
        }

        // 2. Audio files: stable key + tags, staged into the database.
        for file in &audio {
            if self.is_cancelled() {
                break;
            }
            self.register_audio(file, &mut counts);
            emit(&counts, &mut last_emit, &self.sender, false);
        }

        // 3. Lyrics files, after the tracks are committed so they can be associated.
        if !lyrics.is_empty() && !self.is_cancelled() {
            self.flush();
            let lookup = self.db_path.as_ref().and_then(|p| MetaCache::new(p).ok());
            for file in &lyrics {
                if self.is_cancelled() {
                    break;
                }
                self.register_lyrics(file, lookup.as_ref(), &mut counts);
                emit(&counts, &mut last_emit, &self.sender, false);
            }
        }

        // 4. Commit, report the final counts, then close the channel (= scan finished).
        self.flush();
        emit(&counts, &mut last_emit, &self.sender, true);
        lock(&self.progress_senders).remove(&self.id);
        lock(&self.jobs).remove(&self.id);
    }

    fn register_audio(&self, file: &Path, counts: &mut Progress) {
        let Some((size, mtime)) = size_and_mtime(file) else { return };
        let path = file.to_string_lossy().to_string();
        let stable_key = compute_partial_hash(file, 16).ok();
        self.db(DBCommand::UpsertFile {
            path: path.clone(),
            norm_path: path.clone(),
            size,
            mtime,
            device: None,
            inode: None,
            stable_key: stable_key.clone(),
        });
        let Some(partial) = stable_key else { return };
        counts.parsed += 1;

        let (title, artist, album_artist, composer, album) = extract_audio_tags(&file.to_path_buf());
        let now = unix_now();
        self.db(DBCommand::UpsertTrack {
            content_id: path.clone(),
            title,
            artist,
            album_artist,
            composer,
            album,
            canonical_location_id: None,
            created_at: now,
        });
        self.db(DBCommand::UpsertLocation {
            content_id: path.clone(),
            path: path.clone(),
            device: None,
            inode: None,
            size,
            mtime,
            last_seen: now,
            preferred: false,
            available: true,
        });
        self.db(DBCommand::UpsertHash {
            content_id: path,
            algo: "blake3-partial".to_string(),
            full_hash: None,
            partial_hash: Some(partial),
            computed_at: now,
        });
        counts.registered += 1;
    }

    fn register_lyrics(&self, file: &Path, lookup: Option<&MetaCache>, counts: &mut Progress) {
        let has_timestamps = Lyrics::from_lrc(file)
            .map(|lyrics| !lyrics.lines.is_empty())
            .unwrap_or(false);
        if !has_timestamps {
            return;
        }
        counts.parsed += 1;
        let Some((size, mtime)) = size_and_mtime(file) else { return };
        let parent_name = file
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let file_stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let content_id = lookup
            .and_then(|cache| cache.find_track_content_id_by_file_stem_and_parent(file_stem, parent_name).ok())
            .flatten();
        self.db(DBCommand::UpsertLyricsLocation {
            content_id,
            path: file.to_string_lossy().to_string(),
            size,
            mtime,
            last_seen: unix_now(),
        });
        counts.registered += 1;
    }
}
