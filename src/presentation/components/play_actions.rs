use crate::domain::model::{PlaybackMode, PlaybackState};
use crate::presentation::egui_app::GuiApp;
use rand::seq::IteratorRandom;
use rand::thread_rng;
use std::time::Instant;

pub fn queue_seek_position(app: &mut GuiApp, position_ms: u64) {
    app.play_position_ms = position_ms;
    app.pending_seek_ms = Some(position_ms);
    app.seek_last_change_at = Some(Instant::now());
}

pub fn play_selected_queue_item(app: &mut GuiApp) {
    if let Some(index) = app.selected_queue_index {
        if let Some(path) = app.queue.tracks.get(index).cloned() {
            app.select_queue_item_single(index);
            app.current_queue_index = Some(index);
            app.play_path = path;
            app.play_position_ms = 0;
            app.playback_state = PlaybackState::Buffering;
            app.discover_lyrics_for_current_play_path();
            app.refresh_play_metadata();
            app.request_play();
        }
    }
}

pub fn seek_to_position(app: &mut GuiApp) {
    if app.play_path.trim().is_empty() {
        return;
    }
    if app.play_duration_ms.is_none() {
        app.refresh_play_metadata();
    }

    let target_ms = app.play_position_ms;
    match app.playback_state {
        PlaybackState::Stopped | PlaybackState::Paused => {
            app.play_status = Some(format!("Seek position updated: {} ms", target_ms));
        }
        _ => {
            app.playback_state_before_seek = Some(app.playback_state);
            app.playback_state = PlaybackState::Seeking;
            app.play_status = Some(format!("Seeking to {} ms", target_ms));
            app.request_play();
        }
    }
}

pub fn play_current_queue_item(app: &mut GuiApp) {
    let index = app
        .current_queue_index
        .or(app.selected_queue_index)
        .or(if app.queue.tracks.is_empty() { None } else { Some(0) });
    if let Some(index) = index {
        if let Some(path) = app.queue.tracks.get(index).cloned() {
            app.select_queue_item_single(index);
            app.current_queue_index = Some(index);
            app.play_path = path;
            app.playback_state = PlaybackState::Buffering;
            app.discover_lyrics_for_current_play_path();
            app.refresh_play_metadata();
            app.request_play();
        }
    }
}

pub fn play_next(app: &mut GuiApp) {
    let len = app.queue.tracks.len();
    if len == 0 {
        return;
    }
    let current = app
        .current_queue_index
        .or(app.selected_queue_index)
        .unwrap_or(0);

    let next_index = match app.playback_mode {
        PlaybackMode::Sequential => current.checked_add(1).filter(|&index| index < len),
        PlaybackMode::RepeatOne => Some(current),
        PlaybackMode::RepeatAll => {
            if current + 1 < len {
                Some(current + 1)
            } else {
                Some(0)
            }
        }
        PlaybackMode::Shuffle => {
            let mut rng = thread_rng();
            let choices: Vec<usize> = (0..len).filter(|&i| i != current).collect();
            if choices.is_empty() {
                Some(current)
            } else {
                choices.into_iter().choose(&mut rng)
            }
        }
    };

    if let Some(index) = next_index {
        app.select_queue_item_single(index);
        play_selected_queue_item(app);
    }
}

pub fn play_previous(app: &mut GuiApp) {
    let len = app.queue.tracks.len();
    if len == 0 {
        return;
    }
    let current = app
        .current_queue_index
        .or(app.selected_queue_index)
        .unwrap_or(0);

    let prev_index = match app.playback_mode {
        PlaybackMode::Sequential => current.checked_sub(1),
        PlaybackMode::RepeatOne => Some(current),
        PlaybackMode::RepeatAll => {
            if current == 0 {
                Some(len - 1)
            } else {
                Some(current - 1)
            }
        }
        PlaybackMode::Shuffle => {
            let mut rng = thread_rng();
            let choices: Vec<usize> = (0..len).filter(|&i| i != current).collect();
            if choices.is_empty() {
                Some(current)
            } else {
                choices.into_iter().choose(&mut rng)
            }
        }
    };

    if let Some(index) = prev_index {
        app.select_queue_item_single(index);
        play_selected_queue_item(app);
    }
}

pub fn toggle_play_pause(app: &mut GuiApp) {
    match app.playback_state {
        PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => {
            app.pause_playback();
        }
        PlaybackState::Paused | PlaybackState::Stopped => {
            if app.play_path.trim().is_empty() {
                if !app.queue.tracks.is_empty() {
                    app.play_current_queue_item();
                    return;
                }
                app.play_status = Some("No track loaded".to_string());
                return;
            }
            app.playback_state = PlaybackState::Buffering;
            app.play_status = Some("Resuming playback".to_string());
            app.request_play();
        }
    }
}
