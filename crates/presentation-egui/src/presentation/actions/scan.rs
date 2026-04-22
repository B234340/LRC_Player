//! スキャン操作に関する処理を提供するモジュール。
//!
//! - スキャン要求/キャンセル
//! - スキャン対象の追加/削除
//! - LRC/除外対象の追加/削除

use crate::presentation::app::GuiApp;
use lrc_player_shared::ipc as ipc_mod;

/// スキャン要求をバックエンドプロセスに送信します。
/// 
/// 設定されたスキャン対象パスとLRC歌詞パスをバックエンドに送信し、
/// メディアファイルのスキャンを開始します。
/// スキャン対象が一つも設定されていない場合はエラーメッセージを表示します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn request_scan(app: &mut GuiApp) {
    // スキャン対象パスをクローン
    let paths = app.scan_targets.clone();
    let lrc_paths = app.scan_lrc_targets.clone();
    
    // スキャン対象が空の場合はエラーを表示して終了
    if paths.is_empty() && lrc_paths.is_empty() {
        app.last_scan_status = Some("No scan targets configured".to_string());
        app.save_state();
        app.update_app_vm();
        return;
    }

    // ステータスメッセージを更新
    app.last_scan_status = Some("Requested scan".to_string());
    // 状態を保存
    app.save_state();
    // UIを更新
    app.update_app_vm();

    // IPCソケットのパスを取得
    let socket = ipc_mod::default_socket_path();
    // スキャン要求オブジェクトを構築
    let request = ipc_mod::ScanRequest {
        paths,                             // 音楽ファイルのスキャン対象
        lrc_paths,                         // LRC歌詞ファイルのスキャン対象
        excludes: app.scan_excludes.clone(), // 除外パターン
        cancel: false,                     // キャンセル要求ではない
    };
    
    // 別スレッドでIPC要求を送信（UIスレッドをブロックしないため）
    std::thread::spawn(move || {
        // 現在のスレッド用のTokioランタイムを構築
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            // 非同期でスキャン要求を送信
            let _ = rt.block_on(async move {
                ipc_mod::send_scan_request(&socket, &request).await
            });
        }
    });
}

/// スキャンキャンセル要求をバックエンドプロセスに送信します。
/// 
/// 現在実行中のスキャン処理を中断するようバックエンドに要求します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn request_cancel(app: &mut GuiApp) {
    // ステータスメッセージを更新
    app.last_scan_status = Some("Requested scan cancel".to_string());
    // 状態を保存
    app.save_state();
    // UIを更新
    app.update_app_vm();

    // IPCソケットのパスを取得
    let socket = ipc_mod::default_socket_path();
    // キャンセル要求オブジェクトを構築
    let request = ipc_mod::ScanRequest {
        paths: Vec::new(),                 // キャンセル時は空で良い
        lrc_paths: Vec::new(),             // キャンセル時は空で良い
        excludes: app.scan_excludes.clone(), // 除外パターン（念のため送信）
        cancel: true,                      // キャンセル要求であることを示す
    };
    
    // 別スレッドでIPC要求を送信
    std::thread::spawn(move || {
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            let _ = rt.block_on(async move {
                ipc_mod::send_scan_request(&socket, &request).await
            });
        }
    });
}

/// スキャン対象パスを追加します。
/// 
/// 入力フィールドの内容をトリムし、空でなければスキャン対象リストに追加します。
/// 追加後は入力フィールドをクリアし、状態を保存してUIを更新します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn add_scan_target(app: &mut GuiApp) {
    // 入力文字列をトリムして取得
    let target = app.scan_target_input.trim().to_string();
    // 空でなければ追加
    if !target.is_empty() {
        app.scan_targets.push(target);
        // 入力フィールドをクリア
        app.scan_target_input.clear();
        // 設定を永続化
        app.save_state();
        // UIを更新
        app.update_app_vm();
    }
}

/// 指定されたインデックスのスキャン対象パスを削除します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - 削除するスキャン対象のインデックス
pub fn remove_scan_target(app: &mut GuiApp, index: usize) {
    // インデックスが有効範囲内かチェック
    if index < app.scan_targets.len() {
        app.scan_targets.remove(index);
        // 設定を永続化
        app.save_state();
        // UIを更新
        app.update_app_vm();
    }
}

/// LRC歌詞ファイルのスキャン対象パスを追加します。
/// 
/// 入力フィールドの内容をトリムし、空でなければLRCスキャン対象リストに追加します。
/// 追加後は入力フィールドをクリアし、状態を保存してUIを更新します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn add_scan_lrc_target(app: &mut GuiApp) {
    // 入力文字列をトリムして取得
    let target = app.scan_lrc_target_input.trim().to_string();
    // 空でなければ追加
    if !target.is_empty() {
        app.scan_lrc_targets.push(target);
        // 入力フィールドをクリア
        app.scan_lrc_target_input.clear();
        // 設定を永続化
        app.save_state();
        // UIを更新
        app.update_app_vm();
    }
}

/// 指定されたインデックスのLRCスキャン対象パスを削除します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - 削除するLRCスキャン対象のインデックス
pub fn remove_scan_lrc_target(app: &mut GuiApp, index: usize) {
    // インデックスが有効範囲内かチェック
    if index < app.scan_lrc_targets.len() {
        app.scan_lrc_targets.remove(index);
        // 設定を永続化
        app.save_state();
        // UIを更新
        app.update_app_vm();
    }
}

/// スキャン除外パターンを追加します。
/// 
/// 入力フィールドの内容をトリムし、空でなければ除外パターンリストに追加します。
/// 除外パターンは、スキャン対象ディレクトリ内で無視するファイルやディレクトリを
/// 指定するためのものです（例: "*.tmp", "backup/" など）。
/// 追加後は入力フィールドをクリアし、状態を保存してUIを更新します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn add_scan_exclude(app: &mut GuiApp) {
    // 入力文字列をトリムして取得
    let exclude = app.scan_exclude_input.trim().to_string();
    // 空でなければ追加
    if !exclude.is_empty() {
        app.scan_excludes.push(exclude);
        // 入力フィールドをクリア
        app.scan_exclude_input.clear();
        // 設定を永続化
        app.save_state();
        // UIを更新
        app.update_app_vm();
    }
}

/// 指定されたインデックスのスキャン除外パターンを削除します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - 削除する除外パターンのインデックス
pub fn remove_scan_exclude(app: &mut GuiApp, index: usize) {
    // インデックスが有効範囲内かチェック
    if index < app.scan_excludes.len() {
        app.scan_excludes.remove(index);
        // 設定を永続化
        app.save_state();
        // UIを更新
        app.update_app_vm();
    }
}