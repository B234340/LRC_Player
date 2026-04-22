use crate::model::PlaybackState;

/// Pure playback business logic.
pub struct PlaybackService;

impl PlaybackService {
    pub fn new() -> Self {
        PlaybackService
    }

    /// Toggle between play and pause states.
    ///
    /// - Playing/Buffering -> Paused
    /// - Paused/Stopped -> Playing
    /// - Seeking remains Seeking
    pub fn toggle_pause(&self, current: PlaybackState) -> PlaybackState {
        match current {
            PlaybackState::Playing | PlaybackState::Buffering => PlaybackState::Paused,
            PlaybackState::Paused | PlaybackState::Stopped => PlaybackState::Playing,
            PlaybackState::Seeking => PlaybackState::Seeking,
        }
    }
}

impl Default for PlaybackService {
    fn default() -> Self {
        Self::new()
    }
}
