//! Debounced, background JSON state saver.
//!
//! `JsonStore::save` serializes pretty JSON, `fsync`s and renames on the calling
//! thread. Calling it from UI hot paths (every frame of a seek drag, every play,
//! every view-model refresh) stalls the UI. `StateSaver` instead:
//!
//! * accepts values with [`StateSaver::save`] (non-blocking; just a channel send),
//! * coalesces bursts: at most one write per `debounce` interval, always writing
//!   the most recent value (latest wins), with bounded latency (a write happens
//!   no later than `debounce` after the first unsaved value arrived),
//! * writes on a dedicated background thread: compact JSON -> temp file in the
//!   same directory -> `fsync` -> atomic rename over the target,
//! * [`StateSaver::flush`] blocks until everything submitted so far is on disk
//!   (call it on exit, e.g. from eframe's `on_exit`); `Drop` also flushes.

use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Default coalescing window used by [`StateSaver::open_default`].
pub const DEFAULT_STATE_SAVE_DEBOUNCE: Duration = Duration::from_millis(1500);

enum Msg {
    Save(Vec<u8>),
    Flush(Sender<std::result::Result<(), String>>),
    Shutdown,
}

pub struct StateSaver<T> {
    tx: Sender<Msg>,
    worker: Option<JoinHandle<()>>,
    path: PathBuf,
    last_error: Arc<Mutex<Option<String>>>,
    _marker: PhantomData<fn(&T)>,
}

impl<T: Serialize> StateSaver<T> {
    /// Start a saver for `path` that writes at most once per `debounce`.
    pub fn new<P: AsRef<Path>>(path: P, debounce: Duration) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let (tx, rx) = mpsc::channel::<Msg>();
        let last_error = Arc::new(Mutex::new(None));
        let worker_path = path.clone();
        let worker_error = Arc::clone(&last_error);
        let worker = std::thread::Builder::new()
            .name("state-saver".to_string())
            .spawn(move || {
                let mut pending: Option<Vec<u8>> = None;
                let mut deadline: Option<Instant> = None;
                let write = |bytes: &[u8]| -> std::result::Result<(), String> {
                    let res = write_atomic(&worker_path, bytes).map_err(|e| format!("{e:#}"));
                    *worker_error.lock().unwrap_or_else(|p| p.into_inner()) = res.clone().err();
                    res
                };
                loop {
                    let msg = match deadline {
                        Some(at) => {
                            let now = Instant::now();
                            if now >= at {
                                Err(RecvTimeoutError::Timeout)
                            } else {
                                rx.recv_timeout(at - now)
                            }
                        }
                        None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                    };
                    match msg {
                        Ok(Msg::Save(bytes)) => {
                            pending = Some(bytes);
                            if deadline.is_none() {
                                deadline = Some(Instant::now() + debounce);
                            }
                        }
                        Ok(Msg::Flush(reply)) => {
                            let res = match pending.take() {
                                Some(bytes) => write(&bytes),
                                None => Ok(()),
                            };
                            deadline = None;
                            let _ = reply.send(res);
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if let Some(bytes) = pending.take() {
                                let _ = write(&bytes);
                            }
                            deadline = None;
                        }
                        Ok(Msg::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                            if let Some(bytes) = pending.take() {
                                let _ = write(&bytes);
                            }
                            return;
                        }
                    }
                }
            })
            .context("spawning state saver thread")?;

