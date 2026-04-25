//! キュー管理に関する操作を提供するモジュール。
//!
//! - キュー内の選択状態管理
//! - キュー項目の移動/削除
//! - キューへのトラック挿入
//! - 再生中のキュー状態調整

use lrc_player_domain::model::PlaybackState;
use crate::presentation::actions::play;
use crate::presentation::app::GuiApp;

/// 入力フィールドの内容をキュー末尾に追加します。
/// 
/// 空文字列や空白のみの場合は何もしません。
/// 追加後は入力フィールドをクリアし、UIを更新します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn add_queue_item(app: &mut GuiApp) {
    // 入力文字列をトリムして取得
    let item = app.queue_item_input.trim().to_string();
    // 空の場合は何もしない
    if item.is_empty() {
        return;
    }
    // キュー末尾に追加
    app.queue.tracks.push(item);
    // 入力フィールドをクリア
    app.queue_item_input.clear();
    // UIを更新
    app.update_app_vm();
}

/// キュー内のすべての選択状態をクリアします。
/// 
/// 単一選択インデックス、複数選択インデックス、選択アンカーをすべてリセットします。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn clear_queue_selection(app: &mut GuiApp) {
    app.selected_queue_index = None;
    app.selected_queue_indices.clear();
    app.queue_selection_anchor = None;
}

/// 指定されたインデックスを単一選択状態にします。
/// 
/// 複数選択はクリアされ、指定インデックスのみが選択状態になります。
/// 選択アンカーも指定インデックスに設定されます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - 選択するアイテムのインデックス
pub fn select_queue_item_single(app: &mut GuiApp, index: usize) {
    app.selected_queue_index = Some(index);
    app.selected_queue_indices.clear();
    app.selected_queue_indices.push(index);
    app.queue_selection_anchor = Some(index);
}

/// 指定トラックをキュー末尾に追加する。
/// 
/// 外部からトラックパスを直接キューに追加するための関数です。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `path` - 追加するトラックのファイルパス
pub fn perform_enqueue_track_path(app: &mut GuiApp, path: &str) {
    let item = path.trim();
    // 空パスは無視
    if item.is_empty() {
        return;
    }
    // キュー末尾に追加
    app.queue.tracks.push(item.to_string());
    // UIを更新
    app.update_app_vm();
}

/// 次に再生する位置にトラックを挿入する。
/// 
/// 現在再生中または選択中のトラックの直後に挿入します。
/// どちらも存在しない場合はキュー末尾に追加します。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `track` - 挿入するトラックのファイルパス
pub fn perform_add_track_next_in_queue(app: &mut GuiApp, track: &str) {
    let item = track.trim();
    // 空パスは無視
    if item.is_empty() {
        return;
    }

    // 挿入位置を決定
    // 優先順位: 現在再生中の次 > 選択中の次 > キュー末尾
    let insert_at = app
        .current_queue_index
        .or(app.selected_queue_index)
        .map(|index| (index + 1).min(app.queue.tracks.len()))
        .unwrap_or(app.queue.tracks.len());

    // 指定位置に挿入
    app.queue.tracks.insert(insert_at, item.to_string());
    // UIを更新
    app.update_app_vm();
}

/// 表示中のリストを再生キューに設定し、指定トラックを選択して再生開始する。
/// 
/// ライブラリビューなどから「再生」操作を行った際に使用されます。
/// 既存のキューは完全に置き換えられます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `displayed_tracks` - 表示中のトラックリスト
/// * `clicked_index` - 再生を開始するトラックのインデックス
pub fn perform_play_from_displayed_list(
    app: &mut GuiApp,
    displayed_tracks: &[String],
    clicked_index: usize,
) {
    // 空リストまたは範囲外インデックスの場合は何もしない
    if displayed_tracks.is_empty() || clicked_index >= displayed_tracks.len() {
        return;
    }

    // キューを表示中のリストで置き換え
    app.queue.tracks = displayed_tracks.to_vec();
    // 現在再生中インデックスをクリア
    app.current_queue_index = None;
    // 選択状態をクリア
    clear_queue_selection(app);
    // クリックされたトラックを単一選択
    select_queue_item_single(app, clicked_index);
    // UIを更新
    app.update_app_vm();
    // 選択されたトラックを再生
    play::play_selected_queue_item(app);
}

