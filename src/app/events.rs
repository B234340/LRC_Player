//! アプリ内で発生するイベント列挙
//!
//! `AppEvent` / `PlaybackEvent` は `AppFacade` が操作の結果として UI に通知するために使用します。
use crate::core::ids::TrackId;
use crate::core::persistence::TabId;
use crate::core::playback::PlaybackState;
use crate::core::scan::ScanState;

#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackEvent {
    /// 再生状態が変化した（Playing/Paused 等）
    StateChanged(PlaybackState),
    /// 再生トラックが変わった（None = 停止状態）
    TrackChanged(Option<TrackId>),
    /// 再生位置やバッファ量の更新
    PositionUpdated { position_ms: u64, buffered_ms: u32 },
    /// オーディオデバイスが利用不能になった（underrun）
    DeviceUnderrun,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    /// 再生関連イベントを内包する
    Playback(PlaybackEvent),
    QueueChanged,
    PlaylistChanged,
    SettingsChanged,
    ScanStateChanged(ScanState),
    TabChanged(TabId),
    TrackDetailsRequested(TrackId),
}
