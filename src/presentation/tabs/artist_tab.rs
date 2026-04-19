use crate::infrastructure::persistence::MetaCache;
use crate::presentation::egui_app::{ArtistViewMode, GuiApp};
use eframe::egui;
use std::collections::HashMap;

/// アーティスト一覧画面のUIを描画する関数
/// 
/// 左側にアーティスト一覧、右側に選択されたアーティストのトラック一覧を表示する
/// 2ペイン形式のレイアウト
pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    // アプリケーション状態から必要なデータを取得
    let selected_artist = app.app_vm.library.selected_artist.clone();
    let selected_artist_tracks = app.app_vm.library.selected_artist_tracks.clone();
    
    // アーティスト名の一覧を抽出
    let artist_names: Vec<String> = app
        .app_vm
        .library
        .artists
        .iter()
        .map(|a| a.name.clone())
        .collect();
    
    // 遅延実行するアクションを保持する変数群
    let mut action: Option<Option<String>> = None;  // アーティスト選択アクション
    let mut pending_play_index: Option<usize> = None;  // 再生開始インデックス
    let mut pending_play_list: Vec<String> = Vec::new();  // 再生リスト
    let mut pending_play_now_path: Option<String> = None;  // 即時再生するトラックパス
    let mut pending_next_path: Option<String> = None;  // 次に再生するトラックパス
    let mut pending_enqueue_path: Option<String> = None;  // キューに追加するトラックパス
    let mut pending_add_to_playlist: Option<(usize, String)> = None;  // プレイリストに追加するトラック
    let mut pending_show_details_path: Option<String> = None;  // 詳細表示するトラックパス
    let mut pending_add_favorite = false;  // 全トラックをお気に入りに追加するフラグ
    
    // プレイリストの一覧を作成（インデックスと名前のペア）
    let playlist_targets: Vec<(usize, String)> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(idx, p)| (idx, p.name.clone()))
        .collect();
    
    // メタデータキャッシュを開く
    let meta_cache = MetaCache::open_default().ok();
    let mut track_metadata_cache: HashMap<String, (String, String)> = HashMap::new();

    // ヘッダー表示
    ui.heading("Artists");
    
    // アーティスト表示モードの切り替えボタン（アーティスト / アルバムアーティスト / 作曲者）
    ui.horizontal(|ui| {
        ui.label("Type:");
        let mut mode = app.artist_view_mode;
        
        // アーティストモード
        if ui
            .selectable_value(&mut mode, ArtistViewMode::Artist, ArtistViewMode::Artist.label())
            .changed()
        {
            app.set_artist_view_mode(mode);
        }
        
        // アルバムアーティストモード
        if ui
            .selectable_value(
                &mut mode,
                ArtistViewMode::AlbumArtist,
                ArtistViewMode::AlbumArtist.label(),
            )
            .changed()
        {
            app.set_artist_view_mode(mode);
        }
        
        // 作曲者モード
        if ui
            .selectable_value(&mut mode, ArtistViewMode::Composer, ArtistViewMode::Composer.label())
            .changed()
        {
            app.set_artist_view_mode(mode);
        }
    });
    
    ui.label(format!("{} artists in library", artist_names.len()));
    ui.separator();

    if artist_names.is_empty() {
        ui.label("No artists available.");
    } else {
        // レイアウトの寸法設定
        let list_row_height = 24.0_f32;  // アーティストリストの行の高さ
        let track_row_height = 22.0_f32;  // トラックリストの行の高さ
        let body_height = ui.available_height();  // 利用可能な高さ

        // メインの2ペインレイアウト（左：アーティストリスト、右：トラックリスト）
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), body_height),
            egui::Layout::left_to_right(egui::Align::Min),  // 左から右へのレイアウト
            |ui| {
                // 左ペインの幅を計算（全体の42%、最小220px、最大460px）
                let left_width = (ui.available_width() * 0.42).clamp(220.0, 460.0);

                // --- 左ペイン：アーティスト一覧 ---
                ui.allocate_ui_with_layout(
                    egui::vec2(left_width, body_height),
                    egui::Layout::top_down(egui::Align::Min),  // 上から下へのレイアウト
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Artist list");
                            // 選択解除ボタン
                            if ui.button("Clear").clicked() {
                                action = Some(None);
                            }
                        });
                        ui.separator();

                        let list_block_width = ui.available_width();
                        
                        // スクロール可能なアーティストリスト
                        egui::ScrollArea::vertical()
                            .id_source("artist_list_scroll")
                            .auto_shrink([false, false])
                            .max_width(list_block_width)
                            .max_height(ui.available_height())
                            .show_rows(ui, list_row_height, artist_names.len(), |ui, row_range| {
                                for row in row_range {
                                    let name = artist_names[row].as_str();
                                    let is_selected = selected_artist.as_deref() == Some(name);
                                    
                                    // 選択可能なラベルとしてアーティスト名を表示
                                    let row = egui::SelectableLabel::new(is_selected, name);
                                    if ui
                                        .add_sized([ui.available_width(), list_row_height], row)
                                        .clicked()
                                    {
                                        action = Some(Some(name.to_string()));  // アーティスト選択
                                    }
                                }
                            });
                    },
                );

                ui.separator();  // 左右ペイン間の区切り線

                // --- 右ペイン：選択されたアーティストのトラック一覧 ---
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), body_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.heading("Tracks");
                        
                        if let Some(selected) = selected_artist.as_deref() {
                            ui.label(format!("Artist: {}", selected));
                            
                            // 全トラックをお気に入りに追加するボタン
                            if !selected_artist_tracks.is_empty()
                                && ui.button("Add all to favorite").clicked()
                            {
                                pending_add_favorite = true;
                            }
                            ui.separator();

                            if selected_artist_tracks.is_empty() {
                                ui.label("No tracks found for this artist.");
                            } else {
                                ui.label("Double left click: create queue from this list and play. Right click: common actions.");
                                
                                // スクロール可能なトラックリスト
                                egui::ScrollArea::vertical()
                                    .id_source("artist_tracks_scroll")
                                    .auto_shrink([false, false])
                                    .max_height(ui.available_height())
                                    .show_rows(
                                        ui,
                                        track_row_height,
                                        selected_artist_tracks.len(),
                                        |ui, row_range| {
                                            // 表示用トラックリストを作成
                                            let displayed_tracks: Vec<String> = selected_artist_tracks
                                                .iter()
                                                .map(|track| track.title.clone())
                                                .collect();
                                            
                                            for row in row_range {
                                                let track_path = &selected_artist_tracks[row].title;
                                                
                                                // ファイル名を抽出
                                                let file_name = std::path::Path::new(track_path)
                                                    .file_name()
                                                    .and_then(|n| n.to_str())
                                                    .unwrap_or(track_path);

                                                // メタデータキャッシュからトラック情報（タイトル、アルバム）を取得
                                                let (title, album) = track_metadata_cache
                                                    .entry(track_path.clone())
                                                    .or_insert_with(|| {
                                                        meta_cache
                                                            .as_ref()
                                                            .and_then(|cache| {
                                                                cache.track_metadata_by_location(track_path)
                                                                    .ok()
                                                                    .flatten()
                                                            })
                                                            .map(|(t, _a, al)| {
                                                                // タイトルが空の場合はファイル名を使用
                                                                let t = if t.trim().is_empty() {
                                                                    file_name.to_string()
                                                                } else {
                                                                    t
                                                                };
                                                                // アルバム名が空の場合は "Unknown album" を使用
                                                                let al = if al.trim().is_empty() {
                                                                    "Unknown album".to_string()
                                                                } else {
                                                                    al
                                                                };
                                                                (t, al)
                                                            })
                                                            .unwrap_or_else(|| {
                                                                (file_name.to_string(), "Unknown album".to_string())
                                                            })
                                                    })
                                                    .clone();

                                                // 表示ラベル：「[アルバム名] トラックタイトル」形式
                                                let label = format!("[{}] {}", album, title);
                                                
                                                ui.horizontal(|ui| {
                                                    // お気に入り切り替えボタン（★/☆）
                                                    let toggle_response = ui
                                                        .allocate_ui_with_layout(
                                                            egui::vec2(28.0, 44.0),
                                                            egui::Layout::top_down(egui::Align::Center),
                                                            |ui| {
                                                                ui.add_sized(
                                                                    [22.0, 22.0],
                                                                    egui::Button::new(
                                                                        if app.is_track_favorite(track_path) { "★" } else { "☆" }
                                                                    ),
                                                                )
                                                            },
                                                        )
                                                        .inner
                                                        .on_hover_text("Toggle favorite");
                                                    
                                                    if toggle_response.clicked() {
                                                        app.toggle_favorite_track(track_path);
                                                    }
                                                    
                                                    ui.add_space(4.0);
                                                    
                                                    ui.vertical(|ui| {
                                                        let response = ui.selectable_label(false, label);
                                                        
                                                        // ダブルクリックで再生開始
                                                        if response.double_clicked() {
                                                            pending_play_index = Some(row);
                                                            pending_play_list = displayed_tracks.clone();
                                                        }
                                                        
                                                        // 右クリックコンテキストメニュー
                                                        response.context_menu(|ui| {
                                                            if ui.button("Play now").clicked() {
                                                                pending_play_now_path = Some(track_path.clone());
                                                                ui.close_menu();
                                                            }
                                                            if ui.button("Play next").clicked() {
                                                                pending_next_path = Some(track_path.clone());
                                                                ui.close_menu();
                                                            }
                                                            if ui.button("Add to queue").clicked() {
                                                                pending_enqueue_path = Some(track_path.clone());
                                                                ui.close_menu();
                                                            }
                                                            ui.separator();
                                                            ui.label("Add to playlist");
                                                            // 各プレイリストへの追加メニュー
                                                            for (target_index, target_name) in &playlist_targets {
                                                                if ui.button(target_name).clicked() {
                                                                    pending_add_to_playlist =
                                                                        Some((*target_index, track_path.clone()));
                                                                    ui.close_menu();
                                                                }
                                                            }
                                                            ui.separator();
                                                            if ui.button("Show details").clicked() {
                                                                pending_show_details_path = Some(track_path.clone());
                                                                ui.close_menu();
                                                            }
                                                        });
                                                    });
                                                });
                                            }
                                        },
                                    );
                            }
                        } else {
                            ui.label("Select an artist to view tracks.");
                        }
                    },
                );
            },
        );
    }

    // 保留中のアクションを実行
    if let Some(next) = action {
        app.select_artist(next);  // アーティスト選択
    }
    if let Some(index) = pending_play_index {
        app.play_from_displayed_list(&pending_play_list, index);  // リストから再生開始
    }
    if let Some(path) = pending_play_now_path {
        app.play_track_now(&path);  // 即時再生
    }
    if let Some(path) = pending_next_path {
        app.add_track_next_in_queue(&path);  // 次に再生
    }
    if let Some(path) = pending_enqueue_path {
        app.enqueue_track_path(&path);  // キューに追加
    }
    if let Some((target_index, track)) = pending_add_to_playlist {
        app.add_track_to_playlist(target_index, &track);  // プレイリストに追加
    }
    if let Some(path) = pending_show_details_path {
        app.open_track_details(&path);  // 詳細表示
    }
    if pending_add_favorite {
        // 選択中のアーティストの全トラックをお気に入りに追加
        let tracks: Vec<String> = selected_artist_tracks
            .iter()
            .map(|track| track.title.clone())
            .collect();
        app.add_tracks_to_favorite_playlist(&tracks);
    }
}