/// 指定されたインデックスのキューアイテムの選択状態をトグル（切り替え）します。
/// 
/// 選択済みの場合は選択解除、未選択の場合は選択追加します。
/// 選択アンカーは最初の選択時に設定され、全選択解除時のみクリアされます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - 選択状態を切り替えるアイテムのインデックス
pub fn toggle_queue_item_selection(app: &mut GuiApp, index: usize) {
    // 既に選択されているか確認
    if let Some(pos) = app.selected_queue_indices.iter().position(|&i| i == index) {
        // 選択済み → 選択解除
        app.selected_queue_indices.remove(pos);
        // 単一選択インデックスを最後に選択されたものに更新
        app.selected_queue_index = app.selected_queue_indices.last().copied();
        // 全選択解除された場合はアンカーもクリア
        if app.selected_queue_indices.is_empty() {
            app.queue_selection_anchor = None;
        }
    } else {
        // 未選択 → 選択追加
        app.selected_queue_indices.push(index);
        app.selected_queue_index = Some(index);
        // アンカーが未設定の場合は設定
        if app.queue_selection_anchor.is_none() {
            app.queue_selection_anchor = Some(index);
        }
    }
}

/// アンカーから指定インデックスまでの範囲を選択します。
/// 
/// Shift+クリックなどでの範囲選択に使用されます。
/// アンカーが未設定の場合は、現在の単一選択インデックスまたは指定インデックスが使用されます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - 範囲選択の終点インデックス
pub fn select_queue_item_range_to(app: &mut GuiApp, index: usize) {
    // アンカーを決定（アンカー > 単一選択 > index）
    let anchor = app
        .queue_selection_anchor
        .or(app.selected_queue_index)
        .unwrap_or(index);
    
    // 開始と終了を小さい順に並べる
    let (start, end) = if anchor <= index {
        (anchor, index)
    } else {
        (index, anchor)
    };
    
    // start から end までの全インデックスを選択
    app.selected_queue_indices = (start..=end).collect();
    app.selected_queue_index = Some(index);
    // アンカーは維持
    app.queue_selection_anchor = Some(anchor);
}

