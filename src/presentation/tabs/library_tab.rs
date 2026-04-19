use crate::application::view_model::LibraryDirectoryVm;
use crate::presentation::egui_app::GuiApp;
use eframe::egui;

/// ライブラリ画面のUIを描画する関数
/// 
/// ライブラリの概要、ディレクトリツリー、選択されたディレクトリのトラック一覧を表示する
pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    // 遅延実行するアクションを保持する変数群
    let mut pending_select_directory: Option<String> = None;  // ディレクトリ選択アクション
    let mut pending_play_index: Option<usize> = None;  // 再生開始インデックス
    let mut pending_play_list: Vec<String> = Vec::new();  // 再生リスト
    let mut pending_play_now_path: Option<String> = None;  // 即時再生するトラックパス
    let mut pending_next_path: Option<String> = None;  // 次に再生するトラックパス
    let mut pending_enqueue_path: Option<String> = None;  // キューに追加するトラックパス
    let mut pending_add_to_playlist: Option<(usize, String)> = None;  // プレイリストに追加するトラック
    let mut pending_show_details_path: Option<String> = None;  // 詳細表示するトラックパス
    
    // プレイリストの一覧を作成（インデックスと名前のペア）
    let playlist_targets: Vec<(usize, String)> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(idx, p)| (idx, p.name.clone()))
        .collect();

    // ライブラリ情報を取得
    let library = app.app_vm.library.clone();
    let selected_directory = library.selected_directory.as_deref();

    // --- ヘッダーと統計情報 ---
    ui.heading("Library");
    ui.label("Browse scanned library metadata and recent track locations.");
    ui.separator();
    ui.label(format!("Albums: {}", library.album_count));  // アルバム数
    ui.label(format!("Tracks: {}", library.track_count));  // トラック数

    // --- お気に入りディレクトリ一覧 ---
    ui.separator();
    ui.heading("Favorites");
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.favorite_directory_input);
        if ui.button("Add favorite").clicked() {
            app.add_favorite_directory();
        }
    });
    if app.favorite_directories.is_empty() {
        ui.label("No favorite directories configured.");
    } else {
        let mut remove_index = None;
        for (index, favorite) in app.favorite_directories.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("• {}", favorite));
                if ui.button("Remove").clicked() {
                    remove_index = Some(index);
                }
            });
        }
        if let Some(index) = remove_index {
            app.remove_favorite_directory(index);
        }
    }

    // --- 最近使用したディレクトリ一覧 ---
    ui.separator();
    ui.heading("Recent directories");
    if library.recent_directories.is_empty() {
        ui.label("No recent directories available.");
    } else {
        ui.group(|ui| {
            // 最大20件まで表示
            for dir in library.recent_directories.iter().take(20) {
                ui.label(dir);
            }
        });
    }

    // --- 除外パス一覧（スキャン対象外のパス）---
    ui.separator();
    ui.heading("Excluded paths");
    if app.scan_excludes.is_empty() {
        ui.label("No excluded paths configured.");
    } else {
        ui.group(|ui| {
            for exclude in app.scan_excludes.iter() {
                ui.label(exclude);
            }
        });
    }

    // --- ディレクトリツリー表示 ---
    ui.separator();
    ui.heading("Directory tree");
    if library.directory_tree.is_empty() {
        ui.label("No library directories available.");
    } else {
        // ルートディレクトリから再帰的に表示
        for root in library.directory_tree.iter() {
            render_directory_node(
                ui,
                app,
                root,
                0,  // インデントレベル0から開始
                selected_directory,
                &mut pending_select_directory,
            );
        }
    }

    // --- 選択されたディレクトリのトラック一覧 ---
    if let Some(selected) = selected_directory {
        ui.separator();
        ui.heading("Selected directory");
        ui.label(selected);
        
        if library.selected_directory_tracks.is_empty() {
            ui.label("No tracks found for this directory.");
        } else {
            ui.group(|ui| {
                ui.label("Left click: create queue from this list and play. Right click: common actions.");
                
                // トラック一覧を表示
                for (index, track) in library.selected_directory_tracks.iter().enumerate() {
                    ui.horizontal(|ui| {
                        // お気に入り切り替えボタン（★/☆）
                        ui.vertical_centered(|ui| {
                            let is_favorite = app.is_track_favorite(&track.title);
                            let star_label = if is_favorite { "★" } else { "☆" };
                            if ui
                                .add_sized([22.0, 22.0], egui::Button::new(star_label))
                                .on_hover_text("Toggle favorite")
                                .clicked()
                            {
                                app.toggle_favorite_track(&track.title);
                            }
                        });
                        
                        ui.add_space(4.0);
                        
                        // トラック選択ラベル
                        ui.vertical(|ui| {
                            let response = ui.selectable_label(false, &track.title);
                            
                            // クリックで再生開始（リスト全体を再生キューに）
                            if response.clicked() {
                                pending_play_index = Some(index);
                                pending_play_list = library
                                    .selected_directory_tracks
                                    .iter()
                                    .map(|row| row.title.clone())
                                    .collect();
                            }
                            
                            // 右クリックコンテキストメニュー
                            response.context_menu(|ui| {
                                if ui.button("Play now").clicked() {
                                    pending_play_now_path = Some(track.title.clone());
                                    ui.close_menu();
                                }
                                if ui.button("Play next").clicked() {
                                    pending_next_path = Some(track.title.clone());
                                    ui.close_menu();
                                }
                                if ui.button("Add to queue").clicked() {
                                    pending_enqueue_path = Some(track.title.clone());
                                    ui.close_menu();
                                }
                                ui.separator();
                                ui.label("Add to playlist");
                                // 各プレイリストへの追加メニュー
                                for (target_index, target_name) in &playlist_targets {
                                    if ui.button(target_name).clicked() {
                                        pending_add_to_playlist = Some((*target_index, track.title.clone()));
                                        ui.close_menu();
                                    }
                                }
                                ui.separator();
                                if ui.button("Show details").clicked() {
                                    pending_show_details_path = Some(track.title.clone());
                                    ui.close_menu();
                                }
                            });
                        });
                    });
                }
            });
        }
    }

    // --- 最近再生したトラック一覧 ---
    ui.separator();
    ui.heading("Recent tracks");
    if library.recent_locations.is_empty() {
        ui.label("No recent track locations available.");
    } else {
        ui.group(|ui| {
            // 最大20件まで表示
            for path in library.recent_locations.iter().take(20) {
                ui.label(path);
            }
        });
    }

    if let Some(details) = app.track_details_dialog.clone() {
        let mut open = true;
        egui::Window::new("Track details")
            .open(&mut open)
            .resizable(true)
            .show(ui.ctx(), |ui| {
                ui.label(format!("Title: {}", details.title));
                ui.label(format!("Artist: {}", details.artist));
                ui.label(format!("Album: {}", details.album));
                ui.separator();
                ui.label(format!("File path: {}", details.path));
            });
        if !open {
            app.track_details_dialog = None;
        }
    }

    // 保留中のアクションを実行
    if let Some(path) = pending_select_directory {
        app.select_directory(Some(path));  // ディレクトリ選択
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
}

