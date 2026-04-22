//! プレイリスト構造の基本操作を提供するモジュール。
//!
//! - プレイリストの追加/削除/並び替え
//! - プレイリスト一覧の正規化
//! - プレイリストの永続化

use lrc_player_application::dto::PlaylistDto;
use lrc_player_application::infrastructure::persistence::PlaylistsFile;
use crate::presentation::app::GuiApp;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// プレイリスト一覧を正規化します。
/// 
/// 以下の処理を行います：
/// - "favorite" という名前のプレイリストを一つに統合する
/// - 統合された favorite プレイリスト内の重複トラックを除去する
/// - favorite プレイリストをリストの先頭に配置する
/// 
/// # 引数
/// * `playlists` - 正規化対象のプレイリストDTOのベクタ
/// 
/// # 戻り値
/// 正規化されたプレイリストDTOのベクタ（favoriteが先頭、その他は元の順序を保持）
pub fn normalize_playlists(playlists: Vec<PlaylistDto>) -> Vec<PlaylistDto> {
    // 統合後の favorite トラックを格納するベクタ
    let mut favorite_tracks: Vec<String> = Vec::new();
    // 重複除去のための既出トラック追跡用ベクタ
    let mut seen_tracks: Vec<String> = Vec::new();
    // 最初に見つかった favorite プレイリストのIDを保持
    let mut favorite_id: Option<String> = None;
    // favorite 以外のプレイリストを格納するベクタ
    let mut others: Vec<PlaylistDto> = Vec::new();

    // 全プレイリストを走査
    for playlist in playlists {
        // 名前が "favorite"（大文字小文字区別なし）の場合
        if playlist.name.eq_ignore_ascii_case("favorite") {
            // 最初の favorite の ID を保持（存在しない場合のみ）
            if favorite_id.is_none() {
                favorite_id = Some(playlist.id.clone());
            }
            // 各トラックを重複チェックしながら追加
            for track in playlist.tracks {
                if seen_tracks.contains(&track) {
                    continue; // 既出のトラックはスキップ
                }
                seen_tracks.push(track.clone());
                favorite_tracks.push(track);
            }
        } else {
            // favorite 以外はそのまま保持
            others.push(playlist);
        }
    }

    // 統合された favorite プレイリストを構築
    // IDが存在しない場合はデフォルトID "playlist-favorite" を使用
    let favorite = PlaylistDto {
        id: favorite_id.unwrap_or_else(|| "playlist-favorite".to_string()),
        name: "favorite".to_string(),
        tracks: favorite_tracks,
    };

    // 結果ベクタを構築（favorite + その他）
    let mut normalized = Vec::with_capacity(others.len() + 1);
    normalized.push(favorite);       // favorite を先頭に配置
    normalized.extend(others);       // その他のプレイリストを後続に追加
    normalized
}

/// 現在のプレイリスト一覧をファイルに保存します。
/// 
/// プレイリストの追加・削除・並び替え・編集後に呼び出され、
/// 変更内容を永続化します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への参照
pub fn save_playlists(app: &GuiApp) {
    // プレイリストファイルを開く
    if let Ok(file) = PlaylistsFile::open() {
        // 現在のプレイリスト一覧を保存
        let _ = file.save(&app.playlists);
    }
}

/// 新しいプレイリストを追加します。
/// 
/// 以下の条件を満たす場合のみ追加されます：
/// - 名前が空でないこと
/// - 名前が "favorite" でないこと（予約語）
/// - 同じ名前のプレイリストが既に存在しないこと（大文字小文字区別なし）
/// 
/// 追加後は入力フィールドをクリアし、保存とUI更新を行います。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn add_playlist(app: &mut GuiApp) {
    // 入力文字列をトリムして取得
    let name = app.new_playlist_name.trim().to_string();
    
    // 名前が空の場合は何もしない
    if name.is_empty() {
        return;
    }
    
    // "favorite" は予約語のため追加不可
    if name.eq_ignore_ascii_case("favorite") {
        return;
    }
    
    // 同名プレイリストが既に存在するかチェック
    if app
        .playlists
        .iter()
        .any(|playlist| playlist.name.eq_ignore_ascii_case(&name))
    {
        return;
    }

    // 現在時刻（ミリ秒）をベースにユニークなIDを生成
    let id = format!(
        "playlist-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    // 新しいプレイリストを追加（トラックは空）
    app.playlists.push(PlaylistDto {
        id,
        name,
        tracks: Vec::new(),
    });
    
    // 入力フィールドをクリア
    app.new_playlist_name.clear();
    // ファイルに保存
    save_playlists(app);
    // UIを更新
    app.update_app_vm();
}

