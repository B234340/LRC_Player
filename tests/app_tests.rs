use std::time::{SystemTime, UNIX_EPOCH};

use lrc_player::app::Application;
use lrc_player::app::commands::AppCommand;
use lrc_player::app::facade::AppFacade;
use lrc_player::core::ids::TrackId;
use lrc_player::core::library::TrackInput;
use lrc_player::core::persistence::{AppStateFile, JsonStore, PersistencePaths};

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
