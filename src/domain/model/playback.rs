use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Buffering,
    Playing,
    Paused,
    Seeking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackMode {
    Sequential,
    RepeatOne,
    RepeatAll,
    Shuffle,
}

impl Default for PlaybackMode {
    fn default() -> Self {
        PlaybackMode::Sequential
    }
}

impl fmt::Display for PlaybackMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            PlaybackMode::Sequential => "Sequential",
            PlaybackMode::RepeatOne => "Repeat One",
            PlaybackMode::RepeatAll => "Repeat All",
            PlaybackMode::Shuffle => "Shuffle",
        };
        write!(f, "{}", label)
    }
}

impl fmt::Display for PlaybackState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            PlaybackState::Stopped => "Stopped",
            PlaybackState::Buffering => "Buffering",
            PlaybackState::Playing => "Playing",
            PlaybackState::Paused => "Paused",
            PlaybackState::Seeking => "Seeking",
        };
        write!(f, "{}", label)
    }
}
