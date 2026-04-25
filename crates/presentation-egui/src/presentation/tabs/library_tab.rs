use lrc_player_application::view_model::LibraryDirectoryVm;
use crate::presentation::actions::play::perform_play_track_now;
use crate::presentation::actions::queue::{
    perform_add_track_next_in_queue,
    perform_enqueue_track_path,
    perform_play_from_displayed_list,
};
use crate::presentation::actions::track::{
    perform_add_track_to_playlist,
    perform_open_track_details,
};
use crate::presentation::components::track_list::{render_standard_track_list, TrackActivation};
use crate::presentation::app::GuiApp;
use eframe::egui;

/// ライブラリ画面のUIを描画する関数
/// 
/// この画面には以下の要素が含まれる：
/// - ライブラリ統計情報（アルバム数、トラック数）
/// - お気に入りディレクトリの管理
/// - 最近使用したディレクトリ一覧
/// - スキャン除外パス一覧
/// - ディレクトリツリー（階層表示）
/// - 選択ディレクトリ内のトラック一覧
/// - 最近再生したトラック一覧
pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    // ========== 遅延実行するアクションを保持する変数群 ==========
    // UI描画中に直接状態を変更するとeguiの制約に違反するため、
    // アクションを一時変数に保存し、描画完了後に実行する
    
    // ディレクトリ選択アクション（選択するディレクトリのパス）
    let mut pending_select_directory: Option<String> = None;
    let mut pending_play_index: Option<usize> = None;           // 再生開始インデックス
    let mut pending_play_list: Vec<String> = Vec::new();        // 再生リスト
    let mut pending_play_now_path: Option<String> = None;       // 即時再生するトラックパス
    let mut pending_next_path: Option<String> = None;           // 次に再生するトラックパス
    let mut pending_enqueue_path: Option<String> = None;        // キューに追加するトラックパス
    let mut pending_add_to_playlist: Option<(usize, String)> = None;  // プレイリストに追加するトラック
    let mut pending_show_details_path: Option<String> = None;   // 詳細表示するトラックパス
    
    // ========== プレイリスト一覧の準備 ==========
    // コンテキストメニュー用に (インデックス, プレイリスト名) のペアを作成
    let playlist_targets: Vec<(usize, String)> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(idx, p)| (idx, p.name.clone()))
        .collect();

    // ライブラリ情報を取得（ViewModelから）
    let library = app.app_vm.library.clone();
    let selected_directory = library.selected_directory.as_deref();

    // ========== ヘッダーと統計情報 ==========
    ui.heading("Library");
    ui.label("Browse scanned library metadata and recent track locations.");
    ui.separator();
    ui.label(format!("Albums: {}", library.album_count));  // ライブラリ内のアルバム総数
    ui.label(format!("Tracks: {}", library.track_count));  // ライブラリ内のトラック総数

    // ========== お気に入りディレクトリ一覧 ==========
    ui.separator();
    ui.heading("Favorites");
    ui.horizontal(|ui| {
        // お気に入り追加用のテキスト入力フィールド
        ui.text_edit_singleline(&mut app.favorite_directory_input);
        if ui.button("Add favorite").clicked() {
            app.add_favorite_directory();  // 入力されたパスをお気に入りに追加
        }
    });
    
    if app.favorite_directories.is_empty() {
        ui.label("No favorite directories configured.");
    } else {
        let mut remove_index = None;  // 削除するお気に入りのインデックス
        for (index, favorite) in app.favorite_directories.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("• {}", favorite));  // お気に入りパスの表示
                if ui.button("Remove").clicked() {
                    remove_index = Some(index);  // 削除アクションを記録
                }
            });
        }
        // 削除アクションの実行（UI描画後に処理するため即時実行可能）
        if let Some(index) = remove_index {
            app.remove_favorite_directory(index);
        }
    }

    // ========== 最近使用したディレクトリ一覧 ==========
    ui.separator();
    ui.heading("Recent directories");
    if library.recent_directories.is_empty() {
        ui.label("No recent directories available.");
    } else {
        ui.group(|ui| {
            // 最大20件まで表示（画面の見やすさを考慮）
            for dir in library.recent_directories.iter().take(20) {
                ui.label(dir);
            }
        });
    }

    // ========== 除外パス一覧（スキャン対象外のパス）==========
    ui.separator();
    ui.heading("Excluded paths");
    if app.scan_excludes.is_empty() {
        ui.label("No excluded paths configured.");
    } else {
        ui.group(|ui| {
            for exclude in app.scan_excludes.iter() {
                ui.label(exclude);  // 除外パスの表示
            }
        });
    }

    // ========== ディレクトリツリー表示 ==========
    ui.separator();
    ui.heading("Directory tree");
    if library.directory_tree.is_empty() {
        ui.label("No library directories available.");
    } else {
        // ルートディレクトリから再帰的にツリーを描画
        for root in library.directory_tree.iter() {
            render_directory_node(
                ui,
                app,
                root,
                0,  // ルートノードはインデントレベル0から開始
                selected_directory,
                &mut pending_select_directory,
            );
        }
    }

    // ========== 選択されたディレクトリのトラック一覧 ==========
    if let Some(selected) = selected_directory {
        ui.separator();
        ui.heading("Selected directory");
        ui.label(selected);
        
        if library.selected_directory_tracks.is_empty() {
            ui.label("No tracks found for this directory.");
        } else {
            ui.group(|ui| {
                ui.label("Double click: create queue from this list and play. Right click: common actions.");

                let track_paths: Vec<String> = library
                    .selected_directory_tracks
                    .iter()
                    .map(|row| row.id.clone())
                    .collect();

                let track_list_outcome = render_standard_track_list(
                    ui,
                    app,
                    "library_directory_tracks",
                    &track_paths,
                    &playlist_targets,
                    TrackActivation::DoubleClick,
                    24.0,
                    22.0,
                    Some(ui.available_height()),
                    |_index, display| display.title.clone(),
                    |_index| false,
                );

                pending_play_index = track_list_outcome.play_index;
                pending_play_list = track_list_outcome.play_list;
                pending_play_now_path = track_list_outcome.play_now_path;
                pending_next_path = track_list_outcome.next_path;
                pending_enqueue_path = track_list_outcome.enqueue_path;
                pending_add_to_playlist = track_list_outcome.add_to_playlist;
                pending_show_details_path = track_list_outcome.show_details_path;
            });
        }
    }

    // ========== 最近再生したトラック一覧 ==========
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

    // ========== トラック詳細ダイアログ ==========
    // 詳細表示が要求されている場合、ダイアログウィンドウを表示
    if let Some(details) = app.track_details_dialog.clone() {
        let mut open = true;  // ウィンドウの表示状態（閉じられたらfalseになる）
        egui::Window::new("Track details")
            .open(&mut open)
            .resizable(true)  // リサイズ可能
            .show(ui.ctx(), |ui| {
                // トラックの詳細情報を表示
                ui.label(format!("Title: {}", details.title));
                ui.label(format!("Artist: {}", details.artist));
                ui.label(format!("Album: {}", details.album));
                ui.separator();
                ui.label(format!("File path: {}", details.path));
            });
        
        // ウィンドウが閉じられたらダイアログ状態をクリア
        if !open {
            app.track_details_dialog = None;
        }
    }

    // ========== 保留中のアクションを実行 ==========
    // UI描画完了後に実際の状態変更を行う
    
    if let Some(path) = pending_select_directory {
        app.select_directory(Some(path));  // ディレクトリを選択
    }
    if let Some(index) = pending_play_index {
        perform_play_from_displayed_list(&mut *app, &pending_play_list, index);  // リストから再生開始
    }
    if let Some(path) = pending_play_now_path {
        perform_play_track_now(&mut *app, &path);  // 即時再生
    }
    if let Some(path) = pending_next_path {
        perform_add_track_next_in_queue(&mut *app, &path);  // 次に再生するトラックとしてキューに追加
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
}

