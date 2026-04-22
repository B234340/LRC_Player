//! キュー管理に関する操作を提供するモジュール。
//!
//! - キュー内の選択状態管理
//! - キュー項目の移動/削除
//! - キューへのトラック挿入
//! - 再生中のキュー状態調整

use lrc_player_domain::model::PlaybackState;
use crate::presentation::actions::play;
use crate::presentation::app::GuiApp;

pub fn add_queue_item(app: &mut GuiApp) {
    let item = app.queue_item_input.trim().to_string();
    if item.is_empty() {
        return;
    }
    app.queue.tracks.push(item);
    app.queue_item_input.clear();
    app.update_app_vm();
}

pub fn clear_queue_selection(app: &mut GuiApp) {
    app.selected_queue_index = None;
    app.selected_queue_indices.clear();
    app.queue_selection_anchor = None;
}

pub fn select_queue_item_single(app: &mut GuiApp, index: usize) {
    app.selected_queue_index = Some(index);
    app.selected_queue_indices.clear();
    app.selected_queue_indices.push(index);
    app.queue_selection_anchor = Some(index);
}

/// 指定トラックをキュー末尾に追加する。
pub fn perform_enqueue_track_path(app: &mut GuiApp, path: &str) {
    let item = path.trim();
    if item.is_empty() {
        return;
    }
    app.queue.tracks.push(item.to_string());
    app.update_app_vm();
}

/// 次に再生する位置にトラックを挿入する。
pub fn perform_add_track_next_in_queue(app: &mut GuiApp, track: &str) {
    let item = track.trim();
    if item.is_empty() {
        return;
    }

    let insert_at = app
        .current_queue_index
        .or(app.selected_queue_index)
        .map(|index| (index + 1).min(app.queue.tracks.len()))
        .unwrap_or(app.queue.tracks.len());

    app.queue.tracks.insert(insert_at, item.to_string());
    app.update_app_vm();
}

/// 表示中のリストを再生キューに設定し、指定トラックを選択して再生開始する。
pub fn perform_play_from_displayed_list(
    app: &mut GuiApp,
    displayed_tracks: &[String],
    clicked_index: usize,
) {
    if displayed_tracks.is_empty() || clicked_index >= displayed_tracks.len() {
        return;
    }

    app.queue.tracks = displayed_tracks.to_vec();
    app.current_queue_index = None;
    clear_queue_selection(app);
    select_queue_item_single(app, clicked_index);
    app.update_app_vm();
    play::play_selected_queue_item(app);
}

pub fn toggle_queue_item_selection(app: &mut GuiApp, index: usize) {
    if let Some(pos) = app.selected_queue_indices.iter().position(|&i| i == index) {
        app.selected_queue_indices.remove(pos);
        app.selected_queue_index = app.selected_queue_indices.last().copied();
        if app.selected_queue_indices.is_empty() {
            app.queue_selection_anchor = None;
        }
    } else {
        app.selected_queue_indices.push(index);
        app.selected_queue_index = Some(index);
        if app.queue_selection_anchor.is_none() {
            app.queue_selection_anchor = Some(index);
        }
    }
}

pub fn select_queue_item_range_to(app: &mut GuiApp, index: usize) {
    let anchor = app
        .queue_selection_anchor
        .or(app.selected_queue_index)
        .unwrap_or(index);
    let (start, end) = if anchor <= index {
        (anchor, index)
    } else {
        (index, anchor)
    };
    app.selected_queue_indices = (start..=end).collect();
    app.selected_queue_index = Some(index);
    app.queue_selection_anchor = Some(anchor);
}

pub fn remove_selected_queue_items(app: &mut GuiApp) {
    if app.selected_queue_indices.is_empty() {
        if let Some(index) = app.selected_queue_index {
            remove_queue_item(app, index);
        }
        return;
    }

    let current_index = app.current_queue_index;
    let mut indices = app.selected_queue_indices.clone();
    indices.sort_unstable();
    indices.dedup();
    let current_removed = current_index
        .map(|current| indices.contains(&current))
        .unwrap_or(false);

    indices.reverse();
    for index in indices.iter().copied() {
        if index < app.queue.tracks.len() {
            app.queue.tracks.remove(index);
        }
    }

    let removed_before_current = current_index.map(|old_current| {
        indices.iter().filter(|&&removed| removed < old_current).count()
    });

    clear_queue_selection(app);

    if current_removed {
        if app.queue.tracks.is_empty() {
            clear_queue(app);
            stop_playback_and_clear_play_tab(app);
        } else if let Some(old_index) = current_index {
            let next_index = if old_index < app.queue.tracks.len() {
                old_index
            } else {
                app.queue.tracks.len() - 1
            };
            select_queue_item_single(app, next_index);
            match app.playback_state {
                PlaybackState::Paused => {
                    load_selected_queue_item_paused(app, next_index);
                }
                PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => {
                    play::play_selected_queue_item(app);
                }
                PlaybackState::Stopped => {
                    load_selected_queue_item_stopped(app, next_index);
                }
            }
        }
    } else if let Some(old_current) = current_index {
        let shift = removed_before_current.unwrap_or(0);
        app.current_queue_index = Some(old_current.saturating_sub(shift));
    }

    app.update_app_vm();
}

pub fn clear_queue_handle_hold(app: &mut GuiApp) {
    app.queue_handle_hold_index = None;
    app.queue_handle_hold_started_at = None;
}

pub fn cancel_queue_drag(app: &mut GuiApp) {
    app.queue_drag_source_index = None;
    clear_queue_handle_hold(app);
}

