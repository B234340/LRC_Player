//! トラック単位の操作を提供するモジュール。
//!
//! - トラックの再生/キュー追加/詳細表示などの個別操作
//! - プレイリスト内トラックの追加・削除
//! - お気に入りトラック判定とお気に入りプレイリストの操作

use crate::presentation::components::track_list::TrackListOutcome;
use crate::presentation::app::{GuiApp, TrackDetailsDialog};
use std::path::Path;

/// 再生アクションを `TrackListOutcome` に登録する。
/// 
/// トラックリストコンポーネントからの再生要求を、後続処理のために
/// `TrackListOutcome` 構造体に記録します。
/// 
/// # 引数
/// * `outcome` - トラックリスト操作結果を格納する構造体への可変参照
/// * `index` - 再生するトラックのリスト内インデックス
/// * `displayed_tracks` - 表示中のトラックリスト全体
pub fn apply_play_action(outcome: &mut TrackListOutcome, index: usize, displayed_tracks: &[String]) {
    outcome.play_index = Some(index);
    outcome.play_list = displayed_tracks.to_vec();
}

/// お気に入りアクションを実行する。
/// 
/// 指定されたトラックのお気に入り状態をトグル（切り替え）します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `track_path` - 対象トラックのファイルパス
pub fn apply_toggle_favorite_action(app: &mut GuiApp, track_path: &str) {
    perform_toggle_favorite_action(app, track_path);
}

/// キュー追加アクションを `TrackListOutcome` に登録する。
/// 
/// トラックリストコンポーネントからのキュー追加要求を記録します。
/// 
/// # 引数
/// * `outcome` - トラックリスト操作結果を格納する構造体への可変参照
/// * `track_path` - キューに追加するトラックのファイルパス
pub fn apply_enqueue_action(outcome: &mut TrackListOutcome, track_path: &str) {
    outcome.enqueue_path = Some(track_path.to_string());
}

/// 削除アクションを `TrackListOutcome` に登録する。
/// 
/// トラックリストコンポーネントからの削除要求を記録します。
/// 複数削除に対応するため、インデックスはベクタで保持されます。
/// 
/// # 引数
/// * `outcome` - トラックリスト操作結果を格納する構造体への可変参照
/// * `index` - 削除するトラックのリスト内インデックス
pub fn apply_remove_action(outcome: &mut TrackListOutcome, index: usize) {
    outcome.remove_indices = Some(vec![index]);
}

/// 詳細表示アクションを `TrackListOutcome` に登録する。
/// 
/// トラックリストコンポーネントからの詳細表示要求を記録します。
/// 
/// # 引数
/// * `outcome` - トラックリスト操作結果を格納する構造体への可変参照
/// * `track_path` - 詳細を表示するトラックのファイルパス
pub fn apply_show_details_action(outcome: &mut TrackListOutcome, track_path: &str) {
    outcome.show_details_path = Some(track_path.to_string());
}

/// トラックをお気に入りリストに追加または除外する。
/// 
/// 現在のお気に入り状態に応じて、追加または削除を実行します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `track_path` - 対象トラックのファイルパス
pub fn perform_toggle_favorite_action(app: &mut GuiApp, track_path: &str) {
    // 現在のお気に入り状態を確認
    if perform_is_track_favorite(app, track_path) {
        // お気に入り済み → 削除
        perform_remove_track_from_favorite_playlist(app, track_path);
    } else {
        // 未お気に入り → 追加
        perform_add_track_to_favorite_playlist(app, track_path);
    }
}

/// 指定されたトラックがお気に入りに登録されているかを判定する。
/// 
/// プレイリストの先頭（インデックス0）はお気に入りプレイリストとして扱われます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への参照
/// * `track_path` - 判定するトラックのファイルパス
/// 
/// # 戻り値
/// * `true` - お気に入りに登録されている
/// * `false` - お気に入りに登録されていない、またはお気に入りプレイリストが存在しない
pub fn perform_is_track_favorite(app: &GuiApp, track_path: &str) -> bool {
    // O(1): set rebuilt whenever playlists are saved / reloaded.
    app.favorite_set.contains(track_path)
}

