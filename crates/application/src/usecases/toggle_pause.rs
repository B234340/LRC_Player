use lrc_player_domain::model::PlaybackState;
use lrc_player_domain::services::PlaybackService;

/// Toggle playback state using domain playback business rules.
pub fn execute(service: &PlaybackService, current_state: PlaybackState) -> PlaybackState {
    service.toggle_pause(current_state)
}


