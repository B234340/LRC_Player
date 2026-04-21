use crate::domain::model::PlaybackMode;
use crate::infrastructure::persistence::MetaCache;
use crate::presentation::components::track_context_menu::render_track_context_menu;
use crate::presentation::components::track_display::{render_track_artwork, resolve_track_display};
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
    let mut pending_next_path: Option<String> = None;        // 次に再生
    let mut pending_enqueue_path: Option<String> = None;     // キュー末尾に追加
    let mut pending_add_to_playlist: Option<(usize, Vec<String>)> = None; // プレイリストに追加
    let mut pending_show_details_path: Option<String> = None; // 詳細表示
    
    // プレイリスト一覧（追加先用）
    let playlist_targets: Vec<(usize, String)> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(idx, p)| (idx, p.name.clone()))
        .collect();
    
    let meta_cache = MetaCache::open_default().ok();
    let mut metadata_cache: HashMap<String, (String, String, String)> = HashMap::new();

    // --- スクロール可能なキュー一覧 ---
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, track) in tracks.iter().enumerate() {
                let is_selected = app.is_queue_item_selected(index);  // 選択状態
                let is_playing = app.current_queue_index == Some(index);  // 再生中かどうか
                let display = resolve_track_display(track, meta_cache.as_ref(), &mut metadata_cache);
                let title = display.title.as_str();
                let artist = display.artist.as_str();
                let album = display.album.as_str();

                // 状態に応じてフレームスタイルを変更
                let mut frame = egui::Frame::group(ui.style());
                if is_playing {
                    // 再生中：選択色で塗りつぶし、太枠
                    frame = frame
                        .fill(ui.visuals().selection.bg_fill)
                        .stroke(egui::Stroke::new(2.0, ui.visuals().selection.stroke.color));
                } else if is_selected {
                    // 選択中：細枠
                    frame = frame.stroke(egui::Stroke::new(1.0, ui.visuals().widgets.active.bg_stroke.color));
                }
                if app.queue_drag_source_index == Some(index) {
                    // ドラッグ元：警告色の太枠
                    frame = frame.stroke(egui::Stroke::new(2.0, ui.visuals().warn_fg_color));
                }

                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // --- ドラッグハンドル（☰ / ⇅）---
                        let art_size = egui::vec2(44.0, 44.0);
                        let handle_label = if app.queue_drag_source_index == Some(index) {
                            "⇅"  // 移動中アイコン
                        } else {
                            "☰"  // 通常ハンドルアイコン
                        };
                        let handle_size = 18.0_f32;
                        let handle_pad_y = ((art_size.y - handle_size) * 0.5).max(0.0);
                        
                        let handle_response = ui
                            .allocate_ui_with_layout(
                                egui::vec2(20.0, art_size.y),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    ui.add_space(handle_pad_y);
                                    ui.add_sized([handle_size, handle_size], egui::Button::new(handle_label))
                                },
                            )
                            .inner
                            .on_hover_text("Handle: click to select. Hold 350ms to start moving one track.");

                        // ドラッグ操作の処理
                        if let Some(source) = app.queue_drag_source_index {
                            // ドラッグ中：クリックで移動先を決定
                            if handle_response.clicked() {
                                if source == index {
                                    app.cancel_queue_drag();  // 同じ位置ならキャンセル
                                } else {
                                    app.move_queue_item_to(source, index);  // 移動実行
                                    app.cancel_queue_drag();
                                }
                            }
                        } else {
                            // 通常時：長押し検出と選択処理
                            if handle_response.is_pointer_button_down_on() {
                                // 長押し開始を記録
                                if app.queue_handle_hold_index != Some(index) {
                                    app.queue_handle_hold_index = Some(index);
                                    app.queue_handle_hold_started_at = Some(Instant::now());
                                }

                                // 350ms以上押し続けたらドラッグモード開始
                                if let Some(started_at) = app.queue_handle_hold_started_at {
                                    let elapsed = started_at.elapsed();
                                    let hold_delay = Duration::from_millis(350);
                                    if elapsed >= hold_delay {
                                        app.queue_drag_source_index = Some(index);
                                        app.select_queue_item_single(index);
                                        app.clear_queue_handle_hold();
                                    } else {
                                        // 再描画を予約
                                        ui.ctx().request_repaint_after(hold_delay - elapsed);
                                    }
                                }
                            }

                            // クリック時の選択処理（修飾キーに応じて動作変更）
                            if handle_response.clicked() {
                                if shift_pressed {
                                    app.select_queue_item_range_to(index);  // Shift: 範囲選択
                                } else if ctrl_pressed {
                                    app.toggle_queue_item_selection(index);  // Ctrl: 選択切り替え
                                } else {
                                    app.select_queue_item_single(index);  // 通常: 単一選択
                                }
                            }
                        }

                        // --- アートワーク表示 ---
                        render_track_artwork(
                            ui,
                            track,
                            &format!("queue_art_{}", index),
                            art_size,
                            is_playing,
                        );

                        // --- トラック情報表示 ---
                        ui.vertical(|ui| {
                            let response = ui.horizontal(|ui| {
                                // トラックタイトル（再生中の場合は [Playing] を付加）
                                let title_text = if is_playing {
                                    format!("{}: {} [Playing]", index + 1, title)
                                } else {
                                    format!("{}: {}", index + 1, title)
                                };
                                let response = ui.selectable_label(is_selected, title_text).on_hover_text(
                                    "Left click: play now (queue order unchanged). Right click: common actions.",
                                );

                                // 複数選択時の対象トラックリストを作成
                                let selected_track_paths: Vec<String> = if app.selected_queue_indices.len() > 1 && app.is_queue_item_selected(index) {
                                    app.selected_queue_indices
                                        .iter()
                                        .filter_map(|&selected_index| tracks.get(selected_index).cloned())
                                        .collect()
                                } else {
                                    vec![track.clone()]
                                };

                                // クリック時の処理
                                if response.clicked() {
                                    if let Some(source) = app.queue_drag_source_index {
                                        // ドラッグ中：移動先として処理
                                        if source == index {
                                            app.cancel_queue_drag();
                                        } else {
                                            app.move_queue_item_to(source, index);
                                            app.cancel_queue_drag();
                                        }
                                    } else {
                                        // 通常時：選択して再生
                                        app.select_queue_item_single(index);
                                        app.play_selected_queue_item();
                                    }
                                }
                                
                                // 右クリックコンテキストメニュー
                                response.clone().context_menu(|ui| {
                                    let menu_outcome = render_track_context_menu(
                                        ui,
                                        track,
                                        &playlist_targets,
                                        "Add to queue tail",
                                        |_, _| {},
                                    );
                                    if menu_outcome.play_now_path.is_some() {
                                        pending_play = Some(index);
                                    }
                                    if let Some(path) = menu_outcome.next_path {
                                        pending_next_path = Some(path);
                                    }
                                    if let Some(path) = menu_outcome.enqueue_path {
                                        pending_enqueue_path = Some(path);
                                    }
                                    if let Some((target_index, _path)) = menu_outcome.add_to_playlist {
                                        // queue tab uses selected track paths when multiple are selected
                                        if app.selected_queue_indices.len() > 1 && app.is_queue_item_selected(index) {
                                            pending_add_to_playlist = Some((target_index, selected_track_paths.clone()));
                                        } else {
                                            pending_add_to_playlist = Some((target_index, vec![track.clone()]));
                                        }
                                    }
                                    if let Some(path) = menu_outcome.show_details_path {
                                        pending_show_details_path = Some(path);
                                    }
                                    ui.separator();
                                    if ui.button("Remove from queue").clicked() {
                                        if app.selected_queue_indices.len() > 1 && app.is_queue_item_selected(index) {
                                            pending_remove_indices = Some(app.selected_queue_indices.clone());
                                        } else {
                                            pending_remove_indices = Some(vec![index]);
                                        }
                                        ui.close_menu();
                                    }
                                });
                                response
                            })
                            .inner;

                            // アーティスト名とアルバム名を表示
                            ui.label(format!("Artist: {}", artist));
                            ui.label(format!("Album: {}", album));

                            // 再生中トラックへの自動スクロール
                            if is_playing && app.queue_scroll_to_playing {
                                response.scroll_to_me(Some(egui::Align::Center));
                                app.queue_scroll_to_playing = false;
                            }
                        });

                        // --- 右寄せ操作ボタン ---
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Play").clicked() {
                                pending_play = Some(index);
                            }
                            if ui.button("Remove").clicked() {
                                if app.selected_queue_indices.len() > 1 && app.is_queue_item_selected(index) {
                                    pending_remove_indices = Some(app.selected_queue_indices.clone());
                                } else {
                                    pending_remove_indices = Some(vec![index]);
                                }
                            }
                            if ui.button("Up").clicked() {
                                pending_move_up = Some(index);
                            }
                            if ui.button("Down").clicked() {
                                pending_move_down = Some(index);
                            }
                        });
                    });
                });

                ui.add_space(4.0);
            }
        });

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
        app.add_track_next_in_queue(&path);
    }
    if let Some(path) = pending_enqueue_path {
        app.enqueue_track_path(&path);
    }
    if let Some((target_index, tracks_to_add)) = pending_add_to_playlist {
        for track in tracks_to_add {
            app.add_track_to_playlist(target_index, &track);
        }
    }
    if let Some(path) = pending_show_details_path {
        app.open_track_details(&path);
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