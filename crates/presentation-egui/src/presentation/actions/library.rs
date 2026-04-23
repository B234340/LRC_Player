//! ライブラリおよびアーティスト選択に関する操作を提供するモジュール。
//!
//! - ライブラリのクリア
//! - アルバム/アーティスト/ディレクトリの選択
//! - ディレクトリ展開状態の管理
//! - お気に入りディレクトリの追加/削除

use lrc_player_application::infrastructure::persistence::MetaCache;
use crate::presentation::app::{ArtistViewMode, GuiApp};

/// ライブラリ全体をクリアします。
/// 
/// メタデータキャッシュを削除し、アプリケーション内の選択状態をリセットします。
/// アルバムアートワークのキャッシュも同時にクリアされます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn clear_library(app: &mut GuiApp) {
    // メタデータキャッシュを開き、ライブラリをクリア
    if let Ok(cache) = MetaCache::open_default() {
        if let Err(err) = cache.clear_library() {
            app.last_scan_status = Some(format!("Failed to clear library: {}", err));
            app.update_app_vm();
            return;
        }
    } else {
        app.last_scan_status = Some("Failed to open meta cache for clear library".to_string());
        app.update_app_vm();
        return;
    }
    // メモリ上のアルバムアートワークキャッシュをクリア
    app.clear_album_artwork_cache();
    // 選択状態をすべてリセット
    app.selected_album = None;
    app.selected_artist = None;
    app.selected_directory = None;
    // ステータスメッセージを更新
    app.last_scan_status = Some("Library cleared".to_string());
    // ViewModel を更新して UI に反映
    app.update_app_vm();
}

/// 指定されたアルバムを選択状態にします。
/// 
/// アルバムを選択すると、アーティスト選択とディレクトリ選択は解除されます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `album` - 選択するアルバム名（`None` の場合は選択解除）
pub fn select_album(app: &mut GuiApp, album: Option<String>) {
    app.selected_album = album;
    // アルバム選択時は、他の選択モードをクリア
    app.selected_artist = None;
    app.selected_directory = None;
    // UI を更新
    app.update_app_vm();
}

/// 指定されたアーティストを選択状態にします。
/// 
/// アーティストを選択すると、アルバム選択とディレクトリ選択は解除されます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `artist` - 選択するアーティスト名（`None` の場合は選択解除）
pub fn select_artist(app: &mut GuiApp, artist: Option<String>) {
    app.selected_artist = artist;
    // アーティスト選択時は、他の選択モードをクリア
    app.selected_album = None;
    app.selected_directory = None;
    // UI を更新
    app.update_app_vm();
}

/// アーティストビューの表示モードを設定します。
/// 
/// 同じモードが指定された場合は何も行いません。
/// モード変更時にはアーティスト選択がリセットされ、状態が保存されます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `mode` - 設定する表示モード
pub fn set_artist_view_mode(app: &mut GuiApp, mode: ArtistViewMode) {
    // 同じモードの場合は早期リターン
    if app.artist_view_mode == mode {
        return;
    }
    // モードを更新
    app.artist_view_mode = mode;
    // モード変更に伴いアーティスト選択をクリア
    app.selected_artist = None;
    // UI を更新
    app.update_app_vm();
    // 設定を永続化
    app.save_state();
}

/// 指定されたディレクトリを選択状態にします。
/// 
/// ディレクトリを選択すると、アルバム選択とアーティスト選択は解除されます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `directory` - 選択するディレクトリパス（`None` の場合は選択解除）
pub fn select_directory(app: &mut GuiApp, directory: Option<String>) {
    app.selected_directory = directory;
    // ディレクトリ選択時は、他の選択モードをクリア
    app.selected_album = None;
    app.selected_artist = None;
    // UI を更新
    app.update_app_vm();
}

/// ライブラリディレクトリツリー内の指定されたパスの展開状態をトグル（切り替え）します。
/// 
/// 展開済みの場合は折りたたみ、折りたたまれている場合は展開します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `path` - 展開状態を切り替えるディレクトリのパス
pub fn toggle_library_directory_expanded(app: &mut GuiApp, path: &str) {
    // 既に展開済みかどうかで処理を分岐
    if app.library_expanded_directories.contains(path) {
        // 展開済み → セットから削除（折りたたみ）
        app.library_expanded_directories.remove(path);
    } else {
        // 未展開 → セットに追加（展開）
        app.library_expanded_directories.insert(path.to_string());
    }
}

/// 指定されたパスのライブラリディレクトリが展開状態かどうかを返します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への参照
/// * `path` - 確認するディレクトリのパス
/// 
/// # 戻り値
/// * `true` - 展開されている
/// * `false` - 折りたたまれている
pub fn is_library_directory_expanded(app: &GuiApp, path: &str) -> bool {
    app.library_expanded_directories.contains(path)
}

/// お気に入りディレクトリを追加します。
/// 
/// 入力フィールド (`favorite_directory_input`) の内容を取得し、
/// 空でなく、かつ未登録であればお気に入りリストに追加します。
/// 追加後は入力フィールドをクリアし、状態を保存します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn add_favorite_directory(app: &mut GuiApp) {
    // 入力文字列をトリムして取得
    let dir = app.favorite_directory_input.trim().to_string();
    // 空文字列または既に登録済みの場合は何もしない
    if dir.is_empty() || app.favorite_directories.contains(&dir) {
        return;
    }

    // お気に入りリストに追加
    app.favorite_directories.push(dir);
    // 入力フィールドをクリア
    app.favorite_directory_input.clear();
    // 設定を永続化
    app.save_state();
    // UI を更新
    app.update_app_vm();
}

/// 指定されたインデックスのお気に入りディレクトリを削除します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - 削除するお気に入りディレクトリのインデックス
pub fn remove_favorite_directory(app: &mut GuiApp, index: usize) {
    // インデックスが有効範囲内かチェック
    if index < app.favorite_directories.len() {
        // 指定インデックスの要素を削除
        app.favorite_directories.remove(index);
        // 設定を永続化
        app.save_state();
        // UI を更新
        app.update_app_vm();
    }
}