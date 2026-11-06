//! Unit tests for GUI-side logic that does not need a window.

use crate::presentation::app::playback::{eq_from_dto, eq_to_dto, map_engine_state};
use lrc_player_application::dto::EqDto;
use lrc_player_application::infrastructure::audio::engine::EngineState;
use lrc_player_domain::model::PlaybackState;

#[test]
fn eq_dto_roundtrip() {
    let mut dto = EqDto {
        enabled: true,
        preamp_db: -3.0,
        ..Default::default()
    };
    dto.bands_db[9] = 4.5;
    assert_eq!(eq_to_dto(&eq_from_dto(&dto)), dto);
}

#[test]
fn engine_states_map_to_ui_states() {
    assert_eq!(map_engine_state(EngineState::Loading), PlaybackState::Buffering);
    assert_eq!(map_engine_state(EngineState::Playing), PlaybackState::Playing);
    assert_eq!(map_engine_state(EngineState::Paused), PlaybackState::Paused);
    assert_eq!(map_engine_state(EngineState::Stopped), PlaybackState::Stopped);
}

// ---------------------------------------------------------------------------------------
// Headless smoke test of the real GuiApp (egui context without a window). The engine is a
// real `AudioEngine`; without an output device it still accepts commands and reports
// `TrackStarted`, which is enough to exercise the transport / queue wiring.
// ---------------------------------------------------------------------------------------

use crate::presentation::app::{AppOptions, GuiApp, RepeatMode};
use crate::presentation::ui::{Page, RightPanel, UiAction};
use eframe::egui;
use std::path::Path;
use std::time::{Duration, Instant};

fn write_wav(path: &Path, seconds: f32) {
    let rate = 44_100u32;
    let frames = (rate as f32 * seconds) as u32;
    let mut data = Vec::with_capacity(frames as usize * 4);
    for i in 0..frames {
        let v = ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 8_000.0) as i16;
        data.extend_from_slice(&v.to_le_bytes());
        data.extend_from_slice(&v.to_le_bytes());
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::write(path, out).unwrap();
}

fn run_frame(ctx: &egui::Context, app: &mut GuiApp, frame: &mut eframe::Frame) {
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        eframe::App::ui(app, ui, frame);
    });
}

/// App with temp data: no persistence, a library DB in `dir`, real engine when `engine`.
pub(crate) fn test_app(ctx: &egui::Context, dir: &Path, engine: bool) -> GuiApp {
    GuiApp::with_options(
        ctx,
        AppOptions {
            persist: false,
            engine,
            meta_path: Some(dir.join("meta_cache.db")),
            load_fonts: false,
            media_controls: false,
            ..Default::default()
        },
    )
}

#[test]
fn gui_app_smoke_transport_and_pages() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.wav");
    let b = dir.path().join("b.wav");
    write_wav(&a, 1.0);
    write_wav(&b, 1.0);
    std::fs::write(dir.path().join("a.lrc"), "[00:00.10]line one\n[00:00.50]line two\n").unwrap();
    let a = a.to_string_lossy().into_owned();
    let b = b.to_string_lossy().into_owned();

    let ctx = egui::Context::default();
    let mut app = test_app(&ctx, dir.path(), true);
    let mut frame = eframe::Frame::_new_kittest();

    app.set_shuffle(false);
    app.set_repeat(RepeatMode::Off);
    app.play_now(vec![a.clone(), b.clone()]);
    assert_eq!(app.current_queue_index, Some(0));
    assert_eq!(app.play_path, a);

    // Pump frames until the engine accepted the track and the worker found lyrics/duration.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && (app.play_duration_ms.is_none() || app.loaded_timeline.is_none()) {
        run_frame(&ctx, &mut app, &mut frame);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(app.play_duration_ms, Some(1000));
    assert!(app.loaded_timeline.is_some(), "lyrics discovered on the worker");
    let engine_active = app
        .engine_snapshot
        .as_ref()
        .is_some_and(|s| s.state != lrc_player_application::infrastructure::audio::engine::EngineState::Stopped);
    if engine_active {
        assert_eq!(app.pending_next, Some((1, b.clone())));
    }

    // Render every page (and both right panels) once.
    for page in [Page::NowPlaying, Page::Library, Page::Albums, Page::Artists, Page::Playlists, Page::Folders, Page::Settings] {
        app.navigate(page);
        for panel in [None, Some(RightPanel::Queue), Some(RightPanel::Lyrics)] {
            app.ui.right_panel = panel;
            run_frame(&ctx, &mut app, &mut frame);
        }
    }

    // Next / Previous (restart threshold) / repeat.
    app.next();
    assert_eq!(app.current_queue_index, Some(1));
    assert_eq!(app.play_path, b);
    app.play_position_ms = 500;
    app.previous();
    assert_eq!(app.current_queue_index, Some(0));
    app.play_position_ms = 5_000;
    app.previous();
    assert_eq!(app.current_queue_index, Some(0), "after 3 s Previous restarts");
    assert_eq!(app.pending_seek_ms, Some(0));

    app.set_repeat(RepeatMode::One);
    app.update_gapless_next();
    app.cycle_repeat();
    assert_eq!(app.repeat(), RepeatMode::Off);

    app.set_volume(0.25);
    app.set_muted(false);
    app.toggle_mute();
    assert!(app.is_muted());
    let view = app.playback_view();
    assert_eq!(view.volume, 0.25);
    assert_eq!(view.queue_len, 2);

    app.stop();
    assert_eq!(app.play_position_ms, 0);
    app.shutdown();
}

