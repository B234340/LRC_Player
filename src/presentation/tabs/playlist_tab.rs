use crate::presentation::components::track_display::render_track_artwork;
use crate::presentation::components::track_actions::{
    apply_enqueue_action,
    apply_play_action,
    apply_remove_action,
    apply_toggle_favorite_action,
    perform_add_track_next_in_queue,
    perform_add_track_to_playlist,
    perform_add_track_to_selected_playlist,
    perform_enqueue_track_path,
    perform_open_track_details,
    perform_play_from_displayed_list,
    perform_play_track_now,
    perform_remove_track_from_selected_playlist,
};
use crate::presentation::components::track_list::{render_track_list_with_artwork, TrackActivation};
use crate::presentation::egui_app::GuiApp;
use eframe::egui;

/// プレイリスト画面のUIを描画する関数
/// 
/// この画面には以下の要素が含まれる：
/// - プレイリスト一覧（選択、削除、並び替え）
/// - 新規プレイリスト作成
/// - 選択プレイリスト内のトラック一覧
/// - トラックの追加/削除操作
pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    // ========== ヘッダー表示 ==========
    ui.heading("Playlists");
    ui.label(format!("Saved playlists: {}", app.playlists.len()));
    ui.separator();

    // ========== 新規プレイリスト作成 ==========
    ui.horizontal(|ui| {
        // プレイリスト名入力フィールド
        ui.text_edit_singleline(&mut app.new_playlist_name);
        if ui.button("Create playlist").clicked() {
            app.add_playlist();  // 入力された名前で新規プレイリストを作成
        }
    });

    // プレイリストが存在しない場合は早期リターン
    if app.playlists.is_empty() {
        ui.label("No playlists available.");
        return;
    }

    ui.separator();
    
    // ========== 遅延実行するアクションを保持する変数群 ==========
    // UI描画中に直接状態を変更するとeguiの制約に違反するため、
    // アクションを一時変数に保存し、描画完了後に実行する
    
    // プレイリスト選択アクション（Some(インデックス)で選択、Noneで選択解除）
    let mut pending_select_playlist: Option<Option<usize>> = None;
    let mut pending_delete_playlist: Option<usize> = None;      // プレイリスト削除
    let mut pending_move_playlist_up: Option<usize> = None;     // プレイリストを上に移動
    let mut pending_move_playlist_down: Option<usize> = None;   // プレイリストを下に移動
    
    // ========== プレイリスト一覧の表示 ==========
    for (index, playlist) in app.playlists.iter().enumerate() {
        let selected = app.selected_playlist == Some(index);
        // "favorite" という名前のプレイリストは特別扱い（削除・並び替え不可）
        let is_favorite = playlist.name.eq_ignore_ascii_case("favorite");
        
        // 選択状態に応じてフレームのスタイルを変更（ハイライト表示）
        let mut frame = egui::Frame::group(ui.style());
        if selected {
            frame = frame
                .fill(ui.visuals().selection.bg_fill)  // 選択時の背景色
                .stroke(egui::Stroke::new(2.0, ui.visuals().selection.stroke.color));
        }

        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                // お気に入りプレイリストの場合は★マークを付けて視覚的に区別
                let label = if is_favorite {
                    format!("★ {}", playlist.name)
                } else {
                    playlist.name.clone()
                };
                
                // プレイリスト選択ラベル（クリックで選択/選択解除）
                let response = ui.add(egui::SelectableLabel::new(selected, label));
                if response.clicked() {
                    if selected {
                        pending_select_playlist = Some(None);  // 選択解除
                    } else {
                        pending_select_playlist = Some(Some(index));  // 選択
                    }
                }
                
                // 右寄せで操作ボタンを配置
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 下に移動ボタン
                    // お気に入り以外、かつ最後の要素以外の場合のみ有効
                    if ui.add_enabled(!is_favorite && index < app.playlists.len() - 1, egui::Button::new("↓")).clicked() {
                        pending_move_playlist_down = Some(index);
                    }
                    // 上に移動ボタン
                    // お気に入り以外、かつインデックス1以上の場合のみ有効
                    // （インデックス0はお気に入りプレイリストの可能性があるため1以上とする）
                    if ui.add_enabled(index > 1, egui::Button::new("↑")).clicked() {
                        pending_move_playlist_up = Some(index);
                    }
                    // 削除ボタン（お気に入り以外のみ削除可能）
                    if ui.add_enabled(!is_favorite, egui::Button::new("Delete")).clicked() {
                        pending_delete_playlist = Some(index);
                    }
                });
            });
        });
        ui.add_space(4.0);  // プレイリスト間の余白
    }

    // ========== プレイリスト削除アクションの実行 ==========
    // 削除は即時実行（選択状態などが変わるため早期リターン）
    if let Some(index) = pending_delete_playlist {
        app.remove_playlist(index);
        return;
    }

    // ========== プレイリスト並び替えアクションの実行 ==========
    if let Some(index) = pending_move_playlist_up {
        app.move_playlist_up(index);
    }
    if let Some(index) = pending_move_playlist_down {
        app.move_playlist_down(index);
    }

    // ========== プレイリスト選択アクションの実行 ==========
    if let Some(selection) = pending_select_playlist {
        app.selected_playlist = selection;
        app.update_app_vm();  // ViewModelを更新
    }

    // ========== 選択中のプレイリストの詳細表示 ==========
    if let Some(selected_index) = app.selected_playlist {
        let mut selected_name: Option<String> = None;
        let mut selected_len: usize = 0;
        
        // トラック操作の遅延アクション変数
        let mut pending_remove_track: Option<usize> = None;           // トラック削除
        let mut pending_play_index: Option<usize> = None;             // 再生開始インデックス
        let mut pending_play_list: Vec<String> = Vec::new();          // 再生リスト
        let mut pending_play_now_path: Option<String> = None;         // 即時再生
        let mut pending_next_path: Option<String> = None;             // 次に再生
        let mut pending_enqueue_path: Option<String> = None;          // キューに追加
        let mut pending_add_to_playlist: Option<(usize, String)> = None;  // 別プレイリストに追加
        let mut pending_show_details_path: Option<String> = None;     // 詳細表示
        
        // プレイリスト一覧（コンテキストメニューの「プレイリストに追加」用）
        let playlist_targets: Vec<(usize, String)> = app
            .playlists
            .iter()
            .enumerate()
            .map(|(idx, p)| (idx, p.name.clone()))
            .collect();
        
        let playlist = app.playlists.get(selected_index).cloned();
        if let Some(playlist) = playlist {
            selected_name = Some(playlist.name.clone());
            selected_len = playlist.tracks.len();
            
            ui.separator();
            ui.heading(format!("Playlist: {}", playlist.name));
            ui.label(format!("{} tracks", playlist.tracks.len()));

            if playlist.tracks.is_empty() {
                ui.label("No tracks in this playlist.");
            } else {
                ui.label("Left click: create queue from this list and play. Right click: common actions.");
                let current_play_path = app.play_path.clone();

                let track_list_outcome = render_track_list_with_artwork(
                    ui,
                    app,
                    "playlist_tracks_scroll",
                    &playlist.tracks,
                    &playlist_targets,
                    TrackActivation::SingleClick,
                    54.0,
                    Some(ui.available_height().max(120.0)),
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
                    |_, _, _, _| false,
                    render_track_artwork,
                    |_, _, _, _, _| {},
                    |_, _, _, _, _, _| false,
                    |ui, app, index, _display, track_path, outcome| {
                        if ui.button("Remove").clicked() {
                            apply_remove_action(outcome, index);
                        }
                        if ui.button("Play").clicked() {
                            apply_play_action(outcome, index, &playlist.tracks);
                        }
                        if ui.button("Add to queue").clicked() {
                            apply_enqueue_action(outcome, track_path);
                        }
                        if ui.button("Favorite").clicked() {
                            apply_toggle_favorite_action(app, track_path);
                        }
                    },
                    |_, _, _, _, _, _| {},
                );

                if let Some(indices) = track_list_outcome.remove_indices {
                    if let Some(index) = indices.into_iter().next() {
                        pending_remove_track = Some(index);
                    }
                }
                pending_play_index = track_list_outcome.play_index;
                pending_play_list = track_list_outcome.play_list;
                pending_play_now_path = track_list_outcome.play_now_path;
                pending_next_path = track_list_outcome.next_path;
                pending_enqueue_path = track_list_outcome.enqueue_path;
                pending_add_to_playlist = track_list_outcome.add_to_playlist;
                pending_show_details_path = track_list_outcome.show_details_path;
            }
        }

        // ========== トラック追加用の入力欄 ==========
        if selected_name.is_some() {
            ui.horizontal(|ui| {
                // 追加するトラックパスの入力フィールド
                ui.text_edit_singleline(&mut app.playlist_track_input);
                if ui.button("Add track").clicked() {
                    perform_add_track_to_selected_playlist(&mut *app);  // 入力されたパスをプレイリストに追加
                }
                // プレイリストが空の場合のみ「(empty)」と表示
                if selected_len == 0 {
                    ui.label("(empty)");
                }
            });
        }

        // ========== 保留中のアクションを実行 ==========
        // トラック削除
        if let Some(track_index) = pending_remove_track {
            perform_remove_track_from_selected_playlist(&mut *app, track_index);
        }
        // リストから再生開始
        if let Some(index) = pending_play_index {
            perform_play_from_displayed_list(&mut *app, &pending_play_list, index);
        }
        // コンテキストメニューからの各アクション
        if let Some(path) = pending_play_now_path {
            perform_play_track_now(&mut *app, &path);  // 即時再生
        }
        if let Some(path) = pending_next_path {
            perform_add_track_next_in_queue(&mut *app, &path);  // 次に再生
        }
        if let Some(path) = pending_enqueue_path {
            perform_enqueue_track_path(&mut *app, &path);  // キュー末尾に追加
        }
        if let Some((target_index, track)) = pending_add_to_playlist {
            perform_add_track_to_playlist(&mut *app, target_index, &track);  // 別プレイリストに追加
        }
        if let Some(path) = pending_show_details_path {
            perform_open_track_details(&mut *app, &path);  // 詳細ダイアログを開く
        }
    }
}