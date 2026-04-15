#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Buffering,
    Playing,
    Paused,
    Seeking,
}
