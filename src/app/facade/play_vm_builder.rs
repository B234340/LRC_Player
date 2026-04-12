use crate::app::view_models::PlayTabVm;

/// Build PlayTabVm from facade state.
pub fn build_play_tab(facade: &super::AppFacade) -> PlayTabVm {
    PlayTabVm {
        current_track: facade
            .playback
            .current_track
            .map(|track_id| facade.track_row_vm(track_id, None)),
        playback_state: facade.playback.state,
        position_ms: facade.playback.position_ms,
    }
}
