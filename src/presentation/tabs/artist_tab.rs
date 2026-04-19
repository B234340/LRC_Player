use crate::infrastructure::persistence::MetaCache;
use crate::presentation::components::track_display::{render_track_artwork, resolve_track_display};
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
                                ui.separator();

                                let track_paths: Vec<String> = selected_artist_tracks
                                    .iter()
                                    .map(|track| track.id.clone())
                                    .collect();
                                let meta_cache = MetaCache::open_default().ok();
                                let mut metadata_cache: HashMap<String, (String, String, String)> = HashMap::new();

                                egui::ScrollArea::vertical()
                                    .id_source("artist_tracks_scroll")
                                    .auto_shrink([false, false])
                                    .max_height(ui.available_height())
                                    .show(ui, |ui| {
                                        for (track_index, track_path) in track_paths.iter().enumerate() {
                                            let display = resolve_track_display(
                                                track_path,
                                                meta_cache.as_ref(),
                                                &mut metadata_cache,
                                            );
                                            let is_playing = app.play_path == *track_path;

                                            let mut frame = egui::Frame::group(ui.style());
                                            if is_playing {
                                                frame = frame
                                                    .fill(ui.visuals().selection.bg_fill)
                                                    .stroke(egui::Stroke::new(2.0, ui.visuals().selection.stroke.color));
                                            }

                                            frame.show(ui, |ui| {
                                                ui.horizontal(|ui| {
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
                                                    render_track_artwork(
                                                        ui,
                                                        track_path,
                                                        &format!("artist_track_art_{}", track_index),
                                                        egui::vec2(44.0, 44.0),
                                                        is_playing,
                                                    );
                                                    ui.add_space(8.0);

                                                    ui.vertical(|ui| {
                                                        let title_text = if is_playing {
                                                            format!("{} [Playing]", display.title)
                                                        } else {
                                                            display.title.clone()
                                                        };
                                                        let title_size = ui.style().text_styles[&egui::TextStyle::Body].size * 1.5;
                                                        let response = ui
                                                            .selectable_label(false, egui::RichText::new(title_text).size(title_size))
                                                            .on_hover_text("Double click to play. Right click for actions.");
                                                        if response.double_clicked() {
                                                            pending_play_index = Some(track_index);
                                                            pending_play_list = track_paths.clone();
                                                        }
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
                                                            for (target_index, target_name) in &playlist_targets {
                                                                if ui.button(target_name).clicked() {
                                                                    pending_add_to_playlist = Some((*target_index, track_path.clone()));
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

                                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                        if ui.button("Play").clicked() {
                                                            pending_play_index = Some(track_index);
                                                            pending_play_list = track_paths.clone();
                                                        }
                                                    });
                                                });
                                            });
                                            ui.add_space(4.0);
                                        }
                                    });
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