//! トラック単位の操作を提供するモジュール。
//!
//! - トラックの再生/キュー追加/詳細表示などの個別操作
//! - プレイリスト内トラックの追加・削除
//! - お気に入りトラック判定とお気に入りプレイリストの操作

use lrc_player_application::infrastructure::persistence::MetaCache;
use crate::presentation::components::track_list::TrackListOutcome;
use crate::presentation::app::{GuiApp, TrackDetailsDialog};
use std::path::Path;

/// 再生アクションを `TrackListOutcome` に登録する。
pub fn apply_play_action(outcome: &mut TrackListOutcome, index: usize, displayed_tracks: &[String]) {
    outcome.play_index = Some(index);
    outcome.play_list = displayed_tracks.to_vec();
}

/// お気に入りアクションを実行する。
pub fn apply_toggle_favorite_action(app: &mut GuiApp, track_path: &str) {
    perform_toggle_favorite_action(app, track_path);
}

/// キュー追加アクションを `TrackListOutcome` に登録する。
pub fn apply_enqueue_action(outcome: &mut TrackListOutcome, track_path: &str) {
    outcome.enqueue_path = Some(track_path.to_string());
}

/// 削除アクションを `TrackListOutcome` に登録する。
pub fn apply_remove_action(outcome: &mut TrackListOutcome, index: usize) {
    outcome.remove_indices = Some(vec![index]);
}

/// 詳細表示アクションを `TrackListOutcome` に登録する。
pub fn apply_show_details_action(outcome: &mut TrackListOutcome, track_path: &str) {
    outcome.show_details_path = Some(track_path.to_string());
}

/// トラックをお気に入りリストに追加/除外する。
pub fn perform_toggle_favorite_action(app: &mut GuiApp, track_path: &str) {
    if perform_is_track_favorite(app, track_path) {
        perform_remove_track_from_favorite_playlist(app, track_path);
    } else {
        perform_add_track_to_favorite_playlist(app, track_path);
    }
}

/// お気に入りトラック判定。
pub fn perform_is_track_favorite(app: &GuiApp, track_path: &str) -> bool {
    app.playlists
        .first()
        .map(|playlist| playlist.tracks.contains(&track_path.to_string()))
        .unwrap_or(false)
}

/// 現在のプレイリスト選択に入力されているトラックを追加する。
pub fn perform_add_track_to_selected_playlist(app: &mut GuiApp) {
    let track = app.playlist_track_input.trim().to_string();
    if track.is_empty() {
        return;
    }
    if let Some(index) = app.selected_playlist {
        if let Some(playlist) = app.playlists.get_mut(index) {
            if !playlist.tracks.contains(&track) {
                playlist.tracks.push(track.clone());
                app.playlist_track_input.clear();
                app.save_playlists();
                app.update_app_vm();
            }
        }
    }
}

/// 指定プレイリストにトラックを追加する。
pub fn perform_add_track_to_playlist(app: &mut GuiApp, playlist_index: usize, track: &str) {
    let item = track.trim();
    if item.is_empty() {
        return;
    }
    if let Some(playlist) = app.playlists.get_mut(playlist_index) {
        if !playlist.tracks.iter().any(|existing| existing == item) {
            playlist.tracks.push(item.to_string());
            app.save_playlists();
            app.update_app_vm();
        }
    }
}

/// トラック詳細ダイアログを開く。
pub fn perform_open_track_details(app: &mut GuiApp, path: &str) {
    let item = path.trim();
    if item.is_empty() {
        return;
    }

    let fallback_title = Path::new(item)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(item)
        .to_string();

    let (title, artist, album) = MetaCache::open_default()
        .ok()
        .and_then(|cache| cache.track_metadata_by_location(item).ok().flatten())
        .map(|(title, artist, album)| {
            let title = if title.trim().is_empty() {
                fallback_title.clone()
            } else {
                title
            };
            let artist = if artist.trim().is_empty() {
                "Unknown artist".to_string()
            } else {
                artist
            };
            let album = if album.trim().is_empty() {
                "Unknown album".to_string()
            } else {
                album
            };
            (title, artist, album)
        })
        .unwrap_or((fallback_title.clone(), "Unknown artist".to_string(), "Unknown album".to_string()));

    app.track_details_dialog = Some(TrackDetailsDialog {
        path: item.to_string(),
        title,
        artist,
        album,
    });
}

/// 選択中プレイリストからトラックを削除する。
pub fn perform_remove_track_from_selected_playlist(app: &mut GuiApp, track_index: usize) {
    if let Some(index) = app.selected_playlist {
        if let Some(playlist) = app.playlists.get_mut(index) {
            if track_index < playlist.tracks.len() {
                playlist.tracks.remove(track_index);
                app.save_playlists();
                app.update_app_vm();
            }
        }
    }
}

/// 指定トラックをお気に入りプレイリストに追加する。
pub fn perform_add_track_to_favorite_playlist(app: &mut GuiApp, track: &str) {
    let track = track.trim();
    if track.is_empty() {
        return;
    }
    let favorite_index = perform_ensure_favorite_playlist(app);
    if let Some(playlist) = app.playlists.get_mut(favorite_index) {
        if !playlist.tracks.contains(&track.to_string()) {
            playlist.tracks.push(track.to_string());
            app.save_playlists();
            app.update_app_vm();
        }
    }
}

/// 指定トラックをお気に入りプレイリストから削除する。
pub fn perform_remove_track_from_favorite_playlist(app: &mut GuiApp, track: &str) {
    if app.playlists.is_empty() {
        return;
    }
    let favorite_index = perform_ensure_favorite_playlist(app);
    if let Some(playlist) = app.playlists.get_mut(favorite_index) {
        if let Some(pos) = playlist.tracks.iter().position(|existing| existing == track) {
            playlist.tracks.remove(pos);
            app.save_playlists();
            app.update_app_vm();
        }
    }
}

/// 複数トラックをお気に入りプレイリストへ追加する。
pub fn perform_add_tracks_to_favorite_playlist(app: &mut GuiApp, tracks: &[String]) {
    if tracks.is_empty() {
        return;
    }

    let favorite_index = perform_ensure_favorite_playlist(app);
    let mut changed = false;
    if let Some(playlist) = app.playlists.get_mut(favorite_index) {
        for track in tracks {
            if !playlist.tracks.contains(track) {
                playlist.tracks.push(track.clone());
                changed = true;
            }
        }
    }

    if changed {
        app.save_playlists();
        app.update_app_vm();
    }
}

fn perform_ensure_favorite_playlist(app: &mut GuiApp) -> usize {
    if app.playlists.is_empty() || !app.playlists[0].name.eq_ignore_ascii_case("favorite") {
        app.playlists = GuiApp::normalize_playlists(std::mem::take(&mut app.playlists));
    }
    0
}
