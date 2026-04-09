use crate::core::models::TrackId;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone)]
pub struct PlaybackSnapshot {
    pub status: PlaybackStatus,
    pub current_track: Option<TrackId>,
    pub position_ms: u64,
}

impl Default for PlaybackSnapshot {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            current_track: None,
            position_ms: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum PlaybackCommand {
    PlayTrack(TrackId),
    Pause,
    Resume,
    Stop,
    Seek(u64),
}

#[derive(Debug)]
pub struct PlayerError {
    message: String,
}

impl PlayerError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for PlayerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PlayerError {}

pub trait PlayerBackend {
    fn dispatch(&mut self, command: PlaybackCommand) -> Result<(), PlayerError>;
    fn snapshot(&self) -> PlaybackSnapshot;
}

#[derive(Debug, Default)]
pub struct NullPlayer {
    snapshot: PlaybackSnapshot,
}

impl PlayerBackend for NullPlayer {
    fn dispatch(&mut self, command: PlaybackCommand) -> Result<(), PlayerError> {
        match command {
            PlaybackCommand::PlayTrack(track_id) => {
                self.snapshot.current_track = Some(track_id);
                self.snapshot.position_ms = 0;
                self.snapshot.status = PlaybackStatus::Playing;
            }
            PlaybackCommand::Pause => {
                self.snapshot.status = PlaybackStatus::Paused;
            }
            PlaybackCommand::Resume => {
                self.snapshot.status = PlaybackStatus::Playing;
            }
            PlaybackCommand::Stop => {
                self.snapshot.status = PlaybackStatus::Stopped;
                self.snapshot.position_ms = 0;
            }
            PlaybackCommand::Seek(position_ms) => {
                self.snapshot.position_ms = position_ms;
            }
        }

        Ok(())
    }

    fn snapshot(&self) -> PlaybackSnapshot {
        self.snapshot.clone()
    }
}