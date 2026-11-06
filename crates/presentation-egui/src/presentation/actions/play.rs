//! 再生/シークに関する操作を提供するモジュール。
//!
//! Thin wrappers over the engine-backed control API on [`GuiApp`]
//! (see `presentation/app/playback.rs`).

use crate::presentation::app::GuiApp;

/// Seek (debounced) to `position_ms`.
pub fn queue_seek_position(app: &mut GuiApp, position_ms: u64) {
    app.seek_to(position_ms);
}

/// Plays the selected queue item.
pub fn play_selected_queue_item(app: &mut GuiApp) {
    if let Some(index) = app.selected_queue_index {
        if index < app.queue.tracks.len() {
            app.play_queue_index(index);
        }
    }
}

/// Plays `path` right away: it is inserted after the current queue item (or appended) so
/// Next/Previous keep working from there.
pub fn perform_play_track_now(app: &mut GuiApp, path: &str) {
    let path = path.trim();
    if path.is_empty() {
        return;
    }
    let insert_at = app
        .current_queue_index
        .map(|i| (i + 1).min(app.queue.tracks.len()))
        .unwrap_or(app.queue.tracks.len());
    app.queue.tracks.insert(insert_at, path.to_string());
    app.navigator.invalidate();
    app.update_app_vm();
    app.play_queue_index(insert_at);
}

/// Plays the current (or selected, or first) queue item.
pub fn play_current_queue_item(app: &mut GuiApp) {
    let len = app.queue.tracks.len();
    if len == 0 {
        return;
    }
    let index = app
        .current_queue_index
        .filter(|&i| i < len)
        .or(app.selected_queue_index.filter(|&i| i < len))
        .unwrap_or(0);
    app.play_queue_index(index);
}

pub fn play_next(app: &mut GuiApp) {
    app.next();
}

pub fn play_previous(app: &mut GuiApp) {
    app.previous();
}

pub fn pause_playback(app: &mut GuiApp) {
    app.pause();
}

pub fn toggle_play_pause(app: &mut GuiApp) {
    app.toggle_play_pause();
}