/// 現在選択されているキューアイテムをすべて削除します。
/// 
/// 削除後、再生状態に応じて適切な後続処理を行います：
/// - 再生中だったトラックが削除された場合、次のトラックを自動再生またはロード
/// - 再生中以外のトラックが削除された場合、現在のインデックスを適切に調整
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn remove_selected_queue_items(app: &mut GuiApp) {
    // 複数選択が空の場合は単一選択を削除
    if app.selected_queue_indices.is_empty() {
        if let Some(index) = app.selected_queue_index {
            remove_queue_item(app, index);
        }
        return;
    }

    // 削除前の状態を保存
    let current_index = app.current_queue_index;
    let mut indices = app.selected_queue_indices.clone();
    // ソートして重複除去
    indices.sort_unstable();
    indices.dedup();
    
    // 現在再生中のトラックが削除対象かどうか
    let current_removed = current_index
        .map(|current| indices.contains(&current))
        .unwrap_or(false);

    // 後ろから削除（インデックスのずれを防ぐため）
    indices.reverse();
    for index in indices.iter().copied() {
        if index < app.queue.tracks.len() {
            app.queue.tracks.remove(index);
        }
    }

    // 現在のインデックスより前に削除されたアイテムの数を計算
    let removed_before_current = current_index.map(|old_current| {
        indices.iter().filter(|&&removed| removed < old_current).count()
    });

    // 選択状態をクリア
    clear_queue_selection(app);

    // 現在再生中のトラックが削除された場合の処理
    if current_removed {
        if app.queue.tracks.is_empty() {
            // キューが空になった場合
            clear_queue(app);
            stop_playback_and_clear_play_tab(app);
        } else if let Some(old_index) = current_index {
            // 次のインデックスを決定（範囲内に収める）
            let next_index = if old_index < app.queue.tracks.len() {
                old_index
            } else {
                app.queue.tracks.len() - 1
            };
            select_queue_item_single(app, next_index);
            
            // 再生状態に応じた後続処理
            match app.playback_state {
                PlaybackState::Paused => {
                    // 一時停止中 → 選択トラックをロードして一時停止状態を維持
                    load_selected_queue_item_paused(app, next_index);
                }
                PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => {
                    // 再生中 → 選択トラックを再生開始
                    play::play_selected_queue_item(app);
                }
                PlaybackState::Stopped => {
                    // 停止中 → 選択トラックをロードして停止状態を維持
                    load_selected_queue_item_stopped(app, next_index);
                }
            }
        }
    } else if let Some(old_current) = current_index {
        // 再生中トラックは削除されていないが、前のアイテムが削除されたためインデックスを調整
        let shift = removed_before_current.unwrap_or(0);
        app.current_queue_index = Some(old_current.saturating_sub(shift));
    }

    // UIを更新
    app.update_app_vm();
}

/// キューハンドルのホールド状態をクリアします。
/// 
/// ドラッグ操作の中断時に呼び出されます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn clear_queue_handle_hold(app: &mut GuiApp) {
    app.queue_handle_hold_index = None;
    app.queue_handle_hold_started_at = None;
}

/// キュー内のドラッグ操作をキャンセルします。
/// 
/// ドラッグ元インデックスとハンドルホールド状態をクリアします。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn cancel_queue_drag(app: &mut GuiApp) {
    app.queue_drag_source_index = None;
    clear_queue_handle_hold(app);
}

/// キューアイテムを指定位置に移動します。
/// 
/// 移動に伴い、すべての関連インデックス（選択状態、アンカー、現在再生中）も
/// 適切に再マッピングされます。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `from` - 移動元インデックス
/// * `to` - 移動先インデックス
pub fn move_queue_item_to(app: &mut GuiApp, from: usize, to: usize) {
    // 範囲外または同じ位置への移動は無視
    if from >= app.queue.tracks.len() || to >= app.queue.tracks.len() || from == to {
        return;
    }

    // アイテムを削除して移動先に挿入
    let item = app.queue.tracks.remove(from);
    let insert_at = if to > from { to - 1 } else { to };
    app.queue.tracks.insert(insert_at, item);

    // インデックス再マッピング用クロージャ
    // 移動前のインデックスから移動後のインデックスを計算
    let remap_index = |idx: usize| {
        if idx == from {
            insert_at // 移動元は挿入先へ
        } else if from < insert_at {
            // 下方向への移動
            if idx > from && idx <= insert_at {
                idx - 1 // 間にあるものは1つずれる
            } else {
                idx
            }
        } else if insert_at < from {
            // 上方向への移動
            if idx >= insert_at && idx < from {
                idx + 1 // 間にあるものは1つずれる
            } else {
                idx
            }
        } else {
            idx
        }
    };

    // 複数選択インデックスを再マッピング
    app.selected_queue_indices = app
        .selected_queue_indices
        .iter()
        .copied()
        .map(remap_index)
        .collect();
    app.selected_queue_indices.sort_unstable();
    app.selected_queue_indices.dedup();

    // 各インデックスを再マッピング
    app.selected_queue_index = app.selected_queue_index.map(remap_index);
    app.queue_selection_anchor = app.queue_selection_anchor.map(remap_index);
    app.current_queue_index = app.current_queue_index.map(remap_index);

    // 選択状態をクリア
    clear_queue_selection(app);
    // UIを更新
    app.update_app_vm();
}