        Ok(Self {
            tx,
            worker: Some(worker),
            path,
            last_error,
            _marker: PhantomData,
        })
    }

    /// Saver for the default `data/state.json` next to the executable
    /// (same file as `StateFile::open()`), with [`DEFAULT_STATE_SAVE_DEBOUNCE`].
    pub fn open_default() -> Result<Self> {
        Self::new(default_state_path()?, DEFAULT_STATE_SAVE_DEBOUNCE)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Queue `value` to be written. Serializes on the calling thread (compact
    /// JSON, no I/O) and returns immediately. Bursts are coalesced.
    pub fn save(&self, value: &T) -> Result<()> {
        let bytes = serde_json::to_vec(value)
            .with_context(|| format!("serializing state for {}", self.path.display()))?;
        self.tx
            .send(Msg::Save(bytes))
            .map_err(|_| anyhow::anyhow!("state saver thread has stopped"))
    }

    /// Write any pending value now and wait until it is on disk.
    pub fn flush(&self) -> Result<()> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Msg::Flush(reply_tx))
            .map_err(|_| anyhow::anyhow!("state saver thread has stopped"))?;
        reply_rx
            .recv()
            .map_err(|_| anyhow::anyhow!("state saver thread has stopped"))?
            .map_err(|e| anyhow::anyhow!(e))
    }

    /// Error message of the most recent background write, if it failed.
    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl<T> Drop for StateSaver<T> {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// `data/state.json` next to the current executable (directory is created).
pub fn default_state_path() -> Result<PathBuf> {
    let exe_path = std::env::current_exe().context("unable to determine current executable path")?;
    let exe_dir = exe_path
        .parent()
        .context("current executable has no parent directory")?;
    let data_dir = exe_dir.join("data");
    fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating data directory {}", data_dir.display()))?;
    Ok(data_dir.join("state.json"))
}

/// Write `bytes` to `path` atomically: temp file in the same directory, fsync,
/// rename over the target. Readers never observe a partially written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent directory {}", parent.display()))?;
    }
    let file_name = path
        .file_name()
        .with_context(|| format!("{} has no file name", path.display()))?
        .to_string_lossy();
    // Distinct from JsonStore's `<stem>.tmp` so both may coexist safely.
    let tmp_name = format!(".{}.{}.saving", file_name, std::process::id());
    let tmp_path = match parent {
        Some(parent) => parent.join(tmp_name),
        None => PathBuf::from(tmp_name),
    };
    let result = (|| -> Result<()> {
        let mut file = fs::File::create(&tmp_path)
            .with_context(|| format!("creating temp file {}", tmp_path.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("writing temp file {}", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing temp file {}", tmp_path.display()))?;
        drop(file);
        fs::rename(&tmp_path, path)
            .with_context(|| format!("renaming {} to {}", tmp_path.display(), path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct S {
        n: u32,
        queue: Vec<String>,
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lrc_state_saver_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(path: &Path) -> S {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn coalesces_bursts_and_writes_latest_after_debounce() {
        let dir = temp_dir("coalesce");
        let path = dir.join("state.json");
        let saver = StateSaver::<S>::new(&path, Duration::from_millis(400)).unwrap();
        let start = Instant::now();
        for n in 0..500 {
            saver.save(&S { n, queue: vec!["x".into(); 10] }).unwrap();
        }
        // Saving 500 times must not block on disk I/O.
        assert!(start.elapsed() < Duration::from_millis(500));
        assert!(!path.exists(), "nothing written before the debounce window");
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(read(&path).n, 499);
        assert!(saver.last_error().is_none());
        drop(saver);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_writes_immediately_and_drop_flushes() {
        let dir = temp_dir("flush");
        let path = dir.join("state.json");
        let saver = StateSaver::<S>::new(&path, Duration::from_secs(3600)).unwrap();
        saver.save(&S { n: 1, queue: vec![] }).unwrap();
        saver.flush().unwrap();
        assert_eq!(read(&path).n, 1);
        saver.save(&S { n: 2, queue: vec![] }).unwrap();
        drop(saver);
        assert_eq!(read(&path).n, 2);
        // No temp files left behind.
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() != "state.json")
            .collect();
        assert!(leftovers.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn output_is_readable_by_json_store() {
        let dir = temp_dir("compat");
        let path = dir.join("state.json");
        {
            let saver = StateSaver::<S>::new(&path, Duration::from_millis(10)).unwrap();
            saver.save(&S { n: 7, queue: vec!["a".into()] }).unwrap();
        }
        let store = crate::infrastructure::persistence::JsonStore::open(&path).unwrap();
        let loaded: S = store.load().unwrap();
        assert_eq!(loaded, S { n: 7, queue: vec!["a".into()] });
        let _ = fs::remove_dir_all(&dir);
    }
}
