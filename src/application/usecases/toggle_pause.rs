use crate::domain::model::PlaybackState;
use crate::domain::services::PlaybackService;

/// Toggle playback state using domain playback business rules.
pub fn execute(service: &PlaybackService, current_state: PlaybackState) -> PlaybackState {
    service.toggle_pause(current_state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_playing_to_paused() {
        let service = PlaybackService::new();
        let result = execute(&service, PlaybackState::Playing);
        assert_eq!(result, PlaybackState::Paused);
    }

    #[test]
    fn toggles_paused_to_playing() {
        let service = PlaybackService::new();
        let result = execute(&service, PlaybackState::Paused);
        assert_eq!(result, PlaybackState::Playing);
    }

    #[test]
    fn toggles_stopped_to_playing() {
        let service = PlaybackService::new();
        let result = execute(&service, PlaybackState::Stopped);
        assert_eq!(result, PlaybackState::Playing);
    }

    #[test]
    fn seeking_remains_seeking() {
        let service = PlaybackService::new();
        let result = execute(&service, PlaybackState::Seeking);
        assert_eq!(result, PlaybackState::Seeking);
    }
}