/// ディレクトリツリーのノードを再帰的に描画する関数
/// 
/// ディレクトリ階層をツリー形式で表示する。
/// 子ディレクトリがある場合は展開/折りたたみボタンを表示し、
/// 展開時は子ノードを再帰的に描画する。
/// 
/// # 引数
/// * `ui` - eguiのUIビルダー
/// * `app` - アプリケーション状態
/// * `node` - 現在描画するディレクトリノード
/// * `indent` - インデントレベル（階層の深さ、0から開始）
/// * `selected_directory` - 現在選択中のディレクトリパス（ハイライト表示用）
/// * `pending_select_directory` - 選択が保留されているディレクトリパス（出力用）
fn render_directory_node(
    ui: &mut egui::Ui,
    app: &mut GuiApp,
    node: &LibraryDirectoryVm,
    indent: usize,
    selected_directory: Option<&str>,
    pending_select_directory: &mut Option<String>,
) {
    let has_children = !node.children.is_empty();  // 子ディレクトリの有無
    let expanded = app.is_library_directory_expanded(&node.path);  // 現在の展開状態

    ui.horizontal(|ui| {
        // 階層の深さに応じたインデントスペースを追加
        for _ in 0..indent {
            ui.add_space(12.0);
        }

        // 展開/折りたたみボタン（子ノードがある場合のみ表示）
        if has_children {
            // 展開状態に応じたアイコン：展開時は「▼」、折りたたみ時は「▶」
            let icon = if expanded { "▼" } else { "▶" };
            if ui.button(icon).clicked() {
                app.toggle_library_directory_expanded(&node.path);  // 展開状態を切り替え
            }
        } else {
            // 子ノードがない場合はアイコン分のスペースを確保（レイアウトの一貫性維持）
            ui.add_space(18.0);
        }

        // ディレクトリ選択ラベル
        // 現在選択中のディレクトリと一致する場合はハイライト表示
        if ui
            .selectable_label(
                selected_directory.map(|s| s == node.path).unwrap_or(false),
                &node.path,
            )
            .clicked()
        {
            // クリックされたら選択アクションを保留（UI描画後に実行）
            *pending_select_directory = Some(node.path.clone());
        }
    });

    // 展開されている場合、子ノードを再帰的に描画
    if expanded {
        for child in node.children.iter() {
            render_directory_node(
                ui,
                app,
                child,
                indent + 1,  // 子ノードはインデントレベルを1増やす
                selected_directory,
                pending_select_directory,
            );
        }
    }
}