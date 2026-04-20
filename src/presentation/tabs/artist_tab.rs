use crate::infrastructure::persistence::MetaCache;
use crate::presentation::components::track_context_menu::render_track_context_menu;
use crate::presentation::components::track_display::{
    render_track_artwork_with_metadata_first,
    resolve_track_display,
};
use crate::presentation::egui_app::{ArtistViewMode, GuiApp};
use eframe::egui;
use std::collections::HashMap;

/// アーティスト一覧画面のUIを描画する関数
/// 
/// この画面は2ペイン形式のレイアウトで構成される：
/// - 左ペイン：アーティスト一覧（スクロール可能な選択リスト）
/// - 右ペイン：選択されたアーティストのトラック一覧
/// 
/// アーティスト表示モードは以下の3種類から選択可能：
/// - Artist（アーティスト）
/// - Album Artist（アルバムアーティスト）
/// - Composer（作曲者）
pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    // ========== アプリケーション状態から必要なデータを取得 ==========
    let selected_artist = app.app_vm.library.selected_artist.clone();
    let selected_artist_tracks = app.app_vm.library.selected_artist_tracks.clone();
    
    // アーティスト名の一覧をViewModelから抽出
    let artist_names: Vec<String> = app
        .app_vm
        .library
        .artists
        .iter()
        .map(|a| a.name.clone())
        .collect();
    
    // ========== 遅延実行するアクションを保持する変数群 ==========
    // UI描画中に直接状態を変更するとeguiの制約に違反するため、
    // アクションを一時変数に保存し、描画完了後に実行する
    
    // アーティスト選択アクション（Some(アーティスト名)で選択、Noneで選択解除）
    let mut action: Option<Option<String>> = None;
    let mut pending_play_index: Option<usize> = None;           // 再生開始インデックス
    let mut pending_play_list: Vec<String> = Vec::new();        // 再生リスト
    let mut pending_play_now_path: Option<String> = None;       // 即時再生するトラックパス
    let mut pending_next_path: Option<String> = None;           // 次に再生するトラックパス
    let mut pending_enqueue_path: Option<String> = None;        // キューに追加するトラックパス
    let mut pending_add_to_playlist: Option<(usize, String)> = None;  // プレイリストに追加するトラック
    let mut pending_show_details_path: Option<String> = None;   // 詳細表示するトラックパス
    let mut pending_add_favorite = false;  // 全トラックをお気に入りに追加するフラグ
    
    // ========== プレイリスト一覧の準備 ==========
    // コンテキストメニュー用に (インデックス, プレイリスト名) のペアを作成
    let playlist_targets: Vec<(usize, String)> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(idx, p)| (idx, p.name.clone()))
        .collect();
    
    // ========== ヘッダー表示 ==========
    ui.heading("Artists");
    
    // アーティスト表示モードの切り替えボタン
    // モードによって表示されるアーティストの種類が変わる：
    // - Artist: トラックのアーティスト名
    // - Album Artist: アルバムアーティスト名
    // - Composer: 作曲者名
    ui.horizontal(|ui| {
        ui.label("Type:");
        let mut mode = app.artist_view_mode;
        
        // アーティストモード選択ボタン
        if ui
            .selectable_value(&mut mode, ArtistViewMode::Artist, ArtistViewMode::Artist.label())
            .changed()
        {
            app.set_artist_view_mode(mode);
        }
        
        // アルバムアーティストモード選択ボタン
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
        
        // 作曲者モード選択ボタン
        if ui
            .selectable_value(&mut mode, ArtistViewMode::Composer, ArtistViewMode::Composer.label())
            .changed()
        {
            app.set_artist_view_mode(mode);
        }
    });
    
    ui.label(format!("{} artists in library", artist_names.len()));
    ui.separator();

    // アーティストが存在しない場合の表示
    if artist_names.is_empty() {
        ui.label("No artists available.");
    } else {
        // ========== レイアウトの寸法設定 ==========
        let list_row_height = 24.0_f32;      // アーティストリストの行の高さ
        let body_height = ui.available_height();  // 利用可能な高さ

        // メインの2ペインレイアウト（左：アーティストリスト、右：トラックリスト）
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), body_height),
            egui::Layout::left_to_right(egui::Align::Min),  // 左から右へのレイアウト
            |ui| {
                // 左ペインの幅を計算（全体の42%、最小220px、最大460px）
                let left_width = (ui.available_width() * 0.42).clamp(220.0, 460.0);

                // ========== 左ペイン：アーティスト一覧 ==========
                ui.allocate_ui_with_layout(
                    egui::vec2(left_width, body_height),
                    egui::Layout::top_down(egui::Align::Min),  // 上から下へのレイアウト
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Artist list");
                            // 選択解除ボタン（アーティスト選択をクリア）
                            if ui.button("Clear").clicked() {
                                action = Some(None);
                            }
                        });
                        ui.separator();

                        let list_block_width = ui.available_width();
                        
                        // スクロール可能なアーティストリスト（仮想スクロールで効率的に表示）
                        egui::ScrollArea::vertical()
                            .id_source("artist_list_scroll")  // スクロール位置記憶用の一意識別子
                            .auto_shrink([false, false])      // コンテンツに合わせて縮小しない
                            .max_width(list_block_width)
                            .max_height(ui.available_height())
                            .show_rows(ui, list_row_height, artist_names.len(), |ui, row_range| {
                                // 表示範囲内の行のみを描画（仮想スクロール）
                                for row in row_range {
                                    let name = artist_names[row].as_str();
                                    let is_selected = selected_artist.as_deref() == Some(name);
                                    
                                    // 選択可能なラベルとしてアーティスト名を表示
                                    let row = egui::SelectableLabel::new(is_selected, name);
                                    if ui
                                        .add_sized([ui.available_width(), list_row_height], row)
                                        .clicked()
                                    {
                                        // クリックされたアーティストを選択（アクションを保留）
                                        action = Some(Some(name.to_string()));
                                    }
                                }
                            });
                    },
                );

                ui.separator();  // 左右ペイン間の区切り線

                // ========== 右ペイン：選択されたアーティストのトラック一覧 ==========
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
                                ui.separator();

                                // トラックパスのリストを作成
                                let track_paths: Vec<String> = selected_artist_tracks
                                    .iter()
                                    .map(|track| track.id.clone())
                                    .collect();
                                
                                // メタデータキャッシュの準備
                                let meta_cache = MetaCache::open_default().ok();
                                let mut metadata_cache: HashMap<String, (String, String, String)> = HashMap::new();

                                // スクロール可能なトラックリスト
                                egui::ScrollArea::vertical()
                                    .id_source("artist_tracks_scroll")
                                    .auto_shrink([false, false])
                                    .max_height(ui.available_height())
                                    .show_rows(ui, 52.0, track_paths.len(), |ui, row_range| {
                                        for track_index in row_range {
                                            let track_path = &track_paths[track_index];
                                            // トラックの表示情報を解決
                                            let display = resolve_track_display(
                                                track_path,
                                                meta_cache.as_ref(),
                                                &mut metadata_cache,
                                            );
                                            let is_playing = app.play_path == *track_path;  // 現在再生中かどうか

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
                                                    let is_favorite = app.is_track_favorite(track_path);
                                                    let star_label = if is_favorite { "★" } else { "☆" };
                                                    let star_area_height = 44.0_f32;
                                                    let vertical_pad = ((star_area_height - 22.0_f32) * 0.5).max(0.0_f32);
                                                    let toggle_response = ui
                                                        .allocate_ui_with_layout(
                                                            egui::vec2(28.0, star_area_height),
                                                            egui::Layout::top_down(egui::Align::Center),
                                                            |ui| {
                                                                ui.add_space(vertical_pad);
                                                                ui.add_sized([22.0, 22.0], egui::Button::new(star_label))
                                                            },
                                                        )
                                                        .inner
                                                        .on_hover_text("Toggle favorite");
                                                    
                                                    if toggle_response.clicked() {
                                                        app.toggle_favorite_track(track_path);
                                                    }

                                                    ui.add_space(8.0);
                                                    
                                                    // --- アートワーク表示（44x44のサムネイル）---
                                                    render_track_artwork_with_metadata_first(
                                                        ui,
                                                        track_path,
                                                        &format!("artist_track_art_{}", track_index),
                                                        egui::vec2(44.0, 44.0),
                                                        is_playing,
                                                    );
                                                    ui.add_space(8.0);

                                                    // --- トラック情報表示 ---
                                                    ui.vertical(|ui| {
                                                        // トラックタイトル（再生中の場合は [Playing] を付加）
                                                        let title_text = if is_playing {
                                                            format!("{} [Playing]", display.title)
                                                        } else {
                                                            display.title.clone()
                                                        };
                                                        
                                                        // タイトルは通常より大きめのフォントサイズで表示
                                                        let title_size = ui.style().text_styles[&egui::TextStyle::Body].size * 1.5;
                                                        let response = ui
                                                            .selectable_label(false, egui::RichText::new(title_text).size(title_size))
                                                            .on_hover_text("Double click to play. Right click for actions.");
                                                        
                                                        // ダブルクリックで再生開始
                                                        if response.double_clicked() {
                                                            pending_play_index = Some(track_index);
                                                            pending_play_list = track_paths.clone();
                                                        }
                                                        
                                                        // 右クリックコンテキストメニュー
                                                        response.context_menu(|ui| {
                                                            let menu_outcome = render_track_context_menu(
                                                                ui,
                                                                track_path,
                                                                &playlist_targets,
                                                                "Add to queue",
                                                                |_, _| {},
                                                            );
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
                                                    });

                                                    // --- 右寄せ操作ボタン（Play）---
                                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                        if ui.button("Play").clicked() {
                                                            pending_play_index = Some(track_index);
                                                            pending_play_list = track_paths.clone();
                                                        }
                                                    });
                                                });
                                            });
                                            ui.add_space(4.0);  // トラック間の余白
                                        }
                                    });
                            }
                        } else {
                            // アーティストが選択されていない場合のメッセージ
                            ui.label("Select an artist to view tracks.");
                        }
                    },
                );
            },
        );
    }

    // ========== 保留中のアクションを実行 ==========
    // UI描画完了後に実際の状態変更を行う
    
    if let Some(next) = action {
        app.select_artist(next);  // アーティスト選択/選択解除
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
        app.enqueue_track_path(&path);  // キュー末尾に追加
    }
    if let Some((target_index, track)) = pending_add_to_playlist {
        app.add_track_to_playlist(target_index, &track);  // 指定プレイリストに追加
    }
    if let Some(path) = pending_show_details_path {
        app.open_track_details(&path);  // 詳細ダイアログを開く
    }
    if pending_add_favorite {
        // 選択中のアーティストの全トラックをお気に入りプレイリストに追加
        let tracks: Vec<String> = selected_artist_tracks
            .iter()
            .map(|track| track.title.clone())
            .collect();
        app.add_tracks_to_favorite_playlist(&tracks);
    }
}