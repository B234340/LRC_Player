use crate::infrastructure::dedup::{compute_partial_hash, DedupService};
use crate::infrastructure::persistence::MetaCache;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

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
        progress_tx: mpsc::Sender<Progress>,
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
}

/// Handle to control the IO worker. Cloneable.
#[derive(Clone)]
pub struct IoWorkerHandle {
    cmd_tx: mpsc::Sender<WorkerCommand>,
    _inner: Arc<Mutex<()>>, // placeholder for possible shared state
}

impl IoWorkerHandle {
    pub fn start_scan(&self, paths: Vec<PathBuf>, options: ScanOptions) -> ScanHandle {
        let id = rand::random::<u64>();
        let (progress_tx, _progress_rx) = mpsc::channel(32);
        let cmd = WorkerCommand::Start {
            id,
            paths,
            options,
            progress_tx,
        };
        let _ = self.cmd_tx.try_send(cmd);
        ScanHandle { id }
    }

    pub fn cancel(&self, handle: &ScanHandle) -> Result<()> {
        let cmd = WorkerCommand::Cancel { id: handle.id };
        // best-effort send
        self.cmd_tx
            .try_send(cmd)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(())
    }

    pub fn subscribe(&self, _handle: &ScanHandle) -> mpsc::Receiver<Progress> {
        // For now return a fresh receiver that won't receive events; real implementation should wire per-job channels
        let (_tx, rx) = mpsc::channel(1);
        rx
    }
}

pub fn spawn_io_worker(db_path: Option<std::path::PathBuf>) -> IoWorkerHandle {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<WorkerCommand>(64);

    // DB command channel
    let (db_tx, mut db_rx) = mpsc::channel::<DBCommand>(64);

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
                                        if staged_count >= CHECKPOINT_N {
                                            // checkpoint now
                                            let _ = m.checkpoint();
                                            staged_count = 0;
                                        }
                                    }
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
    let db_tx_clone = db_tx.clone();
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                WorkerCommand::Start {
                    id,
                    paths,
                    options: _options,
                    progress_tx,
                } => {
                    let (cancel_tx, mut cancel_rx) = oneshot::channel();
                    jobs.lock().await.insert(id, cancel_tx);

                    // spawn background task for this scan job
                    let dedup = DedupService::spawn();
                    // clone DB sender for this job so the root sender isn't moved
                    let db_for_job = db_tx_clone.clone();
                    tokio::spawn(async move {
                        // rebind job-local db sender
                        let db_tx_clone = db_for_job;
                        if paths.is_empty() {
                            // fallback simulated scanning
                            let total = 5u64;
                            for i in 0..total {
                                if cancel_rx.try_recv().is_ok() {
                                    break;
                                }
                                let _ = progress_tx
                                    .send(Progress {
                                        scanned: i + 1,
                                        parsed: i,
                                        registered: i / 2,
                                    })
                                    .await;
                                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                            }
                        } else {
                            // Realistic per-path processing: partial hash -> enqueue full-hash job
                            for p in paths.into_iter() {
                                if cancel_rx.try_recv().is_ok() {
                                    break;
                                }

                                // compute partial hash in blocking thread
                                let p_clone = p.clone();
                                let partial_res = tokio::task::spawn_blocking(move || {
                                    compute_partial_hash(&p_clone, 16)
                                })
                                .await;
                                if let Ok(Ok(partial)) = partial_res {
                                    let _ = progress_tx
                                        .send(Progress {
                                            scanned: 1,
                                            parsed: 1,
                                            registered: 0,
                                        })
                                        .await;
                                    // record partial hash in DB staging via db_tx
                                    if let Ok(meta) = std::fs::metadata(&p) {
                                        let size = meta.len() as i64;
                                        let mtime = meta
                                            .modified()
                                            .ok()
                                            .and_then(|t| {
                                                t.duration_since(std::time::UNIX_EPOCH).ok()
                                            })
                                            .map(|d| d.as_secs() as i64)
                                            .unwrap_or(0);
                                        let norm = p.to_string_lossy().to_string();
                                        let db_sender = db_tx_clone.clone();
                                        let _ = db_sender
                                            .send(DBCommand::UpsertFile {
                                                path: p.to_string_lossy().to_string(),
                                                norm_path: norm.clone(),
                                                size,
                                                mtime,
                                                device: None,
                                                inode: None,
                                                stable_key: Some(partial.clone()),
                                            })
                                            .await;
                                    }

                                    // request full hash asynchronously
                                    let rx = dedup.request_full_hash(p.clone()).await;
                                    // prepare clones to move into spawned task without capturing db_tx_clone by reference
                                    let db_sender_for_task = db_tx_clone.clone();
                                    let p_inner = p.clone();
                                    let partial_inner = partial.clone();
                                    let progress_tx_cloned = progress_tx.clone();
                                    // spawn a task to await full hash without blocking the scan loop
                                    tokio::spawn(async move {
                                        match rx.await {
                                            Ok(Ok(full)) => {
                                                let _ = progress_tx_cloned
                                                    .send(Progress {
                                                        scanned: 1,
                                                        parsed: 1,
                                                        registered: 1,
                                                    })
                                                    .await;
                                                tracing::info!(
                                                    "full hash for {:?}: {} partial {}",
                                                    p_inner,
                                                    full,
                                                    partial_inner
                                                );
                                                // update DB with full hash
                                                if let Ok(meta) = std::fs::metadata(&p_inner) {
                                                    let size = meta.len() as i64;
                                                    let mtime = meta
                                                        .modified()
                                                        .ok()
                                                        .and_then(|t| {
                                                            t.duration_since(std::time::UNIX_EPOCH)
                                                                .ok()
                                                        })
                                                        .map(|d| d.as_secs() as i64)
                                                        .unwrap_or(0);
                                                    let norm =
                                                        p_inner.to_string_lossy().to_string();
                                                    let _ = db_sender_for_task
                                                        .send(DBCommand::UpsertFile {
                                                            path: p_inner
                                                                .to_string_lossy()
                                                                .to_string(),
                                                            norm_path: norm.clone(),
                                                            size,
                                                            mtime,
                                                            device: None,
                                                            inode: None,
                                                            stable_key: Some(full),
                                                        })
                                                        .await;
                                                }
                                            }
                                            Ok(Err(e)) => tracing::warn!(
                                                "full hash failed for {:?}: {}",
                                                p_inner,
                                                e
                                            ),
                                            Err(e) => tracing::warn!(
                                                "full hash receive error for {:?}: {}",
                                                p_inner,
                                                e
                                            ),
                                        }
                                    });
                                } else {
                                    // partial hash failed; report parsed=0
                                    let _ = progress_tx
                                        .send(Progress {
                                            scanned: 1,
                                            parsed: 0,
                                            registered: 0,
                                        })
                                        .await;
                                }
                            }
                        }
                        // job complete — drop progress_tx
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
        _inner: Arc::new(Mutex::new(())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_and_start() {
        let w = spawn_io_worker(None);
        let handle = w.start_scan(vec![], ScanOptions { fast: true });
        let res = w.cancel(&handle);
        assert!(res.is_ok());
    }
}
