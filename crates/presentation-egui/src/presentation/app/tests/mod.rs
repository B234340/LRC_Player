//! Unit tests for GUI-side logic that does not need a window.

use crate::presentation::app::playback::{eq_from_dto, eq_to_dto, map_engine_state};
use crate::presentation::app::seek::hold_step_ms;
use lrc_player_application::dto::EqDto;
use lrc_player_application::infrastructure::audio::engine::EngineState;
use lrc_player_domain::model::PlaybackState;

#[test]
fn key_hold_scan_is_faster_than_real_time() {
    // PB-08: holding an arrow key must move the target forward faster than playback
    // (120 ms per step), never backward.
    let mut target = 0u64;
    let mut wall = 0u64;
    for step in 0..30 {
        target += hold_step_ms(step);
        wall += 120;
        assert!(target > wall);
    }
    assert_eq!(hold_step_ms(0), 2_000);
    assert_eq!(hold_step_ms(100), 10_000);
}

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

use crate::presentation::app::{GuiApp, RepeatMode, Tab};
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

#[test]
fn gui_app_smoke_transport_and_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.wav");
    let b = dir.path().join("b.wav");
    write_wav(&a, 1.0);
    write_wav(&b, 1.0);
    std::fs::write(dir.path().join("a.lrc"), "[00:00.10]line one\n[00:00.50]line two\n").unwrap();
    let a = a.to_string_lossy().into_owned();
    let b = b.to_string_lossy().into_owned();

    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = GuiApp::new(&cc);
    let mut frame = eframe::Frame::_new_kittest();

    app.set_shuffle(false);
    app.set_repeat(RepeatMode::Off);
    app.lrc_overlay_enabled = true;
    app.fft_overlay_enabled = true;
    app.play_now(vec![a.clone(), b.clone()]);
    assert_eq!(app.current_queue_index, Some(0));
    assert_eq!(app.play_path, a);

    // Pump frames until the engine accepted the track and the worker found lyrics/duration.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && (app.play_duration_ms.is_none() || app.loaded_lyrics.is_none()) {
        run_frame(&ctx, &mut app, &mut frame);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(app.play_duration_ms, Some(1000));
    assert!(app.loaded_lyrics.is_some(), "lyrics discovered on the worker");
    // Gapless successor follows the queue order (only queued while the engine holds the
    // track; without an output device the engine may report Stopped).
    let engine_active = app
        .engine_snapshot
        .as_ref()
        .is_some_and(|s| s.state != lrc_player_application::infrastructure::audio::engine::EngineState::Stopped);
    if engine_active {
        assert_eq!(app.pending_next, Some((1, b.clone())));
    }

    // Render every tab once (virtualized lists, artwork placeholders, overlays).
    for tab in [
        Tab::Play,
        Tab::Library,
        Tab::Playlist,
        Tab::Album,
        Tab::Artist,
        Tab::Queue,
        Tab::Scan,
        Tab::Settings,
    ] {
        app.active_tab = tab;
        run_frame(&ctx, &mut app, &mut frame);
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
