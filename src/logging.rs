use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use log::{LevelFilter, Log, Metadata, Record};

pub struct SimpleFileLogger {
    file: Mutex<File>,
    dir: PathBuf,
    base: String,
    max_size: u64,
}

impl SimpleFileLogger {
    pub fn new(dir: PathBuf, base: &str, max_size: u64) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        let file_path = dir.join(format!("{}.log", base));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)?;
        Ok(Self {
            file: Mutex::new(file),
            dir,
            base: base.to_string(),
            max_size,
        })
    }

    fn rotate_if_needed(&self) {
        let file_path = self.dir.join(format!("{}.log", self.base));
        if let Ok(metadata) = fs::metadata(&file_path) {
            if metadata.len() > self.max_size {
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let rotated = self
                    .dir
                    .join(format!("{}.{}.log", self.base, ts));
                let _ = fs::rename(&file_path, rotated);
                // recreate the file for subsequent writes
                let _ = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&file_path);
            }
        }
    }
}

impl Log for SimpleFileLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let line = format!("[{}][{}] {}\n", ts, record.level(), record.args());

        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }

        self.rotate_if_needed();
    }

    fn flush(&self) {}
}

/// Initialize logging to the application's logs directory. This is best-effort.
pub fn init(paths: &crate::core::persistence::PersistencePaths) -> io::Result<()> {
    // Determine logs directory from the JsonStore data root (now stores logs/cache under data/)
    let store = crate::core::persistence::JsonStore::new(paths.clone());
    let logs_dir = store.logs_dir();
    fs::create_dir_all(&logs_dir)?;
    let logger = SimpleFileLogger::new(logs_dir, "lrc_player", 5 * 1024 * 1024)?;
    log::set_boxed_logger(Box::new(logger)).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("logger set error: {e}")))?;
    log::set_max_level(LevelFilter::Info);
    Ok(())
}
