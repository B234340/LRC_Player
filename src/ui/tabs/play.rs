//! Now Playing タブ用の表示型
//!
//! `PlayTab` は現在再生中のトラック情報と再生状態/位置を UI に提供する構造体です。
use crate::app::view_models::TrackRowVm;
use crate::core::playback::PlaybackState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayTab {
    /// 現在のトラック（未選択時は None）
    pub current_track: Option<TrackRowVm>,
    /// 再生状態（Playing/Paused 等）
    pub playback_state: PlaybackState,
    /// 再生位置（ミリ秒）
    pub position_ms: u64,
}
