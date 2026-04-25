use crate::presentation::actions::play::perform_play_track_now;
use crate::presentation::actions::queue::{
    perform_add_track_next_in_queue,
    perform_enqueue_track_path,
    perform_play_from_displayed_list,
};
use crate::presentation::actions::track::{
    apply_enqueue_action,
    apply_play_action,
    perform_add_tracks_to_favorite_playlist,
    perform_add_track_to_playlist,
    perform_open_track_details,
};
use crate::presentation::components::track_display::render_track_artwork_with_file_first;
use crate::presentation::components::track_list::{render_track_list_with_artwork, TrackActivation};
use crate::presentation::app::{ArtistViewMode, GuiApp};
use eframe::egui;

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
                        // タイトル表示
                        ui.heading("Tracks");
                        
                        if let Some(selected) = selected_artist.as_deref() {
                            // 選択中アーティストの名前を表示
                            ui.label(format!("Artist: {}", selected));
                            
                            // 右ペイン上部の操作ボタン：このアーティストの全トラックをお気に入りに追加
                            if !selected_artist_tracks.is_empty()
                                && ui.button("Add all to favorite").clicked()
                            {
                                pending_add_favorite = true;
                            }
                            ui.separator();

                            // トラックが存在しない場合のメッセージ表示
                            if selected_artist_tracks.is_empty() {
                                ui.label("No tracks found for this artist.");
                            } else {
                                // トラック一覧の操作説明
                                ui.label("Double left click: create queue from this list and play. Right click: common actions.");
                                ui.separator();

                                // 選択中アーティストのトラックパスリストを作成
                                let track_paths: Vec<String> = selected_artist_tracks
                                    .iter()
                                    .map(|track| track.id.clone())
                                    .collect();
                                let current_play_path = app.play_path.clone();

                                // トラック一覧を表示（アートワーク付き、2行表示、幅2.5行分）
                                let track_list_outcome = render_track_list_with_artwork(
                                    ui,
                                    app,
                                    "artist_tracks_scroll",
                                    &track_paths,
                                    &playlist_targets,
                                    TrackActivation::DoubleClick,
                                    ui.text_style_height(&egui::TextStyle::Body) * 2.5,
                                    22.0,
                                    Some(ui.available_height()),
                                    |index, display| {
                                        if current_play_path == display.path {
                                            format!(
                                                "{}: {} [Playing]\n{}",
                                                index + 1,
                                                display.title,
                                                display.album,
                                            )
                                        } else {
                                            format!(
                                                "{}: {}\n{}",
                                                index + 1,
                                                display.title,
                                                display.album,
                                            )
                                        }
                                    },
                                    |_, _, _, _| false,
                                    render_track_artwork_with_file_first,
                                    |_, _, _, _, _| {},
                                    |_, _, _, _, _| {},
                                    |_, _, _, _, _, _| false,
                                    |ui, _app, index, _display, track_path, outcome| {
                                        if ui.button("Play").clicked() {
                                            apply_play_action(outcome, index, &track_paths);
                                        }
                                        if ui.button("Add to queue").clicked() {
                                            apply_enqueue_action(outcome, track_path);
                                        }
                                    },
                                    |_, _, _, _, _, _| {},
                                );

                                pending_play_index = track_list_outcome.play_index;
                                pending_play_list = track_list_outcome.play_list;
                                pending_play_now_path = track_list_outcome.play_now_path;
                                pending_next_path = track_list_outcome.next_path;
                                pending_enqueue_path = track_list_outcome.enqueue_path;
                                pending_add_to_playlist = track_list_outcome.add_to_playlist;
                                pending_show_details_path = track_list_outcome.show_details_path;
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
        perform_play_from_displayed_list(&mut *app, &pending_play_list, index);  // リストから再生開始
    }
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
        perform_add_track_to_playlist(&mut *app, target_index, &track);  // 指定プレイリストに追加
    }
    if let Some(path) = pending_show_details_path {
        perform_open_track_details(&mut *app, &path);  // 詳細ダイアログを開く
    }
    if pending_add_favorite {
        // 選択中のアーティストの全トラックをお気に入りプレイリストに追加
        let tracks: Vec<String> = selected_artist_tracks
            .iter()
            .map(|track| track.title.clone())
            .collect();
        perform_add_tracks_to_favorite_playlist(&mut *app, &tracks);
    }
}