//! 再生/シークに関する操作を提供するモジュール。
//!
//! - 再生状態の切り替え
//! - シーク位置の管理
//! - 再生キューの再生制御

use lrc_player_domain::model::{PlaybackMode, PlaybackState};
use crate::presentation::app::GuiApp;
use rand::seq::IteratorRandom;
use rand::thread_rng;
use std::time::Instant;

/// 再生キュー内の現在のトラックのシーク位置を設定します。
/// 
/// # 引数
/// * `app` - アプリケーションの状態を保持する構造体への可変参照
/// * `position_ms` - シーク先の位置（ミリ秒単位）
pub fn queue_seek_position(app: &mut GuiApp, position_ms: u64) {
    // 現在の再生位置を更新
    app.play_position_ms = position_ms;
    // 保留中のシーク要求として記録（実際のシーク処理は別途行われる）
    app.pending_seek_ms = Some(position_ms);
    // シーク操作が最後に変更された時刻を記録（UIのスロットリングなどに使用）
    app.seek_last_change_at = Some(Instant::now());
}

/// キュー内で現在選択されているアイテムを再生します。
/// 
/// 選択されたトラックがあれば、そのトラックを再生対象として設定し、
/// 再生の準備と開始要求を行います。
pub fn play_selected_queue_item(app: &mut GuiApp) {
    // 選択されているキューインデックスを取得
    if let Some(index) = app.selected_queue_index {
        // インデックスに対応するトラックパスをクローン
        if let Some(path) = app.queue.tracks.get(index).cloned() {
            // キュー選択を単一選択に設定（他の選択を解除）
            app.select_queue_item_single(index);
            // 現在のキューインデックスを更新
            app.current_queue_index = Some(index);
            // トラック変更時に歌詞オフセットをリセット
            app.reset_lyrics_offset_if_track_changed(&path);
            // 再生パスを設定
            app.play_path = path;
            // 再生位置を先頭にリセット
            app.play_position_ms = 0;
            // 再生状態をバッファリング中に設定
            app.playback_state = PlaybackState::Buffering;
            // 現在の再生パスに対応する歌詞ファイルを検索
            app.discover_lyrics_for_current_play_path();
            // 再生するトラックのメタデータ（長さなど）を更新
            app.refresh_play_metadata();
            // 再生開始を要求
            app.request_play();
        }
    }
}

/// 指定されたパスのトラックを即座に再生します。
/// 
/// キューとは独立して、直接指定されたファイルを再生します。
/// 
/// # 引数
/// * `app` - アプリケーションの状態
/// * `path` - 再生するオーディオファイルのパス
pub fn perform_play_track_now(app: &mut GuiApp, path: &str) {
    // 空文字列や空白のみのパスは無視
    if path.trim().is_empty() {
        return;
    }
    // トラック変更時に歌詞オフセットをリセット
    app.reset_lyrics_offset_if_track_changed(path);
    // 再生パスを設定
    app.play_path = path.to_string();
    // 再生位置を先頭にリセット
    app.play_position_ms = 0;
    // 再生状態をバッファリング中に設定
    app.playback_state = PlaybackState::Buffering;
    // 歌詞ファイルを検索
    app.discover_lyrics_for_current_play_path();
    // メタデータを更新
    app.refresh_play_metadata();
    // 再生開始を要求
    app.request_play();
}

/// 現在設定されている再生位置へシーク（移動）します。
/// 
/// 再生中、一時停止中、停止中のいずれの状態でも、指定された位置へのシークを試みます。
pub fn seek_to_position(app: &mut GuiApp) {
    // 再生パスが空の場合は何もしない
    if app.play_path.trim().is_empty() {
        return;
    }
    // 再生時間が不明な場合はメタデータを取得
    if app.play_duration_ms.is_none() {
        app.refresh_play_metadata();
    }

    let target_ms = app.play_position_ms;
    match app.playback_state {
        // 停止中または一時停止中の場合
        PlaybackState::Stopped | PlaybackState::Paused => {
            // 状態は変更せず、ステータスメッセージのみ更新
            app.play_status = Some(format!("Seek position updated: {} ms", target_ms));
        }
        // その他の状態（再生中、バッファリング中など）
        _ => {
            // シーク前の状態を保存（シーク完了後に復元するため）
            app.playback_state_before_seek = Some(app.playback_state);
            // 状態をシーク中に変更
            app.playback_state = PlaybackState::Seeking;
            // ステータスメッセージを更新
            app.play_status = Some(format!("Seeking to {} ms", target_ms));
            // 再生要求を送信（バックエンドがシークを処理）
            app.request_play();
        }
    }
}

/// 現在のキューアイテムを再生します。
/// 
/// 再生すべきトラックを決定する優先順位は以下の通りです：
/// 1. 現在再生中のインデックス (`current_queue_index`)
/// 2. 選択中のインデックス (`selected_queue_index`)
/// 3. キューが空でなければ先頭 (0)
pub fn play_current_queue_item(app: &mut GuiApp) {
    let len = app.queue.tracks.len();
    if len == 0 {
        return;
    }

    // 再生するインデックスを決定（範囲外インデックスは無効化）
    let index = app
        .current_queue_index
        .filter(|&i| i < len)
        .or(app.selected_queue_index.filter(|&i| i < len))
        .or(Some(0));

    // インデックスが存在すれば再生を開始
    if let Some(index) = index {
        if let Some(path) = app.queue.tracks.get(index).cloned() {
            app.select_queue_item_single(index);
            app.current_queue_index = Some(index);
            app.reset_lyrics_offset_if_track_changed(&path);
            app.play_path = path;
            app.playback_state = PlaybackState::Buffering;
            app.discover_lyrics_for_current_play_path();
            app.refresh_play_metadata();
            app.request_play();
        }
    }
}

