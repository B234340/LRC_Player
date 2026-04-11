use crate::core::ids::TrackId;
use crate::core::persistence::TabId;
use crate::core::playback::PlaybackState;
use crate::core::scan::ScanState;

#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackEvent {
    StateChanged(PlaybackState),
    TrackChanged(Option<TrackId>),
    PositionUpdated { position_ms: u64, buffered_ms: u32 },
    DeviceUnderrun,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    Playback(PlaybackEvent),
    QueueChanged,
    PlaylistChanged,
    SettingsChanged,
    ScanStateChanged(ScanState),
    TabChanged(TabId),
    TrackDetailsRequested(TrackId),
}
