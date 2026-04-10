use serde::{Deserialize, Serialize};

use crate::core::ids::TrackId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Buffering,
    Playing,
    Paused,
    Seeking,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackModel {
    pub state: PlaybackState,
    pub current_track: Option<TrackId>,
    pub position_ms: u64,
    pub buffered_ms: u32,
    pub volume: f32,
    resume_after_seek: PlaybackState,
}

impl Default for PlaybackModel {
    fn default() -> Self {
        Self {
            state: PlaybackState::Stopped,
            current_track: None,
            position_ms: 0,
            buffered_ms: 0,
            volume: 1.0,
            resume_after_seek: PlaybackState::Playing,
        }
    }
}

impl PlaybackModel {
    pub fn play_track(&mut self, track_id: TrackId) {
        self.current_track = Some(track_id);
        self.position_ms = 0;
        self.buffered_ms = 0;
        self.state = PlaybackState::Buffering;
        self.state = PlaybackState::Playing;
    }

    pub fn toggle_pause(&mut self) {
        self.state = match self.state {
            PlaybackState::Playing => PlaybackState::Paused,
            PlaybackState::Paused => PlaybackState::Playing,
            PlaybackState::Buffering | PlaybackState::Seeking => self.state,
            PlaybackState::Stopped => PlaybackState::Stopped,
        };
    }

    pub fn seek_to(&mut self, position_ms: u64) {
        self.resume_after_seek = match self.state {
            PlaybackState::Paused => PlaybackState::Paused,
            _ => PlaybackState::Playing,
        };
        self.state = PlaybackState::Seeking;
        self.position_ms = position_ms;
        self.state = self.resume_after_seek;
    }

    pub fn seek_by(&mut self, delta_ms: i64) {
        let next_position = if delta_ms.is_negative() {
            self.position_ms.saturating_sub(delta_ms.unsigned_abs())
        } else {
            self.position_ms.saturating_add(delta_ms as u64)
        };
        self.seek_to(next_position);
    }

    pub fn stop(&mut self) {
        self.state = PlaybackState::Stopped;
        self.current_track = None;
        self.position_ms = 0;
        self.buffered_ms = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{PlaybackModel, PlaybackState};
    use crate::core::ids::TrackId;

    #[test]
    fn seek_returns_to_paused_state() {
        let mut playback = PlaybackModel::default();
        playback.play_track(TrackId(10));
        playback.toggle_pause();

        playback.seek_to(42_000);

        assert_eq!(playback.state, PlaybackState::Paused);
        assert_eq!(playback.position_ms, 42_000);
    }
}