/// 次のトラックを再生します。
/// 
/// 現在の再生モード (`PlaybackMode`) に応じて、次のトラックを決定します。
/// - Sequential: 次のインデックス
/// - RepeatOne: 現在と同じトラック
/// - RepeatAll: 次のインデックス（末尾の場合は先頭へ）
/// - Shuffle: ランダムなトラック
pub fn play_next(app: &mut GuiApp) {
    let len = app.queue.tracks.len();
    // キューが空なら何もしない
    if len == 0 {
        return;
    }

    // 現在のインデックスを決定（再生中 > 選択中 > 0）
    let current = app
        .current_queue_index
        .filter(|&i| i < len)
        .or(app.selected_queue_index.filter(|&i| i < len))
        .unwrap_or(0);

    // 再生モードに応じて次のインデックスを計算
    let next_index = match app.playback_mode {
        // 順次再生：次のインデックスへ（範囲内チェック付き）
        PlaybackMode::Sequential => current.checked_add(1).filter(|&index| index < len),
        // 1曲リピート：同じインデックス
        PlaybackMode::RepeatOne => Some(current),
        // 全曲リピート：次のインデックス、最後なら先頭へ
        PlaybackMode::RepeatAll => current
            .checked_add(1)
            .filter(|&index| index < len)
            .or(Some(0)),
        // シャッフル：現在のトラックを除く中からランダムに選択
        PlaybackMode::Shuffle => {
            let mut rng = thread_rng();
            // 現在のインデックス以外を集める
            let choices: Vec<usize> = (0..len).filter(|&i| i != current).collect();
            // 他に選択肢がなければ現在のトラックを維持
            if choices.is_empty() {
                Some(current)
            } else {
                // ランダムに一つ選択
                choices.into_iter().choose(&mut rng)
            }
        }
    };

    // 次のインデックスが決まれば再生
    if let Some(index) = next_index {
        app.select_queue_item_single(index);
        play_selected_queue_item(app);
    }
}

/// 前のトラックを再生します。
/// 
/// 現在の再生モード (`PlaybackMode`) に応じて、前のトラックを決定します。
/// - Sequential: 前のインデックス
/// - RepeatOne: 現在と同じトラック
/// - RepeatAll: 前のインデックス（先頭の場合は末尾へ）
/// - Shuffle: ランダムなトラック
pub fn play_previous(app: &mut GuiApp) {
    let len = app.queue.tracks.len();
    // キューが空なら何もしない
    if len == 0 {
        return;
    }

    // 現在のインデックスを決定
    let current = app
        .current_queue_index
        .filter(|&i| i < len)
        .or(app.selected_queue_index.filter(|&i| i < len))
        .unwrap_or(0);

    // 再生モードに応じて前のインデックスを計算
    let prev_index = match app.playback_mode {
        // 順次再生：前のインデックスへ
        PlaybackMode::Sequential => current.checked_sub(1),
        // 1曲リピート：同じインデックス
        PlaybackMode::RepeatOne => Some(current),
        // 全曲リピート：前のインデックス、先頭なら最後へ
        PlaybackMode::RepeatAll => {
            if current == 0 {
                Some(len - 1)
            } else {
                Some(current - 1)
            }
        }
        // シャッフル：ランダムに選択（`play_next`と同様の動作）
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

    // 前のインデックスが決まれば再生
    if let Some(index) = prev_index {
        app.select_queue_item_single(index);
        play_selected_queue_item(app);
    }
}

/// 再生を一時停止します。
/// 
/// 内部タイマーによる再生位置を確定し、バックエンドに停止信号を送ります。
pub fn pause_playback(app: &mut GuiApp) {
    // 現在の再生位置を内部クロックから更新
    app.update_playback_position_from_clock();
    // バックエンドに停止を通知
    app.signal_playback_stop();
    // 状態を一時停止に変更
    app.playback_state = PlaybackState::Paused;
    // ステータスメッセージを設定
    app.play_status = Some("Paused".to_string());
    // 再生開始時刻の記録をクリア
    app.playback_start_instant = None;
    // 保留中の再生要求フラグをオフ
    app.pending_play_request = false;
}

/// 再生と一時停止をトグル（切り替え）します。
/// 
/// - 再生中、バッファリング中、シーク中の場合 → 一時停止
/// - 一時停止中、停止中の場合 → 再生開始（または再開）
pub fn toggle_play_pause(app: &mut GuiApp) {
    match app.playback_state {
        // アクティブな再生状態 → 一時停止
        PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => {
            pause_playback(app);
        }
        // 停止状態 → 再生開始または再開
        PlaybackState::Paused | PlaybackState::Stopped => {
            // 再生パスが空の場合
            if app.play_path.trim().is_empty() {
                // キューにトラックがあれば、現在のキューアイテムを再生
                if !app.queue.tracks.is_empty() {
                    play_current_queue_item(app);
                    return;
                }
                // キューも空ならエラーメッセージを表示
                app.play_status = Some("No track loaded".to_string());
                return;
            }
            // 再生パスが有効な場合、再生を再開
            app.playback_state = PlaybackState::Buffering;
            app.play_status = Some("Resuming playback".to_string());
            app.request_play();
        }
    }
}