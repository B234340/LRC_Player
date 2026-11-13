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

#[test]
fn favorite_toggles_edit_the_favorites_list() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = test_app(&ctx, dir.path(), false);
    let p = |n: &str| dir.path().join(n).to_string_lossy().into_owned();
    let (a, b, c) = (p("a.flac"), p("b.flac"), p("c.flac"));
    let fav_tracks = |app: &GuiApp| {
        app.playlists.iter().find(|l| super::playlists::is_favorite(l)).unwrap().tracks.clone()
    };

    // Toggle the playing track (transport bar / Now Playing heart).
    app.dispatch(&ctx, UiAction::PlayTracks { paths: vec![a.clone().into()], start: 0 });
    app.dispatch(&ctx, UiAction::ToggleFavoriteCurrent);
    assert!(app.is_favorite_path(&a));
    assert_eq!(fav_tracks(&app), vec![a.clone()]);
    let toasts_before = app.ui.toasts.len();
    app.dispatch(&ctx, UiAction::ToggleFavoriteCurrent);
    assert!(!app.is_favorite_path(&a));
    assert!(fav_tracks(&app).is_empty());
    assert_eq!(app.ui.toasts.len(), toasts_before, "single toggles are silent");

    // Several tracks from a table menu: no duplicates, toast, then removal.
    app.dispatch(&ctx, UiAction::SetFavorite { paths: vec![b.clone().into()], favorite: true });
    app.dispatch(&ctx, UiAction::SetFavorite { paths: vec![a.clone().into(), b.clone().into(), c.clone().into()], favorite: true });
    assert_eq!(fav_tracks(&app), vec![b.clone(), a.clone(), c.clone()]);
    assert!(app.ui.toasts.len() > toasts_before);
    let lookup = super::playlists::favorite_lookup(&app.favorites);
    assert!(lookup(Path::new(&c)));
    drop(lookup);
    app.dispatch(&ctx, UiAction::SetFavorite { paths: vec![a.clone().into(), c.clone().into()], favorite: false });
    assert_eq!(fav_tracks(&app), vec![b.clone()]);
    assert!(app.is_favorite_path(&b) && !app.is_favorite_path(&c));

    // Nothing playing: toggling the current track does nothing.
    app.play_path.clear();
    app.dispatch(&ctx, UiAction::ToggleFavoriteCurrent);
    assert_eq!(fav_tracks(&app), vec![b]);
}

#[test]
fn lyric_text_shift_is_applied_persisted_and_saved() {
    use lrc_player_application::infrastructure::lyrics::{lrc_backup_path, Lyrics, LyricsTimeline};

    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = test_app(&ctx, dir.path(), false);
    app.play_path = dir.path().join("song.flac").to_string_lossy().into_owned();
    let lrc = dir.path().join("song.lrc");
    // Text one line late: the first timestamp is empty, every line sits one slot too late.
    std::fs::write(
        &lrc,
        "[00:01.00]\n[00:03.00]<00:03.00>あ<00:04.00>い<00:05.00>\n[00:06.00]<00:06.00>う<00:07.00>え<00:08.00>\n[00:09.00]end\n",
    )
    .unwrap();
    app.set_lyrics_candidates(Lyrics::rank_candidates([lrc.clone()]));
    let texts = |app: &GuiApp| -> Vec<String> {
        app.loaded_timeline.as_ref().unwrap().lines().iter().map(|l| l.text.clone()).collect()
    };
    let original = texts(&app);
    assert_eq!(original, vec!["", "あい", "うえ", "end"]);

    // Line shift: text moves one line earlier, timestamps stay.
    let rev = app.lyrics_revision;
    app.dispatch(&ctx, UiAction::LyricShiftLines(-1));
    assert_eq!(texts(&app), vec!["あい", "うえ", "end", ""]);
    assert_eq!(app.loaded_timeline.as_ref().unwrap().lines()[0].start_ms, 1_000);
    assert!(app.lyrics_revision > rev, "views re-layout the shifted text");
    assert_eq!(app.lyrics_shifts.get(&app.play_path).map(|s| s.lines), Some(-1));

    // Clamped to the number of lines; reset restores the original.
    app.dispatch(&ctx, UiAction::LyricShiftLines(-100));
    assert_eq!(app.lyrics_shift.lines, -3);
    app.dispatch(&ctx, UiAction::LyricShiftReset);
    assert_eq!(texts(&app), original);
    assert!(app.lyrics_shifts.is_empty());

    // Character shift over the karaoke characters (across lines).
    app.dispatch(&ctx, UiAction::LyricShiftChars(1));
    assert_eq!(texts(&app), vec!["", "あ", "いうえ", "end"]);
    app.dispatch(&ctx, UiAction::LyricShiftChars(-1));
    assert_eq!(texts(&app), original);

    // Saving writes the shifted lyrics, keeps a .bak, and resets the shift.
    app.dispatch(&ctx, UiAction::LyricShiftLines(-1));
    let shown = texts(&app);
    app.dispatch(&ctx, UiAction::SaveLyricOffset);
    assert!(lrc_backup_path(&lrc).is_file());
    assert_eq!(app.lyrics_shift, Default::default());
    assert!(app.lyrics_shifts.is_empty());
    assert_eq!(texts(&app), shown, "same display after saving");
    let reread = LyricsTimeline::from_lrc(&lrc).unwrap();
    let file_texts: Vec<String> = reread.lines().iter().map(|l| l.text.clone()).collect();
    assert_eq!(file_texts, shown);
    assert!(reread.lines()[0].is_karaoke(), "karaoke timing survives the rewrite");
}

#[test]
fn estimated_karaoke_is_display_only() {
    use lrc_player_application::infrastructure::lyrics::{Lyrics, LyricsTimeline};

    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = test_app(&ctx, dir.path(), false);
    app.play_path = dir.path().join("song.flac").to_string_lossy().into_owned();
    let lrc = dir.path().join("song.lrc");
    std::fs::write(&lrc, "[00:01.00]夜に駆ける\n[00:05.00]沈むように\n[00:09.00]end\n").unwrap();
    app.set_lyrics_candidates(Lyrics::rank_candidates([lrc.clone()]));
    let estimated = |app: &GuiApp| app.loaded_timeline.as_ref().unwrap().lines().iter().filter(|l| l.estimated).count();

    // On by default: line-based lines get an estimated wipe.
    assert!(app.config.estimate_karaoke);
    assert_eq!(estimated(&app), 3);
    let pos = app.loaded_timeline.as_ref().unwrap().position_at(1_500).unwrap();
    assert!(pos.highlighted_chars > 0.0 && pos.highlighted_chars < 5.0, "{}", pos.highlighted_chars);
    // No character shift on estimated timing.
    assert_eq!(app.loaded_timeline.as_ref().unwrap().karaoke_char_count(), 0);

    // Turning it off (or karaoke off) shows plain lines again.
    app.config.estimate_karaoke = false;
    app.reload_lyrics_display();
    assert_eq!(estimated(&app), 0);
    app.config.estimate_karaoke = true;
    app.config.karaoke = false;
    app.reload_lyrics_display();
    assert_eq!(estimated(&app), 0);
    app.config.karaoke = true;
    app.reload_lyrics_display();

    // Saving (with a shift) never writes the estimated timing.
    app.dispatch(&ctx, UiAction::LyricShiftLines(-1));
    app.dispatch(&ctx, UiAction::SaveLyricOffset);
    let saved = std::fs::read_to_string(&lrc).unwrap();
    assert!(!saved.contains('<'), "{saved}");
    assert!(LyricsTimeline::parse(&saved).lines().iter().all(|l| !l.is_karaoke()));
}
