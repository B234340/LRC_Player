#[cfg(test)]
mod tests {
    use lrc_player::domain::model::PlaybackState;
    use lrc_player::domain::services::PlaybackService;
    use lrc_player::application::usecases::toggle_pause::*;

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