#[test]
fn actions_edit_queue_playlists_and_lyrics() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = test_app(&ctx, dir.path(), false);
    let p = |n: &str| dir.path().join(n).to_string_lossy().into_owned();
    let (a, b, c) = (p("a.flac"), p("b.flac"), p("c.flac"));

    // Queue: play from the middle of a view, play next, reorder, remove.
    app.dispatch(&ctx, UiAction::PlayTracks { paths: vec![a.clone().into(), b.clone().into()], start: 1 });
    assert_eq!(app.queue, vec![a.clone(), b.clone()]);
    assert_eq!(app.current_queue_index, Some(1));
    assert_eq!(app.play_path, b);
    app.dispatch(&ctx, UiAction::PlayNext(vec![c.clone().into()]));
    assert_eq!(app.queue, vec![a.clone(), b.clone(), c.clone()]);
    app.dispatch(&ctx, UiAction::QueueReorder { from: 2, to: 0 });
    assert_eq!(app.queue, vec![c.clone(), a.clone(), b.clone()]);
    assert_eq!(app.current_queue_index, Some(2), "current entry follows the move");
    app.dispatch(&ctx, UiAction::QueueRemove(0));
    assert_eq!(app.queue, vec![a.clone(), b.clone()]);
    assert_eq!(app.current_queue_index, Some(1));

    // Save queue as playlist -> modal -> create.
    app.dispatch(&ctx, UiAction::QueueSaveAsPlaylist);
    let Some(super::Modal::CreatePlaylist { name, tracks }) = app.ui.modal.clone() else { panic!("modal") };
    assert_eq!(tracks.len(), 2);
    let id = app.create_playlist(&name, tracks).unwrap();
    app.add_to_playlist(&id, vec![c.clone()]);
    app.reorder_playlist(&id, 2, 0);
    let list = &app.playlists[app.playlist_index(&id).unwrap()];
    assert_eq!(list.tracks, vec![c.clone(), a.clone(), b.clone()]);
    app.remove_from_playlist(&id, vec![0, 2]);
    assert_eq!(app.playlists[app.playlist_index(&id).unwrap()].tracks, vec![a.clone()]);
    assert!(app.rename_playlist(&id, "favorite").is_err());
    app.rename_playlist(&id, "Night").unwrap();
    app.delete_playlist(&id);
    assert!(app.playlist_index(&id).is_none());
    assert!(!app.ui.toasts.is_empty(), "edits report through toasts");

    // Lyrics offsets are per track.
    app.dispatch(&ctx, UiAction::LyricOffsetDelta(300));
    assert_eq!(app.lyrics_offsets.get(&b), Some(&300));
    app.dispatch(&ctx, UiAction::LyricOffsetReset);
    assert!(!app.lyrics_offsets.contains_key(&b));

    // Navigation and chrome state.
    app.dispatch(&ctx, UiAction::OpenArtist("X".into()));
    assert_eq!(app.page(), Page::Artists);
    assert_eq!(app.ui.artist_detail.as_deref(), Some("X"));
    app.dispatch(&ctx, UiAction::Navigate(Page::Library));
    assert!(app.ui.artist_detail.is_none());
    app.dispatch(&ctx, UiAction::ToggleRightPanel(RightPanel::Lyrics));
    assert_eq!(app.ui.right_panel, Some(RightPanel::Lyrics));
    app.dispatch(&ctx, UiAction::ToggleRightPanel(RightPanel::Lyrics));
    assert_eq!(app.ui.right_panel, None);
    let state = app.build_state_dto();
    assert_eq!(state.last_tab.as_deref(), Some("library"));
    assert_eq!(super::state::page_from_key("Play"), Some(Page::NowPlaying));
}
