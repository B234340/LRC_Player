use crate::domain::model::PlaybackMode;
use crate::infrastructure::persistence::MetaCache;
use crate::presentation::components::queue_actions::{
    perform_add_track_next_in_queue,
    perform_enqueue_track_path,
};
use crate::presentation::components::track_actions::{
    perform_add_track_to_playlist,
    perform_open_track_details,
};
use crate::presentation::components::track_display::render_track_artwork;
use crate::presentation::components::track_list::{render_track_list_with_artwork, TrackActivation};
use crate::presentation::egui_app::GuiApp;
use eframe::egui;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 再生キュー画面のUIを描画する関数
pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    // --- キーボード修飾キーの状態を取得 ---
    let (ctrl_pressed, shift_pressed, esc_pressed) = ui.ctx().input(|input| {
        (
            input.modifiers.ctrl,   // Ctrlキー押下状態
            input.modifiers.shift,  // Shiftキー押下状態
            input.key_pressed(egui::Key::Escape),  // Escapeキー押下検出
        )
    });
    
    // Escapeキーで選択解除とドラッグ操作をキャンセル
    if esc_pressed {
        app.clear_queue_selection();
        app.cancel_queue_drag();
    }

    // マウスのプライマリボタンが離されたら、ハンドル長押し状態をクリア
    if !ui.ctx().input(|input| input.pointer.primary_down()) {
        app.clear_queue_handle_hold();
    }

    // --- ヘッダー表示 ---
    ui.heading("Queue");
    ui.label(format!("{} tracks in queue", app.app_vm.queue.tracks.len()));
    ui.separator();

    // --- キューへの楽曲追加入力欄 ---
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.queue_item_input);
        if ui.button("Add to queue").clicked() {
            app.add_queue_item();
        }
    });

    // --- 再生モード選択 ---
    let mut mode_changed = false;
    ui.horizontal(|ui| {
        ui.label("Playback mode:");
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, PlaybackMode::Sequential, "Sequential")
            .changed();
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, PlaybackMode::RepeatOne, "Repeat One")
            .changed();
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, PlaybackMode::RepeatAll, "Repeat All")
            .changed();
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, PlaybackMode::Shuffle, "Shuffle")
            .changed();
    });
    
    // モード変更時は状態を保存
    if mode_changed {
        app.save_state();
    }

    ui.add_space(8.0);

    // --- ドラッグ移動中のガイダンス表示 ---
    if let Some(source) = app.queue_drag_source_index {
        ui.label(format!(
            "Moving track {}. Click a destination row/handle to place, or click the same row/handle to cancel.",
            source + 1
        ));
    }

    // キューが空の場合
    let tracks = app.app_vm.queue.tracks.clone();
    if tracks.is_empty() {
        ui.label("Queue is empty.");
        return;
    }

    // --- 遅延実行するアクションを保持する変数群 ---
    let mut pending_move_up: Option<usize> = None;           // 上に移動
    let mut pending_move_down: Option<usize> = None;         // 下に移動
    let mut pending_remove_indices: Option<Vec<usize>> = None; // 削除（複数選択対応）
    let mut pending_play: Option<usize> = None;              // 再生
    let pending_next_path: Option<String> = None;        // 次に再生
    let pending_enqueue_path: Option<String> = None;     // キュー末尾に追加
    let pending_add_to_playlist: Option<(usize, Vec<String>)> = None; // プレイリストに追加
    let pending_show_details_path: Option<String> = None; // 詳細表示
    
    // プレイリスト一覧（追加先用）
    let playlist_targets: Vec<(usize, String)> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(idx, p)| (idx, p.name.clone()))
        .collect();
    
    let _meta_cache = MetaCache::open_default().ok();
    let mut _metadata_cache: HashMap<String, (String, String, String)> = HashMap::new();

    let current_play_path = app.play_path.clone();
    let selected_queue_indices = app.selected_queue_indices.clone();
    let selected_queue_indices_for_row = selected_queue_indices.clone();

    let track_list_outcome = render_track_list_with_artwork(
        ui,
        app,
        "queue_tracks_scroll",
        &tracks,
        &playlist_targets,
        TrackActivation::SingleClick,
        54.0,
        Some(ui.available_height()),
        |index, display| {
            if current_play_path == display.path {
                format!(
                    "{}: {} [Playing]\n{}\n{}",
                    index + 1,
                    display.title,
                    display.artist,
                    display.album,
                )
            } else {
                format!(
                    "{}: {}\n{}\n{}",
                    index + 1,
                    display.title,
                    display.artist,
                    display.album,
                )
            }
        },
        move |_, index, _, _| selected_queue_indices_for_row.contains(&index),
        render_track_artwork,
        |ui, app, index, _, _| {
            let handle_label = if app.queue_drag_source_index == Some(index) {
                "⇅"
            } else {
                "☰"
            };
            let handle_size = 18.0_f32;
            let handle_area_height = 54.0_f32;
            let handle_pad_y = ((handle_area_height - handle_size) * 0.5).max(0.0);

            let handle_response = ui
                .allocate_ui_with_layout(
                    egui::vec2(20.0, handle_area_height),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.add_space(handle_pad_y);
                        ui.add_sized([handle_size, handle_size], egui::Button::new(handle_label))
                    },
                )
                .inner
                .on_hover_text("Handle: click to select. Hold 350ms to start moving one track.");

            if let Some(source) = app.queue_drag_source_index {
                if handle_response.clicked() {
                    if source == index {
                        app.cancel_queue_drag();
                    } else {
                        app.move_queue_item_to(source, index);
                        app.cancel_queue_drag();
                    }
                }
            } else {
                if handle_response.is_pointer_button_down_on() {
                    if app.queue_handle_hold_index != Some(index) {
                        app.queue_handle_hold_index = Some(index);
                        app.queue_handle_hold_started_at = Some(Instant::now());
                    }

                    if let Some(started_at) = app.queue_handle_hold_started_at {
                        let elapsed = started_at.elapsed();
                        let hold_delay = Duration::from_millis(350);
                        if elapsed >= hold_delay {
                            app.queue_drag_source_index = Some(index);
                            app.select_queue_item_single(index);
                            app.clear_queue_handle_hold();
                        } else {
                            ui.ctx().request_repaint_after(hold_delay - elapsed);
                        }
                    }
                }

                if handle_response.clicked() {
                    if shift_pressed {
                        app.select_queue_item_range_to(index);
                    } else if ctrl_pressed {
                        app.toggle_queue_item_selection(index);
                    } else {
                        app.select_queue_item_single(index);
                    }
                }
            }
        },
        |response, app, index, _display, _track_path, _outcome| {
            if response.clicked() {
                if let Some(source) = app.queue_drag_source_index {
                    if source == index {
                        app.cancel_queue_drag();
                    } else {
                        app.move_queue_item_to(source, index);
                        app.cancel_queue_drag();
                    }
                    return true;
                }
            }

            if app.current_queue_index == Some(index) && app.queue_scroll_to_playing {
                response.scroll_to_me(Some(egui::Align::Center));
                app.queue_scroll_to_playing = false;
            }

            false
        },
        |ui, _app, index, _display, _track_path, outcome| {
            if ui.button("Play").clicked() {
                pending_play = Some(index);
            }
            if ui.button("Remove").clicked() {
                if selected_queue_indices.len() > 1 && selected_queue_indices.contains(&index) {
                    outcome.remove_indices = Some(selected_queue_indices.clone());
                } else {
                    outcome.remove_indices = Some(vec![index]);
                }
            }
            if ui.button("Up").clicked() {
                outcome.move_up_index = Some(index);
            }
            if ui.button("Down").clicked() {
                outcome.move_down_index = Some(index);
            }
        },
        |ui, _app, index, _display, _track_path, outcome| {
            if ui.button("Remove from queue").clicked() {
                if selected_queue_indices.len() > 1 && selected_queue_indices.contains(&index) {
                    outcome.remove_indices = Some(selected_queue_indices.clone());
                } else {
                    outcome.remove_indices = Some(vec![index]);
                }
                ui.close_menu();
            }
        },
    );

    if let Some(indices) = track_list_outcome.remove_indices {
        pending_remove_indices = Some(indices);
    }
    if let Some(index) = track_list_outcome.move_up_index {
        pending_move_up = Some(index);
    }
    if let Some(index) = track_list_outcome.move_down_index {
        pending_move_down = Some(index);
    }
    if let Some(index) = track_list_outcome.play_index {
        pending_play = Some(index);
    }


    // --- 保留中のアクションを実行 ---
    if let Some(index) = pending_move_up {
        app.move_queue_item_up(index);
    }
    if let Some(index) = pending_move_down {
        app.move_queue_item_down(index);
    }
    if let Some(indices) = pending_remove_indices {
        let mut indices = indices;
        // 降順ソートして後ろから削除（インデックスずれ防止）
        indices.sort_unstable_by(|a, b| b.cmp(a));
        indices.dedup();  // 重複除去
        for index in indices {
            app.remove_queue_item(index);
        }
        app.clear_queue_selection();
    }
    if let Some(index) = pending_play {
        app.select_queue_item_single(index);
        app.play_selected_queue_item();
    }
    if let Some(path) = pending_next_path {
        perform_add_track_next_in_queue(&mut *app, &path);
    }
    if let Some(path) = pending_enqueue_path {
        perform_enqueue_track_path(&mut *app, &path);
    }
    if let Some((target_index, tracks_to_add)) = pending_add_to_playlist {
        for track in tracks_to_add {
            perform_add_track_to_playlist(&mut *app, target_index, &track);
        }
    }
    if let Some(path) = pending_show_details_path {
        perform_open_track_details(&mut *app, &path);
    }

    // --- 一括操作ボタン ---
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Play selected").clicked() {
            app.play_selected_queue_item();
        }
        if ui.button("Remove selected").clicked() {
            app.remove_selected_queue_items();
        }
        if ui.button("Clear queue").clicked() {
            app.clear_queue();
        }
    });
}