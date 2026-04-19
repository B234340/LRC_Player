use crate::presentation::components::track_list::{
    render_standard_track_list, TrackActivation,
};
use crate::presentation::egui_app::{ArtistViewMode, GuiApp};
use eframe::egui;

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
                                
                                let tracks: Vec<String> = selected_artist_tracks
                                    .iter()
                                    .map(|track| track.title.clone())
                                    .collect();
                                let track_list_outcome = render_standard_track_list(
                                    ui,
                                    app,
                                    "artist_tracks_scroll",
                                    &tracks,
                                    &playlist_targets,
                                    TrackActivation::DoubleClick,
                                    track_row_height,
                                    Some(ui.available_height()),
                                    |_, display| format!("[{}] {}", display.album, display.title),
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