/// 指定されたインデックスのキューアイテムを1件削除します。
/// 
/// 削除後、必要に応じて選択状態や現在再生中インデックスが調整されます。
/// 再生中のトラックが削除された場合は、次のトラックの再生またはロードを行います。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - 削除するアイテムのインデックス
pub fn remove_queue_item(app: &mut GuiApp, index: usize) {
    if index < app.queue.tracks.len() {
        // 削除対象が現在再生中かどうか
        let current_removed = app.current_queue_index == Some(index);
        
        // アイテムを削除
        app.queue.tracks.remove(index);
        
        // 複数選択インデックスを更新（削除されたものを除去し、後続のものをシフト）
        app.selected_queue_indices = app
            .selected_queue_indices
            .iter()
            .filter_map(|&i| {
                if i == index {
                    None // 削除対象は除去
                } else if i > index {
                    Some(i - 1) // 後続は1つ前にシフト
                } else {
                    Some(i) // 前のものはそのまま
                }
            })
            .collect();
        
        // 単一選択インデックスを更新
        if app.selected_queue_index == Some(index) {
            app.selected_queue_index = app.selected_queue_indices.last().copied();
        } else if let Some(selected) = app.selected_queue_index {
            if selected > index {
                app.selected_queue_index = Some(selected - 1);
            }
        }
        
        // 選択アンカーを更新
        if let Some(anchor) = app.queue_selection_anchor {
            app.queue_selection_anchor = if anchor == index {
                app.selected_queue_index // アンカーが削除された場合、単一選択に合わせる
            } else if anchor > index {
                Some(anchor - 1)
            } else {
                Some(anchor)
            };
        }

        // 現在再生中のトラックが削除された場合の処理
        if current_removed {
            if app.queue.tracks.is_empty() {
                // キューが空になった場合
                clear_queue(app);
                stop_playback_and_clear_play_tab(app);
            } else {
                // 次のインデックスを決定
                let next_index = if index < app.queue.tracks.len() {
                    index
                } else {
                    app.queue.tracks.len() - 1
                };
                select_queue_item_single(app, next_index);
                
                // 再生状態に応じた後続処理
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
            // 再生中トラックは削除されていないが、インデックスを調整
            app.current_queue_index = if let Some(current) = app.current_queue_index {
                if current > index {
                    Some(current - 1) // 削除位置より後ろなら1つ前にシフト
                } else {
                    Some(current) // 削除位置より前ならそのまま
                }
            } else {
                None
            };
        }

        // UIを更新
        app.update_app_vm();
    }
}

/// 指定されたインデックスのキューアイテムを一つ上に移動します。
/// 
/// 移動後、そのアイテムが単一選択状態になります。
/// 先頭のアイテムは移動できません。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - 移動対象のインデックス
pub fn move_queue_item_up(app: &mut GuiApp, index: usize) {
    // 先頭または範囲外の場合は移動不可
    if index == 0 || index >= app.queue.tracks.len() {
        return;
    }
    // 一つ上のアイテムと交換
    app.queue.tracks.swap(index, index - 1);
    // 移動先を単一選択
    select_queue_item_single(app, index - 1);
    // UIを更新
    app.update_app_vm();
}

/// 指定されたインデックスのキューアイテムを一つ下に移動します。
/// 
/// 移動後、そのアイテムが単一選択状態になります。
/// 末尾のアイテムは移動できません。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - 移動対象のインデックス
pub fn move_queue_item_down(app: &mut GuiApp, index: usize) {
    // 末尾または範囲外の場合は移動不可
    if index + 1 >= app.queue.tracks.len() {
        return;
    }
    // 一つ下のアイテムと交換
    app.queue.tracks.swap(index, index + 1);
    // 移動先を単一選択
    select_queue_item_single(app, index + 1);
    // UIを更新
    app.update_app_vm();
}

/// キューを完全にクリアします。
/// 
/// 全トラックを削除し、選択状態、ドラッグ状態、現在再生中インデックスをすべてリセットします。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
pub fn clear_queue(app: &mut GuiApp) {
    app.queue.tracks.clear();
    clear_queue_selection(app);
    cancel_queue_drag(app);
    app.current_queue_index = None;
    app.update_app_vm();
}

/// 再生を停止し、再生タブの表示をクリアします。
/// 
/// キューが空になった場合などに呼び出される内部関数です。
/// バックエンドへの停止信号送信、再生状態のリセット、表示情報のクリアを行います。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
fn stop_playback_and_clear_play_tab(app: &mut GuiApp) {
    // バックエンドに停止を通知
    app.signal_playback_stop();
    // 再生状態を停止に設定
    app.playback_state = PlaybackState::Stopped;
    app.play_status = Some("Playback stopped".to_string());
    
    // 再生パスと位置をクリア
    app.play_path.clear();
    app.play_position_ms = 0;
    app.play_duration_ms = None;
    
    // メタデータ表示をクリア
    app.play_title = None;
    app.play_artist = None;
    app.play_album = None;
    app.play_artwork_handle = None;
    app.play_artwork_source = None;
    
    // 歌詞情報をクリア
    app.lyrics_files.clear();
    app.loaded_lyrics = None;
    app.selected_lyrics_index = None;
    app.lyrics_offset_ms = 0;
    
    // 再生タイマー関連をクリア
    app.playback_start_instant = None;
    app.pending_play_request = false;
}

/// 選択されたキューアイテムを一時停止状態でロードします。
/// 
/// キューアイテム削除後、再生状態が一時停止だった場合に呼び出される内部関数です。
/// トラック情報をロードしますが、再生は開始しません。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - ロードするアイテムのインデックス
fn load_selected_queue_item_paused(app: &mut GuiApp, index: usize) {
    if let Some(path) = app.queue.tracks.get(index).cloned() {
        // 選択状態を設定
        select_queue_item_single(app, index);
        app.current_queue_index = Some(index);
        
        // トラック変更時に歌詞オフセットをリセット
        app.reset_lyrics_offset_if_track_changed(&path);
        // 再生パスを設定
        app.play_path = path;
        app.play_position_ms = 0;
        
        // 状態を一時停止に設定
        app.playback_state = PlaybackState::Paused;
        app.play_status = Some("Paused at start of selected track".to_string());
        
        // メタデータと歌詞を読み込み
        app.discover_lyrics_for_current_play_path();
        app.refresh_play_metadata();
        
        // タイマーは停止状態
        app.playback_start_instant = None;
        app.pending_play_request = false;
    }
}

/// 選択されたキューアイテムを停止状態でロードします。
/// 
/// キューアイテム削除後、再生状態が停止だった場合に呼び出される内部関数です。
/// トラック情報をロードしますが、再生は開始しません。
/// 
/// # 引数
/// * `app` - アプリケーション状態への可変参照
/// * `index` - ロードするアイテムのインデックス
fn load_selected_queue_item_stopped(app: &mut GuiApp, index: usize) {
    if let Some(path) = app.queue.tracks.get(index).cloned() {
        // 選択状態を設定
        select_queue_item_single(app, index);
        app.current_queue_index = Some(index);
        
        // トラック変更時に歌詞オフセットをリセット
        app.reset_lyrics_offset_if_track_changed(&path);
        // 再生パスを設定
        app.play_path = path;
        app.play_position_ms = 0;
        
        // 状態を停止に設定
        app.playback_state = PlaybackState::Stopped;
        app.play_status = Some("Stopped at start of selected track".to_string());
        
        // メタデータと歌詞を読み込み
        app.discover_lyrics_for_current_play_path();
        app.refresh_play_metadata();
        
        // タイマーは停止状態
        app.playback_start_instant = None;
        app.pending_play_request = false;
    }
}