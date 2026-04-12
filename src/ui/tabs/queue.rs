//! Queue タブ用の表示データ
//!
//! `QueueTab` は再生キューの行一覧とリピートモードを UI に提供します。
use crate::app::view_models::TrackRowVm;
use crate::core::queue::RepeatMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueTab {
    /// キューに表示するトラック行
    pub rows: Vec<TrackRowVm>,
    /// 現在のリピートモード
    pub repeat_mode: RepeatMode,
}