/// 現在選択中のプレイリストに、入力フィールドのトラックを追加する。
/// 
/// 入力が空の場合や、既にプレイリストに存在するトラックの場合は何もしません。
/// 追加後は入力フィールドをクリアし、保存とUI更新を行います。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn perform_add_track_to_selected_playlist(app: &mut GuiApp) {
    // 入力文字列をトリムして取得
    let track = app.playlist_track_input.trim().to_string();
    // 空の場合は何もしない
    if track.is_empty() {
        return;
    }
    
    // 選択中のプレイリストを取得
    if let Some(index) = app.selected_playlist {
        if let Some(playlist) = app.playlists.get_mut(index) {
            // 重複チェック
            if !playlist.tracks.contains(&track) {
                playlist.tracks.push(track.clone());
                // 入力フィールドをクリア
                app.playlist_track_input.clear();
                // 保存とUI更新
                app.save_playlists();
                app.update_app_vm();
            }
        }
    }
}

/// 指定されたプレイリストにトラックを追加する。
/// 
/// 空文字列や重複するトラックは追加されません。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `playlist_index` - 追加先プレイリストのインデックス
/// * `track` - 追加するトラックのファイルパス
pub fn perform_add_track_to_playlist(app: &mut GuiApp, playlist_index: usize, track: &str) {
    let item = track.trim();
    // 空の場合は何もしない
    if item.is_empty() {
        return;
    }
    
    // 指定インデックスのプレイリストを取得
    if let Some(playlist) = app.playlists.get_mut(playlist_index) {
        // 重複チェック
        if !playlist.tracks.iter().any(|existing| existing == item) {
            playlist.tracks.push(item.to_string());
            app.save_playlists();
            app.update_app_vm();
        }
    }
}

/// トラック詳細ダイアログを開く。
/// 
/// 指定されたパスのメタデータをメタキャッシュから取得し、
/// 詳細表示ダイアログの情報を設定します。
/// メタデータが取得できない場合はファイル名からフォールバック情報を生成します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `path` - 詳細を表示するトラックのファイルパス
pub fn perform_open_track_details(app: &mut GuiApp, path: &str) {
    let item = path.trim();
    // 空パスは無視
    if item.is_empty() {
        return;
    }

    // フォールバック用のタイトル（ファイル名から取得）
    let fallback_title = Path::new(item)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(item)
        .to_string();

    // メタキャッシュからトラック情報を取得
    let (title, artist, album) = app
        .meta
        .as_ref()
        .and_then(|meta| meta.with(|cache| cache.track_metadata_by_location(item).ok().flatten()))
        .map(|(title, artist, album)| {
            // タイトルが空の場合はフォールバックを使用
            let title = if title.trim().is_empty() {
                fallback_title.clone()
            } else {
                title
            };
            // アーティストが空の場合は "Unknown artist"
            let artist = if artist.trim().is_empty() {
                "Unknown artist".to_string()
            } else {
                artist
            };
            // アルバムが空の場合は "Unknown album"
            let album = if album.trim().is_empty() {
                "Unknown album".to_string()
            } else {
                album
            };
            (title, artist, album)
        })
        // メタデータ取得失敗時はすべてフォールバック値を使用
        .unwrap_or((fallback_title.clone(), "Unknown artist".to_string(), "Unknown album".to_string()));

    // 詳細ダイアログ情報を設定
    app.track_details_dialog = Some(TrackDetailsDialog {
        path: item.to_string(),
        title,
        artist,
        album,
    });
}

/// 現在選択中のプレイリストから、指定インデックスのトラックを削除する。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `track_index` - 削除するトラックのプレイリスト内インデックス
pub fn perform_remove_track_from_selected_playlist(app: &mut GuiApp, track_index: usize) {
    // 選択中のプレイリストを取得
    if let Some(index) = app.selected_playlist {
        if let Some(playlist) = app.playlists.get_mut(index) {
            // インデックスが有効範囲内かチェック
            if track_index < playlist.tracks.len() {
                playlist.tracks.remove(track_index);
                app.save_playlists();
                app.update_app_vm();
            }
        }
    }
}

