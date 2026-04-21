//! ライブラリおよびアーティスト選択に関する操作を提供するモジュール。
//!
//! - ライブラリのクリア
//! - アルバム/アーティスト/ディレクトリの選択
//! - ディレクトリ展開状態の管理
//! - お気に入りディレクトリの追加/削除

use crate::infrastructure::persistence::MetaCache;
use crate::presentation::app::{ArtistViewMode, GuiApp};

pub fn clear_library(app: &mut GuiApp) {
    if let Ok(cache) = MetaCache::open_default() {
        let _ = cache.clear_library();
    }
    app.clear_album_artwork_cache();
    app.selected_album = None;
    app.selected_artist = None;
    app.selected_directory = None;
    app.last_scan_status = Some("Library cleared".to_string());
    app.update_app_vm();
}

pub fn select_album(app: &mut GuiApp, album: Option<String>) {
    app.selected_album = album;
    app.selected_artist = None;
    app.selected_directory = None;
    app.update_app_vm();
}

pub fn select_artist(app: &mut GuiApp, artist: Option<String>) {
    app.selected_artist = artist;
    app.selected_album = None;
    app.selected_directory = None;
    app.update_app_vm();
}

pub fn set_artist_view_mode(app: &mut GuiApp, mode: ArtistViewMode) {
    if app.artist_view_mode == mode {
        return;
    }
    app.artist_view_mode = mode;
    app.selected_artist = None;
    app.update_app_vm();
    app.save_state();
}

pub fn select_directory(app: &mut GuiApp, directory: Option<String>) {
    app.selected_directory = directory;
    app.selected_album = None;
    app.selected_artist = None;
    app.update_app_vm();
}

pub fn toggle_library_directory_expanded(app: &mut GuiApp, path: &str) {
    if app.library_expanded_directories.contains(path) {
        app.library_expanded_directories.remove(path);
    } else {
        app.library_expanded_directories.insert(path.to_string());
    }
}

pub fn is_library_directory_expanded(app: &GuiApp, path: &str) -> bool {
    app.library_expanded_directories.contains(path)
}

pub fn add_favorite_directory(app: &mut GuiApp) {
    let dir = app.favorite_directory_input.trim().to_string();
    if dir.is_empty() || app.favorite_directories.contains(&dir) {
        return;
    }

    app.favorite_directories.push(dir);
    app.favorite_directory_input.clear();
    app.save_state();
    app.update_app_vm();
}

pub fn remove_favorite_directory(app: &mut GuiApp, index: usize) {
    if index < app.favorite_directories.len() {
        app.favorite_directories.remove(index);
        app.save_state();
        app.update_app_vm();
    }
}
