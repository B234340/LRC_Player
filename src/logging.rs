use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH, Instant};
use std::thread;

use log::{LevelFilter, Log, Metadata, Record};
use crossbeam_channel::{bounded, TrySendError};

// Built-in categories presented to the UI for selection. These are matched
// against the record target (module path) using substring/prefix matching.
const AVAILABLE_CATEGORIES: &[&str] = &[
    "all",
    "app",
    "core",
    "audio",
    "decode",
    "ui",
    "symphonia",
    "lofty",
    "cpal",
];

static LOG_FILTERS: OnceLock<Arc<RwLock<Vec<String>>>> = OnceLock::new();

pub fn available_log_categories() -> Vec<String> {
    AVAILABLE_CATEGORIES.iter().map(|s| s.to_string()).collect()
}

pub fn set_log_filters(filters: Vec<String>) {
    let arc = LOG_FILTERS.get_or_init(|| Arc::new(RwLock::new(Vec::new()))).clone();
    if let Ok(mut guard) = arc.write() {
        guard.clear();
        guard.extend(filters);
    }
}

/// Async file logger: non-blocking try_send on a bounded channel with
/// a background writer thread performing actual file I/O.
pub struct AsyncFileLogger {
    sender: crossbeam_channel::Sender<String>,
    dropped: AtomicUsize,
}

impl AsyncFileLogger {
    pub fn new(dir: PathBuf, base: &str, max_size: u64, queue_capacity: usize) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        let file_path = dir.join(format!("{}.log", base));

        let (s, r) = bounded::<String>(queue_capacity);

        let dir_clone = dir.clone();
        let base_string = base.to_string();

        thread::spawn(move || {
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file_path)
                .unwrap_or_else(|e| {
                    eprintln!("[logger] failed open file: {}", e);
                    // fallback to /dev/null if file cannot be opened
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open("/dev/null")
                        .unwrap()
                });

            for line in r.iter() {
                let write_start = Instant::now();
                let _ = file.write_all(line.as_bytes());
                let _ = file.flush();
                let write_dur = write_start.elapsed();
                if write_dur.as_micros() > 200 {
                    eprintln!("[logger] write_us={}", write_dur.as_micros());
                }

                // rotation (best-effort)
                let rot_start = Instant::now();
                let file_path = dir_clone.join(format!("{}.log", base_string));
                if let Ok(metadata) = fs::metadata(&file_path) {
                    if metadata.len() > max_size {
                        let ts = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let rotated = dir_clone.join(format!("{}.{}.log", base_string, ts));
                        let _ = fs::rename(&file_path, rotated);
                        file = OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&file_path)
                            .unwrap_or_else(|e| {
                                eprintln!("[logger] failed reopen file: {}", e);
                                OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open("/dev/null")
                                    .unwrap()
                            });
                    }
                }
                let rot_dur = rot_start.elapsed();
                if rot_dur.as_millis() > 10 {
                    eprintln!("[logger] rotate_ms={}", rot_dur.as_millis());
                }
            }
        });

        Ok(Self {
            sender: s,
            dropped: AtomicUsize::new(0),
        })
    }
}

impl Log for AsyncFileLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        // Apply runtime log filters (if any). If filters vector is empty,
        // allow all records. Otherwise only allow records whose target
        // matches at least one selected filter (or when "all" is selected).
        if let Some(filters_arc) = LOG_FILTERS.get() {
            let filters = filters_arc.read().unwrap();
            if !filters.is_empty() {
                let target = record.target();
                let mut matched = false;
                for f in filters.iter() {
                    if f == "all" {
                        matched = true;
                        break;
                    }
                    if target.starts_with(f) || target.contains(f) {
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    return;
                }
            }
        }

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let line = format!("[{}][{}] {}\n", ts, record.level(), record.args());

        match self.sender.try_send(line) {
            Ok(_) => {}
            Err(TrySendError::Full(_)) => {
                let prev = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                if prev % 100 == 0 {
                    eprintln!("[logger] dropped_count={}", prev);
                }
            }
            Err(TrySendError::Disconnected(_)) => {
                // receiver thread is gone; nothing to do
            }
        }
    }

    fn flush(&self) {}
}

/// Initialize logging with an async file sink.
pub fn init(paths: &crate::core::persistence::PersistencePaths) -> io::Result<()> {
    let store = crate::core::persistence::JsonStore::new(paths.clone());
    let logs_dir = store.logs_dir();
    fs::create_dir_all(&logs_dir)?;
    let logger = AsyncFileLogger::new(logs_dir, "lrc_player", 5 * 1024 * 1024, 8192)?;
    log::set_boxed_logger(Box::new(logger))
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("logger set error: {e}")))?;
    // Set initial max log level and filters from persisted app config (if available),
    // so the in-app "Enable debug mode" and selected filters persist across restarts.
    match store.load_config() {
        Ok(cfg) => {
            let initial_level = if cfg.debug_mode {
                LevelFilter::Debug
            } else {
                LevelFilter::Info
            };
            log::set_max_level(initial_level);
            LOG_FILTERS.get_or_init(|| Arc::new(RwLock::new(cfg.log_filters)));
        }
        Err(_) => {
            log::set_max_level(LevelFilter::Info);
            LOG_FILTERS.get_or_init(|| Arc::new(RwLock::new(Vec::new())));
        }
    }
    Ok(())
}
