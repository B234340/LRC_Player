use crate::presentation::components::track_context_menu::render_track_context_menu;
use crate::presentation::components::track_display::{render_track_artwork, resolve_track_display};
use crate::presentation::egui_app::GuiApp;
use eframe::egui;
use std::collections::HashMap;

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
        
        // メタデータキャッシュの準備
        let meta_cache = crate::infrastructure::persistence::MetaCache::open_default().ok();
        let mut metadata_cache: HashMap<String, (String, String, String)> = HashMap::new();
        
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
                
                // ========== スクロール可能なトラック一覧 ==========
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])  // コンテンツに合わせて縮小しない
                    .max_height(ui.available_height().max(120.0))  // 最大高さを設定
                    .show(ui, |ui| {
                        for (track_index, track) in playlist.tracks.iter().enumerate() {
                            // トラックの表示情報を解決
                            let display = resolve_track_display(track, meta_cache.as_ref(), &mut metadata_cache);
                            let is_playing = app.play_path == *track;  // 現在再生中かどうか

                            // 再生中トラックはハイライト表示
                            let mut frame = egui::Frame::group(ui.style());
                            if is_playing {
                                frame = frame
                                    .fill(ui.visuals().selection.bg_fill)
                                    .stroke(egui::Stroke::new(2.0, ui.visuals().selection.stroke.color));
                            }

                            frame.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // --- お気に入り切り替えボタン（★/☆）---
                                    let art_size = egui::vec2(44.0, 44.0);
                                    let handle_pad_y = ((art_size.y - 24.0) * 0.5).max(0.0);
                                    let toggle_response = ui
                                        .allocate_ui_with_layout(
                                            egui::vec2(28.0, art_size.y),
                                            egui::Layout::top_down(egui::Align::Center),
                                            |ui| {
                                                ui.add_space(handle_pad_y);
                                                let is_favorite = app.is_track_favorite(track);
                                                let star_label = if is_favorite { "★" } else { "☆" };
                                                ui.add_sized([24.0, 24.0], egui::Button::new(star_label))
                                            },
                                        )
                                        .inner
                                        .on_hover_text("Toggle favorite");  // ホバー時のツールチップ
                                    
                                    if toggle_response.clicked() {
                                        app.toggle_favorite_track(track);  // お気に入り状態を切り替え
                                    }
                                    ui.add_space(4.0);

                                    // --- アートワーク表示（44x44のサムネイル）---
                                    render_track_artwork(
                                        ui,
                                        track,
                                        &format!("playlist_art_{}_{}", selected_index, track_index),
                                        egui::vec2(44.0, 44.0),
                                        is_playing,
                                    );

                                    // --- トラック情報表示 ---
                                    ui.vertical(|ui| {
                                        // トラックタイトル（再生中の場合は [Playing] を付加）
                                        let title_text = if is_playing {
                                            format!("{}: {} [Playing]", track_index + 1, display.title)
                                        } else {
                                            format!("{}: {}", track_index + 1, display.title)
                                        };
                                        
                                        // トラック選択ラベル（クリックで再生開始）
                                        let response = ui.selectable_label(false, title_text);
                                        if response.clicked() {
                                            pending_play_index = Some(track_index);
                                            pending_play_list = playlist.tracks.clone();
                                        }
                                        
                                        // 右クリックコンテキストメニュー
                                        response.context_menu(|ui| {
                                            let menu_outcome = render_track_context_menu(
                                                ui,
                                                track,
                                                &playlist_targets,
                                                "Add to queue",  // キュー追加ボタンのラベル
                                                |_, _| {},       // 追加メニュー項目なし
                                            );
                                            
                                            // メニュー操作の結果を収集
                                            if let Some(path) = menu_outcome.play_now_path {
                                                pending_play_now_path = Some(path);
                                            }
                                            if let Some(path) = menu_outcome.next_path {
                                                pending_next_path = Some(path);
                                            }
                                            if let Some(path) = menu_outcome.enqueue_path {
                                                pending_enqueue_path = Some(path);
                                            }
                                            if let Some((target_index, path)) = menu_outcome.add_to_playlist {
                                                pending_add_to_playlist = Some((target_index, path));
                                            }
                                            if let Some(path) = menu_outcome.show_details_path {
                                                pending_show_details_path = Some(path);
                                            }
                                        });

                                        // アーティスト名とアルバム名を表示
                                        ui.label(format!("Artist: {}", display.artist));
                                        ui.label(format!("Album: {}", display.album));
                                    });

                                    // --- 右寄せ操作ボタン（Remove / Play）---
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        // プレイリストから削除ボタン
                                        if ui.button("Remove").clicked() {
                                            pending_remove_track = Some(track_index);
                                        }
                                        // 再生ボタン
                                        if ui.button("Play").clicked() {
                                            pending_play_index = Some(track_index);
                                            pending_play_list = playlist.tracks.clone();
                                        }
                                    });
                                });
                            });
                            ui.add_space(4.0);  // トラック間の余白
                        }
                    });
            }
        }

        // ========== トラック追加用の入力欄 ==========
        if selected_name.is_some() {
            ui.horizontal(|ui| {
                // 追加するトラックパスの入力フィールド
                ui.text_edit_singleline(&mut app.playlist_track_input);
                if ui.button("Add track").clicked() {
                    app.add_track_to_selected_playlist();  // 入力されたパスをプレイリストに追加
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
            app.remove_track_from_selected_playlist(track_index);
        }
        // リストから再生開始
        if let Some(index) = pending_play_index {
            app.play_from_displayed_list(&pending_play_list, index);
        }
        // コンテキストメニューからの各アクション
        if let Some(path) = pending_play_now_path {
            app.play_track_now(&path);  // 即時再生
        }
        if let Some(path) = pending_next_path {
            app.add_track_next_in_queue(&path);  // 次に再生
        }
        if let Some(path) = pending_enqueue_path {
            app.enqueue_track_path(&path);  // キュー末尾に追加
        }
        if let Some((target_index, track)) = pending_add_to_playlist {
            app.add_track_to_playlist(target_index, &track);  // 別プレイリストに追加
        }
        if let Some(path) = pending_show_details_path {
            app.open_track_details(&path);  // 詳細ダイアログを開く
        }
    }
}