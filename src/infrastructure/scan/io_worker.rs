use crate::domain::services::DedupService;
use crate::infrastructure::persistence::MetaCache;
use crate::infrastructure::scan::file_scanner::FileScanner;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub fast: bool,
}

#[derive(Clone, Debug)]
pub struct ScanHandle {
    pub id: u64,
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub scanned: u64,
    pub parsed: u64,
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
    Checkpoint,
}

/// Handle to control the IO worker. Cloneable.
#[derive(Clone)]
pub struct IoWorkerHandle {
    cmd_tx: mpsc::Sender<WorkerCommand>,
    progress_senders: Arc<Mutex<HashMap<u64, broadcast::Sender<Progress>>>>,
    db_tx: mpsc::Sender<DBCommand>,
    _inner: Arc<Mutex<()>>, // placeholder for possible shared state
}

impl IoWorkerHandle {
    pub async fn start_scan(
        &self,
        paths: Vec<PathBuf>,
        options: ScanOptions,
    ) -> Result<ScanHandle> {
        let id = rand::random::<u64>();
        let (progress_tx, _progress_rx) = broadcast::channel(32);
        let mut senders = self
            .progress_senders
            .try_lock()
            .expect("failed to lock progress_senders in start_scan");
        senders.insert(id, progress_tx.clone());

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
        let senders = self
            .progress_senders
            .try_lock()
            .expect("failed to lock progress_senders in subscribe");
        if let Some(sender) = senders.get(&handle.id) {
            sender.subscribe()
        } else {
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
    let progress_senders: Arc<Mutex<HashMap<u64, broadcast::Sender<Progress>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Active job cancellation senders
    let active_jobs: Arc<Mutex<HashMap<u64, oneshot::Sender<()>>>> =
        Arc::new(Mutex::new(HashMap::new()));

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
                            if let Some(m) = meta.as_ref() {
                                match db_cmd {
                                    DBCommand::UpsertFile { path, norm_path, size, mtime, device, inode, stable_key } => {
                                        let _ = m.upsert_file_staging(&path, &norm_path, size, mtime, device, inode, stable_key.as_deref());
                                        staged_count += 1;
                                    }
                                    DBCommand::UpsertTrack { content_id, title, artist, album, canonical_location_id, created_at } => {
                                        let _ = m.upsert_track_staging(
                                            &content_id,
                                            title.as_deref(),
                                            artist.as_deref(),
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
                                    DBCommand::Checkpoint => {
                                        let _ = m.checkpoint();
                                        staged_count = 0;
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
                    let (cancel_tx, mut cancel_rx) = oneshot::channel();
                    jobs.lock().await.insert(id, cancel_tx);

                    let sender = {
                        let senders = progress_senders_clone.lock().await;
                        senders
                            .get(&id)
                            .cloned()
                            .unwrap_or_else(|| broadcast::channel(1).0)
                    };

                    let dedup = DedupService::new();
                    let db_for_job = db_tx_clone.clone();
                    let fast_scan = options.fast;
                    tokio::spawn(async move {
                        let scanner = FileScanner::new();
                        if paths.is_empty() {
                            let total = 5u64;
                            for i in 0..total {
                                if cancel_rx.try_recv().is_ok() {
                                    break;
                                }
                                let _ = sender.send(Progress {
                                    scanned: i + 1,
                                    parsed: i,
                                    registered: i / 2,
                                });
                                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                            }
                        } else {
                            for path in paths.into_iter() {
                                if cancel_rx.try_recv().is_ok() {
                                    break;
                                }

                                let files = scanner.enumerate_paths(&path);
                                for file_path in files.into_iter() {
                                    if cancel_rx.try_recv().is_ok() {
                                        break;
                                    }

                                    let file_path_clone = file_path.clone();
                                    let dedup_for_partial = dedup.clone();
                                    let partial_result = tokio::task::spawn_blocking(move || {
                                        dedup_for_partial.compute_stable_key(&file_path_clone)
                                    })
                                    .await;

                                    let partial_hash = match partial_result {
                                        Ok(Ok(hash)) => Some(hash),
                                        _ => None,
                                    };

                                    let scanned = 1;
                                    let parsed = partial_hash.is_some() as u64;
                                    let registered = 0;
                                    let _ = sender.send(Progress {
                                        scanned,
                                        parsed,
                                        registered,
                                    });

                                    if let Some(partial) = partial_hash.clone() {
                                        if let Ok(meta) = std::fs::metadata(&file_path) {
                                            let size = meta.len() as i64;
                                            let mtime = meta
                                                .modified()
                                                .ok()
                                                .and_then(|t| {
                                                    t.duration_since(std::time::UNIX_EPOCH).ok()
                                                })
                                                .map(|d| d.as_secs() as i64)
                                                .unwrap_or(0);
                                            let norm = file_path.to_string_lossy().to_string();
                                            let db_sender = db_for_job.clone();
                                            let _ = db_sender
                                                .send(DBCommand::UpsertFile {
                                                    path: file_path.to_string_lossy().to_string(),
                                                    norm_path: norm.clone(),
                                                    size,
                                                    mtime,
                                                    device: None,
                                                    inode: None,
                                                    stable_key: Some(partial.clone()),
                                                })
                                                .await;

                                            let now_ts = std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .map(|d| d.as_secs() as i64)
                                                .unwrap_or(0);
                                            let _ = db_sender
                                                .send(DBCommand::UpsertTrack {
                                                    content_id: partial.clone(),
                                                    title: None,
                                                    artist: None,
                                                    album: None,
                                                    canonical_location_id: None,
                                                    created_at: now_ts,
                                                })
                                                .await;

                                            let _ = db_sender
                                                .send(DBCommand::UpsertHash {
                                                    content_id: partial.clone(),
                                                    algo: "blake3-partial".to_string(),
                                                    full_hash: None,
                                                    partial_hash: Some(partial.clone()),
                                                    computed_at: now_ts,
                                                })
                                                .await;
                                        }

                                        if fast_scan {
                                            let _ = sender.send(Progress {
                                                scanned: 1,
                                                parsed: 1,
                                                registered: 1,
                                            });
                                            continue;
                                        }

                                        let request_path = file_path.clone();
                                        let db_sender_for_task = db_for_job.clone();
                                        let sender_cloned = sender.clone();
                                        let partial_clone = partial.clone();
                                        let dedup_for_full = dedup.clone();
                                        tokio::spawn(async move {
                                            let full_rx = dedup_for_full
                                                .request_full_hash(request_path.clone())
                                                .await;
                                            match full_rx.await {
                                                Ok(Ok(full)) => {
                                                    let _ = sender_cloned.send(Progress {
                                                        scanned: 1,
                                                        parsed: 1,
                                                        registered: 1,
                                                    });
                                                    let _ = db_sender_for_task
                                                        .send(DBCommand::UpsertHash {
                                                            content_id: partial_clone.clone(),
                                                            algo: "blake3-full".to_string(),
                                                            full_hash: Some(full.clone()),
                                                            partial_hash: Some(
                                                                partial_clone.clone(),
                                                            ),
                                                            computed_at:
                                                                std::time::SystemTime::now()
                                                                    .duration_since(
                                                                        std::time::UNIX_EPOCH,
                                                                    )
                                                                    .map(|d| d.as_secs() as i64)
                                                                    .unwrap_or(0),
                                                        })
                                                        .await;
                                                }
                                                Ok(Err(_)) | Err(_) => {
                                                    let _ = sender_cloned.send(Progress {
                                                        scanned: 1,
                                                        parsed: 0,
                                                        registered: 0,
                                                    });
                                                }
                                            }
                                        });
                                    } else if let Ok(meta) = std::fs::metadata(&file_path) {
                                        let size = meta.len() as i64;
                                        let mtime = meta
                                            .modified()
                                            .ok()
                                            .and_then(|t| {
                                                t.duration_since(std::time::UNIX_EPOCH).ok()
                                            })
                                            .map(|d| d.as_secs() as i64)
                                            .unwrap_or(0);
                                        let norm = file_path.to_string_lossy().to_string();
                                        let db_sender = db_for_job.clone();
                                        let _ = db_sender
                                            .send(DBCommand::UpsertFile {
                                                path: file_path.to_string_lossy().to_string(),
                                                norm_path: norm.clone(),
                                                size,
                                                mtime,
                                                device: None,
                                                inode: None,
                                                stable_key: None,
                                            })
                                            .await;
                                    }
                                }
                            }
                        }
                    });
                }
                WorkerCommand::Cancel { id } => {
                    if let Some(tx) = jobs.lock().await.remove(&id) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;
    use tokio::time::{sleep, timeout, Duration};

    #[tokio::test]
    async fn spawn_and_start() {
        let w = spawn_io_worker(None);
        let handle = w
            .start_scan(vec![], ScanOptions { fast: true })
            .await
            .expect("start scan");
        let res = w.cancel(&handle).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn checkpoint_writes_scanned_file_to_meta_cache() {
        let db_dir = tempdir().unwrap();
        let db_path = db_dir.path().join("scan.db");
        let scan_dir = tempdir().unwrap();
        let test_file = scan_dir.path().join("track.mp3");
        let mut file = File::create(&test_file).unwrap();
        use std::io::Write;
        file.write_all(b"test audio content").unwrap();

        let worker = spawn_io_worker(Some(db_path.clone()));
        let handle = worker
            .start_scan(
                vec![scan_dir.path().to_path_buf()],
                ScanOptions { fast: true },
            )
            .await
            .expect("start scan");
        let mut progress_rx = worker.subscribe(&handle);

        let _ = timeout(Duration::from_secs(3), progress_rx.recv())
            .await
            .expect("expected at least one progress event");

        worker.checkpoint_now().await.expect("checkpoint send");
        sleep(Duration::from_secs(1)).await;

        let cache = MetaCache::new(db_path).expect("open db");
        let id = cache
            .get_file_id(test_file.to_string_lossy().as_ref())
            .expect("query file id");
        assert!(id.is_some(), "scanned file should appear in metadata cache");
    }
}