pub fn move_queue_item_to(app: &mut GuiApp, from: usize, to: usize) {
    if from >= app.queue.tracks.len() || to >= app.queue.tracks.len() || from == to {
        return;
    }

    let item = app.queue.tracks.remove(from);
    let insert_at = if to > from { to - 1 } else { to };
    app.queue.tracks.insert(insert_at, item);

    let remap_index = |idx: usize| {
        if idx == from {
            insert_at
        } else if from < insert_at {
            if idx > from && idx <= insert_at {
                idx - 1
            } else {
                idx
            }
        } else if insert_at < from {
            if idx >= insert_at && idx < from {
                idx + 1
            } else {
                idx
            }
        } else {
            idx
        }
    };

    app.selected_queue_indices = app
        .selected_queue_indices
        .iter()
        .copied()
        .map(remap_index)
        .collect();
    app.selected_queue_indices.sort_unstable();
    app.selected_queue_indices.dedup();

    app.selected_queue_index = app.selected_queue_index.map(remap_index);
    app.queue_selection_anchor = app.queue_selection_anchor.map(remap_index);
    app.current_queue_index = app.current_queue_index.map(remap_index);

    clear_queue_selection(app);
    app.update_app_vm();
}

pub fn remove_queue_item(app: &mut GuiApp, index: usize) {
    if index < app.queue.tracks.len() {
        let current_removed = app.current_queue_index == Some(index);
        app.queue.tracks.remove(index);
        app.selected_queue_indices = app
            .selected_queue_indices
            .iter()
            .filter_map(|&i| {
                if i == index {
                    None
                } else if i > index {
                    Some(i - 1)
                } else {
                    Some(i)
                }
            })
            .collect();
        if app.selected_queue_index == Some(index) {
            app.selected_queue_index = app.selected_queue_indices.last().copied();
        } else if let Some(selected) = app.selected_queue_index {
            if selected > index {
                app.selected_queue_index = Some(selected - 1);
            }
        }
        if let Some(anchor) = app.queue_selection_anchor {
            app.queue_selection_anchor = if anchor == index {
                app.selected_queue_index
            } else if anchor > index {
                Some(anchor - 1)
            } else {
                Some(anchor)
            };
        }

        if current_removed {
            if app.queue.tracks.is_empty() {
                clear_queue(app);
                stop_playback_and_clear_play_tab(app);
            } else {
                let next_index = if index < app.queue.tracks.len() {
                    index
                } else {
                    app.queue.tracks.len() - 1
                };
                select_queue_item_single(app, next_index);
                match app.playback_state {
                    PlaybackState::Paused => {
                        load_selected_queue_item_paused(app, next_index);
                    }
                    PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => {
                        play::play_selected_queue_item(app);
                    }
                    PlaybackState::Stopped => {
                        load_selected_queue_item_stopped(app, next_index);
                    }
                }
            }
        } else {
            app.current_queue_index = if let Some(current) = app.current_queue_index {
                if current > index {
                    Some(current - 1)
                } else {
                    Some(current)
                }
            } else {
                None
            };
        }

        app.update_app_vm();
    }
}

pub fn move_queue_item_up(app: &mut GuiApp, index: usize) {
    if index == 0 || index >= app.queue.tracks.len() {
        return;
    }
    app.queue.tracks.swap(index, index - 1);
    select_queue_item_single(app, index - 1);
    app.update_app_vm();
}

pub fn move_queue_item_down(app: &mut GuiApp, index: usize) {
    if index + 1 >= app.queue.tracks.len() {
        return;
    }
    app.queue.tracks.swap(index, index + 1);
    select_queue_item_single(app, index + 1);
    app.update_app_vm();
}

pub fn clear_queue(app: &mut GuiApp) {
    app.queue.tracks.clear();
    clear_queue_selection(app);
    cancel_queue_drag(app);
    app.current_queue_index = None;
    app.update_app_vm();
}

fn stop_playback_and_clear_play_tab(app: &mut GuiApp) {
    app.signal_playback_stop();
    app.playback_state = PlaybackState::Stopped;
    app.play_status = Some("Playback stopped".to_string());
    app.play_path.clear();
    app.play_position_ms = 0;
    app.play_duration_ms = None;
    app.play_title = None;
    app.play_artist = None;
    app.play_album = None;
    app.play_artwork_handle = None;
    app.play_artwork_source = None;
    app.lyrics_files.clear();
    app.loaded_lyrics = None;
    app.selected_lyrics_index = None;
    app.lyrics_offset_ms = 0;
    app.playback_start_instant = None;
    app.pending_play_request = false;
}

fn load_selected_queue_item_paused(app: &mut GuiApp, index: usize) {
    if let Some(path) = app.queue.tracks.get(index).cloned() {
        select_queue_item_single(app, index);
        app.current_queue_index = Some(index);
        app.play_path = path;
        app.play_position_ms = 0;
        app.playback_state = PlaybackState::Paused;
        app.play_status = Some("Paused at start of selected track".to_string());
        app.discover_lyrics_for_current_play_path();
        app.refresh_play_metadata();
        app.playback_start_instant = None;
        app.pending_play_request = false;
    }
}

fn load_selected_queue_item_stopped(app: &mut GuiApp, index: usize) {
    if let Some(path) = app.queue.tracks.get(index).cloned() {
        select_queue_item_single(app, index);
        app.current_queue_index = Some(index);
        app.play_path = path;
        app.play_position_ms = 0;
        app.playback_state = PlaybackState::Stopped;
        app.play_status = Some("Stopped at start of selected track".to_string());
        app.discover_lyrics_for_current_play_path();
        app.refresh_play_metadata();
        app.playback_start_instant = None;
        app.pending_play_request = false;
    }
}