/// 指定されたトラックをお気に入りプレイリストに追加する。
/// 
/// お気に入りプレイリストが存在しない場合は自動的に作成されます。
/// 既に登録済みのトラックは追加されません（重複防止）。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `track` - 追加するトラックのファイルパス
pub fn perform_add_track_to_favorite_playlist(app: &mut GuiApp, track: &str) {
    let track = track.trim();
    // 空文字列は無視
    if track.is_empty() {
        return;
    }
    
    // お気に入りプレイリストの存在を保証し、インデックスを取得
    let favorite_index = perform_ensure_favorite_playlist(app);
    
    // お気に入りプレイリストにトラックを追加
    if let Some(playlist) = app.playlists.get_mut(favorite_index) {
        // 重複チェック
        if !playlist.tracks.contains(&track.to_string()) {
            playlist.tracks.push(track.to_string());
            app.save_playlists();
            app.update_app_vm();
        }
    }
}

/// 指定されたトラックをお気に入りプレイリストから削除する。
/// 
/// お気に入りプレイリストが存在しない場合は何もしません。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `track` - 削除するトラックのファイルパス
pub fn perform_remove_track_from_favorite_playlist(app: &mut GuiApp, track: &str) {
    // プレイリストが空の場合は何もしない
    if app.playlists.is_empty() {
        return;
    }
    
    // お気に入りプレイリストの存在を保証し、インデックスを取得
    let favorite_index = perform_ensure_favorite_playlist(app);
    
    // お気に入りプレイリストからトラックを削除
    if let Some(playlist) = app.playlists.get_mut(favorite_index) {
        // トラックの位置を検索
        if let Some(pos) = playlist.tracks.iter().position(|existing| existing == track) {
            playlist.tracks.remove(pos);
            app.save_playlists();
            app.update_app_vm();
        }
    }
}

/// 複数のトラックをまとめてお気に入りプレイリストに追加する。
/// 
/// お気に入りプレイリストが存在しない場合は自動的に作成されます。
/// 既に登録済みのトラックはスキップされます（重複防止）。
/// 一つ以上のトラックが追加された場合のみ保存とUI更新を行います。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `tracks` - 追加するトラックのファイルパスのスライス
pub fn perform_add_tracks_to_favorite_playlist(app: &mut GuiApp, tracks: &[String]) {
    // 空リストの場合は何もしない
    if tracks.is_empty() {
        return;
    }

    // お気に入りプレイリストの存在を保証し、インデックスを取得
    let favorite_index = perform_ensure_favorite_playlist(app);
    let mut changed = false;
    
    // 各トラックを追加
    if let Some(playlist) = app.playlists.get_mut(favorite_index) {
        for track in tracks {
            // 重複チェック
            if !playlist.tracks.contains(track) {
                playlist.tracks.push(track.clone());
                changed = true;
            }
        }
    }

    // 変更があった場合のみ保存とUI更新
    if changed {
        app.save_playlists();
        app.update_app_vm();
    }
}

/// お気に入りプレイリストの存在を保証し、そのインデックスを返す内部関数。
/// 
/// プレイリスト一覧が空の場合、または先頭がお気に入りプレイリストでない場合は、
/// プレイリスト一覧を正規化してお気に入りプレイリストを先頭に配置します。
/// お気に入りプレイリストは常にインデックス0に配置されることが保証されます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// 
/// # 戻り値
/// お気に入りプレイリストのインデックス（常に0）
fn perform_ensure_favorite_playlist(app: &mut GuiApp) -> usize {
    // プレイリストが空、または先頭が "favorite" でない場合は正規化
    if app.playlists.is_empty() || !app.playlists[0].name.eq_ignore_ascii_case("favorite") {
        app.playlists = GuiApp::normalize_playlists(std::mem::take(&mut app.playlists));
    }
    // お気に入りプレイリストは常に先頭（インデックス0）
    0
}