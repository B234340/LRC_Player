pub mod commands;
pub mod events;
pub mod facade;
pub mod view_models;

use crate::app::commands::AppCommand;
use crate::app::events::{AppEvent, PlaybackEvent};
use crate::app::view_models::AppViewModel;
use self::facade::AppFacade;

use crate::core::ids::TrackId;
use crate::core::persistence::{JsonStore, PersistencePaths};
use crate::core::playback::PlaybackState;
use crate::core::time::POSITION_SAVE_THRESHOLD_MS;

pub struct Application {
    facade: AppFacade,
    store: JsonStore,
    last_saved_track: Option<TrackId>,
    last_saved_position_ms: u64,
}

impl Default for Application {
    fn default() -> Self {
        let paths = PersistencePaths::for_app("lrc_player").unwrap_or_else(|_| PersistencePaths {
            config_json: "config.json".into(),
            state_json: "state.json".into(),
            playlists_json: "playlists.json".into(),
        });
        Self::with_store(AppFacade::default(), JsonStore::new(paths))
    }
}

impl Application {
    fn with_store(facade: AppFacade, store: JsonStore) -> Self {
        Self {
            last_saved_track: facade.playback.current_track,
            last_saved_position_ms: facade.playback.position_ms,
            facade,
            store,
        }
    }

    pub fn initialize(&mut self) -> std::io::Result<()> {
        self.facade.bootstrap_from_store(&self.store)?;
        self.refresh_save_baseline();
        Ok(())
    }

    pub fn persist(&mut self) -> std::io::Result<()> {
        self.facade.save_to_store(&self.store)?;
        self.refresh_save_baseline();
        Ok(())
    }

    pub fn view_model(&mut self) -> AppViewModel {
        self.facade.view_model()
    }

    pub fn handle_command(&mut self, command: AppCommand) {
        let events = self.facade.handle_command(command);
        self.maybe_persist_after_events(&events);
    }

    pub fn poll_background_tasks(&mut self) {
        let events = self.facade.poll_background_tasks();
        self.maybe_persist_after_events(&events);
    }

    pub fn bootstrap_summary(&self) -> String {
        self.facade.bootstrap_summary()
    }

    pub fn run_demo(&mut self) {
        self.facade.load_demo_state();
    }

    pub fn facade(&self) -> &AppFacade {
        &self.facade
    }

    pub fn facade_mut(&mut self) -> &mut AppFacade {
        &mut self.facade
    }

    fn maybe_persist_after_events(&mut self, events: &[AppEvent]) {
        let mut save_for_track_change = false;
        let mut save_for_position = false;

        for event in events {
            if let AppEvent::Playback(playback_event) = event {
                match playback_event {
                    PlaybackEvent::TrackChanged(track) => {
                        if *track != self.last_saved_track {
                            save_for_track_change = true;
                        }
                    }
                    PlaybackEvent::StateChanged(PlaybackState::Paused) => {
                        save_for_position = true;
                    }
                    PlaybackEvent::PositionUpdated { .. } => {
                        save_for_position = true;
                    }
                    _ => {}
                }
            }
        }

        let should_persist = save_for_track_change
            || (save_for_position && self.should_persist_position());
        if should_persist && let Err(error) = self.persist() {
            eprintln!("failed to persist application state: {error}");
        }
    }

    fn should_persist_position(&self) -> bool {
        self.last_saved_track != self.facade.playback.current_track
            || self
                .facade
                .playback
                .position_ms
                .abs_diff(self.last_saved_position_ms)
                >= POSITION_SAVE_THRESHOLD_MS
    }

    fn refresh_save_baseline(&mut self) {
        self.last_saved_track = self.facade.playback.current_track;
        self.last_saved_position_ms = self.facade.playback.position_ms;
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::Application;
    use crate::app::commands::AppCommand;
    use crate::app::facade::AppFacade;
    use crate::core::ids::TrackId;
    use crate::core::library::TrackInput;
    use crate::core::persistence::{AppStateFile, JsonStore, PersistencePaths};

    fn temp_paths() -> PersistencePaths {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("lrc-player-app-test-{unique}"));
        PersistencePaths {
            config_json: root.join("config.json"),
            state_json: root.join("state.json"),
            playlists_json: root.join("playlists.json"),
        }
    }

    fn build_application() -> (Application, JsonStore) {
        let paths = temp_paths();
        let store = JsonStore::new(paths);
        let mut facade = AppFacade::default();
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "Track One".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: "/music/album/01-track-one.flac".to_string(),
                file_size: 1,
                modified_unix_secs: 10,
                file_name: "01-track-one.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );
        facade.queue.replace_with_tracks(&[TrackId(0)]);
        facade.queue.set_current_index(0);
        facade.playback.play_track(TrackId(0));

        (Application::with_store(facade, store.clone()), store)
    }

    #[test]
    fn pause_persists_when_position_threshold_is_met() {
        let (mut application, store) = build_application();
        application.facade_mut().playback.position_ms = 5_000;

        application.handle_command(AppCommand::TogglePlayPause);

        let state = store.load_state().expect("state should load");
        assert_eq!(state.playback_position_ms, 5_000);
    }

    #[test]
    fn seek_does_not_persist_when_below_threshold() {
        let (mut application, store) = build_application();
        application.facade_mut().playback.position_ms = 5_000;
        application.handle_command(AppCommand::TogglePlayPause);

        application.handle_command(AppCommand::SeekTo { position_ms: 6_000 });

        let state = store.load_state().expect("state should load");
        assert_eq!(state.playback_position_ms, 5_000);
    }

    #[test]
    fn seek_persists_when_threshold_is_met() {
        let (mut application, store) = build_application();
        application.facade_mut().playback.position_ms = 5_000;
        application.handle_command(AppCommand::TogglePlayPause);

        application.handle_command(AppCommand::SeekTo { position_ms: 9_000 });

        let state: AppStateFile = store.load_state().expect("state should load");
        assert_eq!(state.playback_position_ms, 9_000);
    }
}