/// 指定されたインデックスのプレイリストを削除します。
/// 
/// ただし、"favorite" プレイリストは削除できません。
/// 削除対象が現在選択中のプレイリストだった場合、選択状態もクリアされます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - 削除するプレイリストのインデックス
pub fn remove_playlist(app: &mut GuiApp, index: usize) {
    // インデックスが有効範囲内かつ、削除対象が "favorite" でないことを確認
    if index < app.playlists.len()
        && !app.playlists[index].name.eq_ignore_ascii_case("favorite")
    {
        // プレイリストを削除
        app.playlists.remove(index);
        
        // 削除したプレイリストが選択中だった場合、選択を解除
        if app.selected_playlist == Some(index) {
            app.selected_playlist = None;
        }
        
        // ファイルに保存
        save_playlists(app);
        // UIを更新
        app.update_app_vm();
    }
}

/// 指定されたインデックスのプレイリストを一つ上に移動します。
/// 
/// 以下の場合は移動できません：
/// - インデックスが1以下（先頭または範囲外）
/// - 一つ上のプレイリストが "favorite" の場合（favoriteは常に先頭固定）
/// 
/// 移動に伴い、選択状態も適切に追従します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - 移動対象のプレイリストのインデックス
pub fn move_playlist_up(app: &mut GuiApp, index: usize) {
    // インデックスが1以下または範囲外の場合は移動不可
    // （index=0は先頭、index=1の場合は一つ上がfavoriteの可能性があるため後続でチェック）
    if index <= 1 || index >= app.playlists.len() {
        return;
    }
    
    // 一つ上のプレイリストが "favorite" の場合は移動不可（favoriteは先頭固定）
    if app.playlists[index - 1].name.eq_ignore_ascii_case("favorite") {
        return;
    }

    // 一つ上のプレイリストと位置を交換
    app.playlists.swap(index - 1, index);
    
    // 選択状態のインデックスも追従させる
    if let Some(selected) = app.selected_playlist {
        if selected == index {
            // 移動対象が選択中だった場合、選択も一つ上へ
            app.selected_playlist = Some(index - 1);
        } else if selected == index - 1 {
            // 一つ上が選択中だった場合、選択も一つ下へ（交換された先）
            app.selected_playlist = Some(index);
        }
    }
    
    // ファイルに保存
    save_playlists(app);
    // UIを更新
    app.update_app_vm();
}

/// 指定されたインデックスのプレイリストを一つ下に移動します。
/// 
/// 以下の場合は移動できません：
/// - インデックスが0（先頭、すなわち "favorite"）
/// - 移動対象が "favorite" の場合（favoriteは常に先頭固定）
/// - インデックスが末尾の場合
/// 
/// 移動に伴い、選択状態も適切に追従します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - 移動対象のプレイリストのインデックス
pub fn move_playlist_down(app: &mut GuiApp, index: usize) {
    // インデックスが0（先頭）または末尾の場合は移動不可
    if index == 0 || index + 1 >= app.playlists.len() {
        return;
    }
    
    // 移動対象が "favorite" の場合は移動不可（favoriteは先頭固定）
    if app.playlists[index].name.eq_ignore_ascii_case("favorite") {
        return;
    }

    // 一つ下のプレイリストと位置を交換
    app.playlists.swap(index, index + 1);
    
    // 選択状態のインデックスも追従させる
    if let Some(selected) = app.selected_playlist {
        if selected == index {
            // 移動対象が選択中だった場合、選択も一つ下へ
            app.selected_playlist = Some(index + 1);
        } else if selected == index + 1 {
            // 一つ下が選択中だった場合、選択も一つ上へ（交換された先）
            app.selected_playlist = Some(index);
        }
    }
    
    // ファイルに保存
    save_playlists(app);
    // UIを更新
    app.update_app_vm();
}