/// ディレクトリツリーのノードを再帰的に描画する関数
/// 
/// # 引数
/// * `ui` - eguiのUIビルダー
/// * `app` - アプリケーション状態
/// * `node` - 現在描画するディレクトリノード
/// * `indent` - インデントレベル（階層の深さ）
/// * `selected_directory` - 現在選択中のディレクトリパス
/// * `pending_select_directory` - 選択が保留されているディレクトリパス（出力用）
fn render_directory_node(
    ui: &mut egui::Ui,
    app: &mut GuiApp,
    node: &LibraryDirectoryVm,
    indent: usize,
    selected_directory: Option<&str>,
    pending_select_directory: &mut Option<String>,
) {
    let has_children = !node.children.is_empty();  // 子ノードの有無
    let expanded = app.is_library_directory_expanded(&node.path);  // 展開状態

    ui.horizontal(|ui| {
        // インデントスペースの追加（階層の深さに応じて）
        for _ in 0..indent {
            ui.add_space(12.0);
        }

        // 展開/折りたたみボタン（子ノードがある場合のみ表示）
        if has_children {
            let icon = if expanded { "▼" } else { "▶" };  // 展開状態に応じたアイコン
            if ui.button(icon).clicked() {
                app.toggle_library_directory_expanded(&node.path);  // 展開状態を切り替え
            }
        } else {
            ui.add_space(18.0);  // アイコンがない場合のスペース確保
        }

        // ディレクトリ選択ラベル
        if ui
            .selectable_label(
                selected_directory.map(|s| s == node.path).unwrap_or(false),  // 選択状態
                &node.path,
            )
            .clicked()
        {
            *pending_select_directory = Some(node.path.clone());  // 選択アクションを保留
        }
    });

    // 展開されている場合、子ノードを再帰的に描画
    if expanded {
        for child in node.children.iter() {
            render_directory_node(
                ui,
                app,
                child,
                indent + 1,  // インデントレベルを1増やす
                selected_directory,
                pending_select_directory,
            );
        }
    }
}