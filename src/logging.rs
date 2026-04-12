//! 非同期ファイルロガー実装
//!
//! - バウンデッドチャネル経由でログをキューに入れ、別スレッドがファイル書き込みを行う
//! - `try_send` を使うことで呼び出し元スレッドをブロックしない（高頻度ログでの遅延軽減）
//! - ログローテーションはベストエフォートで行う（サイズ超過でリネーム）
//!
//! UI 側で選べるカテゴリフィルタをサポートし、ランタイムでフィルタを適用します。

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH, Instant};
use std::thread;

use log::{LevelFilter, Log, Metadata, Record};
use crossbeam_channel::{bounded, TrySendError};

// UI に提示する組み込みカテゴリ（サブストリング/プレフィクスでマッチ）
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

// 実行時に選択されるログフィルタ（String の集合）を保持するためのグローバル
static LOG_FILTERS: OnceLock<Arc<RwLock<Vec<String>>>> = OnceLock::new();

/// 利用可能なログカテゴリのリストを文字列ベクタで返すユーティリティ
pub fn available_log_categories() -> Vec<String> {
    AVAILABLE_CATEGORIES.iter().map(|s| s.to_string()).collect()
}

/// ランタイムでのログフィルタを設定する。
/// UI から選択されたカテゴリをこの関数で反映させる想定。
pub fn set_log_filters(filters: Vec<String>) {
    let arc = LOG_FILTERS.get_or_init(|| Arc::new(RwLock::new(Vec::new()))).clone();
    if let Ok(mut guard) = arc.write() {
        guard.clear();
        guard.extend(filters);
    }
}

/// 非同期ファイルロガー本体
///
/// 内部で bounded チャネルを用い、書き込みは別スレッドで行うため呼び出し側は基本的にノンブロッキング。
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

        // 背景スレッドを生成し、チャネルから来る行をファイルに書き込む
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
                // ベストエフォートなローテーション処理: ファイルサイズをチェックして超過していればリネーム
                let rot_start = Instant::now();
                let file_path = dir_clone.join(format!("{}.log", base_string));
                if let Ok(metadata) = fs::metadata(&file_path) {
                    if metadata.len() > max_size {
                        // タイムスタンプを付与してログをローテート
                        let ts = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let rotated = dir_clone.join(format!("{}.{}.log", base_string, ts));
                        let _ = fs::rename(&file_path, rotated);
                        // 再オープン。失敗時は /dev/null へフォールバック
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

        // ランタイムフィルタの適用。フィルタが空なら全許可、そうでなければターゲットにマッチするもののみ許可。
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

/// 永続化パスに基づいて非同期ファイルロガーを初期化する。
///
/// - `paths` からログディレクトリを取得してディレクトリを作成
/// - `AsyncFileLogger` を生成して `log` クレートのロガーとして登録
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
