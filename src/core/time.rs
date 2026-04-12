//! Core::time — タイミング関連の定数とシーク用の小さな型
//!
//! このモジュールは再生位置やシーク操作に関する定数と、
//! UI/再生ロジック間でやり取りされる小さな値オブジェクトを定義します。
//!
//! - `DEFAULT_SEEK_STEP_MS`: デフォルトのシークステップ（ミリ秒単位）
//! - `POSITION_SAVE_THRESHOLD_MS`: 再生位置を自動保存する閾値（ミリ秒）

/// デフォルトのシークステップ（ミリ秒）
pub const DEFAULT_SEEK_STEP_MS: u64 = 10_000;

/// ある程度の移動が発生した場合に再生位置を保存する閾値（ミリ秒）
pub const POSITION_SAVE_THRESHOLD_MS: u64 = 3_000;

/// シーク量（ミリ秒）を表す小さなラッパ型
///
/// UI や Playback ロジック間でシークを伝搬する際に使われます。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeekAmount {
    /// シーク長（ミリ秒）
    pub milliseconds: u64,
}

impl Default for SeekAmount {
    fn default() -> Self {
        Self {
            milliseconds: DEFAULT_SEEK_STEP_MS,
        }
    }
}
