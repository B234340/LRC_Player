//! ログユーティリティの互換スタブ
//!
//! このモジュールはログ出力を行わない前提で最小限の互換 API を提供します。
//! - `available_log_categories` / `set_log_filters` は UI 互換性のために残します。
//! - `init` は副作用を持たず Ok(()) を返します。

use std::io;
use std::sync::{Arc, RwLock, OnceLock};

// UI に提示するカテゴリ一覧（互換性のため保持）
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

pub fn init(_paths: &crate::core::persistence::PersistencePaths) -> io::Result<()> {
    // No-op: 互換のために存在するが副作用は与えない
    LOG_FILTERS.get_or_init(|| Arc::new(RwLock::new(Vec::new())));
    Ok(())
}
