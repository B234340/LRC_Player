use lrc_player_application::infrastructure::persistence::MetaCache;
use crate::presentation::actions::track::{
    perform_is_track_favorite,
    perform_toggle_favorite_action,
};
use crate::presentation::components::track_context_menu::render_track_context_menu;
use crate::presentation::app::GuiApp;
use eframe::egui;
use std::collections::HashMap;
use std::path::Path;

/// トラックリスト内でのアクティベーション（選択/再生）方法を定義する列挙型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackActivation {
    /// シングルクリックでアクティベート
    SingleClick,
    /// ダブルクリックでアクティベート
    DoubleClick,
}

/// トラック表示用の情報を保持する構造体
#[derive(Debug, Clone)]
pub struct TrackDisplay {
    /// トラックのファイルパス
    pub path: String,
    /// 表示用タイトル
    pub title: String,
    /// 表示用アーティスト名
    pub artist: String,
    /// 表示用アルバム名
    pub album: String,
}

/// トラックリスト操作の結果を保持する構造体
#[derive(Debug, Clone, Default)]
pub struct TrackListOutcome {
    /// アクティベートされたトラックのインデックス
    pub play_index: Option<usize>,
    /// 現在表示中の全トラックリスト（再生用）
    pub play_list: Vec<String>,
    /// 「今すぐ再生」が選択されたトラックのパス
    pub play_now_path: Option<String>,
    /// 「次に再生」が選択されたトラックのパス
    pub next_path: Option<String>,
    /// 「キューに追加」が選択されたトラックのパス
    pub enqueue_path: Option<String>,
    /// プレイリストへの追加操作（プレイリストインデックス, トラックパス）
    pub add_to_playlist: Option<(usize, String)>,
    /// 一括削除対象のトラックインデックス
    pub remove_indices: Option<Vec<usize>>,
    /// 行を上に移動する操作
    pub move_up_index: Option<usize>,
    /// 行を下に移動する操作
    pub move_down_index: Option<usize>,
    /// 「詳細を表示」が選択されたトラックのパス
    pub show_details_path: Option<String>,
}

/// トラックの表示情報（タイトル、アーティスト、アルバム）を解決する内部関数。
/// 
/// 以下の順序で情報を取得する：
/// 1. インメモリキャッシュ (`metadata_cache`)
/// 2. 永続化メタデータキャッシュ (`meta_cache`)
/// 3. ファイル名からのフォールバック値
/// 
/// # 引数
/// * `track_path` - トラックファイルのパス
/// * `meta_cache` - 永続化されたメタデータキャッシュ（オプション）
/// * `metadata_cache` - インメモリのメタデータキャッシュ
/// 
/// # 戻り値
/// 解決されたトラック表示情報
fn resolve_track_display(
    track_path: &str,
    meta_cache: Option<&MetaCache>,
    metadata_cache: &mut HashMap<String, (String, String, String)>,
) -> TrackDisplay {
    // ファイル名をフォールバック用タイトルとして取得
    let fallback_title = Path::new(track_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(track_path)
        .to_string();

    // メタデータキャッシュから情報を取得（キャッシュミス時は永続化キャッシュから読み込み）
    let (title, artist, album) = metadata_cache
        .entry(track_path.to_string())
        .or_insert_with(|| {
            meta_cache
                .and_then(|cache| cache.track_metadata_by_location(track_path).ok().flatten())
                .map(|(title, artist, album)| {
                    // 空文字列の場合はフォールバック値を使用
                    let title = if title.trim().is_empty() {
                        fallback_title.clone()
                    } else {
                        title
                    };
                    let artist = if artist.trim().is_empty() {
                        "Unknown artist".to_string()
                    } else {
                        artist
                    };
                    let album = if album.trim().is_empty() {
                        "Unknown album".to_string()
                    } else {
                        album
                    };
                    (title, artist, album)
                })
                .unwrap_or_else(|| {
                    // メタデータが全く存在しない場合のデフォルト値
                    (
                        fallback_title.clone(),
                        "Unknown artist".to_string(),
                        "Unknown album".to_string(),
                    )
                })
        })
        .clone(); // HashMap からの借用問題を回避するためクローン

    TrackDisplay {
        path: track_path.to_string(),
        title,
        artist,
        album,
    }
}

/// 標準的なトラックリストを描画する。
/// 
/// 各トラック行には以下の要素が含まれる：
/// - お気に入り切り替え用の星ボタン（★/☆）
/// - トラック情報ラベル
/// - 右クリックによるコンテキストメニュー
/// 
/// # 引数
/// * `ui` - egui の UI ビルダー
/// * `app` - アプリケーション状態
/// * `id_source` - スクロールエリアの識別子
/// * `tracks` - 表示するトラックパスのリスト
/// * `playlist_targets` - 追加先プレイリストの候補リスト（インデックス, 名前）
/// * `activation` - トラックのアクティベーション方法（シングル/ダブルクリック）
/// * `row_height` - 各行の高さ
/// * `max_height` - リスト全体の最大高さ（スクロール領域の制限用）
/// * `label_for` - 各トラックの表示ラベルを生成するクロージャ
/// 
/// # 戻り値
/// ユーザー操作の結果を表す `TrackListOutcome`
#[allow(clippy::too_many_arguments)]
pub fn render_standard_track_list<F>(
    ui: &mut egui::Ui,
    app: &mut GuiApp,
    id_source: &str,
    tracks: &[String],
    playlist_targets: &[(usize, String)],
    activation: TrackActivation,
    row_height: f32,
    max_height: Option<f32>,
    mut label_for: F,
) -> TrackListOutcome
where
    F: FnMut(usize, &TrackDisplay) -> String,
{
    // 戻り値となる結果オブジェクトを初期化
    let mut outcome = TrackListOutcome::default();
    // メタデータの一時キャッシュ（表示中のみ有効）
    let mut metadata_cache: HashMap<String, (String, String, String)> = HashMap::new();
    // 永続化メタデータキャッシュの読み込み試行
    let meta_cache = MetaCache::open_default().ok();
    // 再生リスト用にトラックリストをクローン
    let displayed_tracks = tracks.to_vec();

    // スクロールエリア内に表示する行を生成するクロージャ
    let mut render_rows = |ui: &mut egui::Ui| {
        for (index, track) in tracks.iter().enumerate() {
            // トラックの表示情報を解決
            let display = resolve_track_display(track, meta_cache.as_ref(), &mut metadata_cache);
            // カスタムラベルを生成
            let label = label_for(index, &display);

            // 行全体を水平レイアウトで配置
            ui.horizontal(|ui| {
                // --- お気に入り星ボタン ---
                let is_favorite = perform_is_track_favorite(app, track);
                let star_label = if is_favorite { "★" } else { "☆" };
                
                // 星ボタンエリアの高さを計算（行の高さに合わせる、最小22px）
                let star_area_height = row_height.max(22.0);
                let vertical_pad = ((star_area_height - 22.0) * 0.5).max(0.0);
                
                // 星ボタンを中央配置するための専用UI領域
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
                    .on_hover_text("Toggle favorite"); // ホバー時のツールチップ

                // 星ボタンがクリックされたらお気に入り状態を切り替え
                if toggle_response.clicked() {
                    perform_toggle_favorite_action(app, track);
                }

                ui.add_space(4.0); // 星ボタンとラベルの間隔
                
                // --- トラック情報ラベル ---
                let response = ui.selectable_label(false, label);

                // アクティベーション方法に応じたクリック判定
                let activated = match activation {
                    TrackActivation::SingleClick => response.clicked(),
                    TrackActivation::DoubleClick => response.double_clicked(),
                };
                
                // アクティベートされた場合、再生開始情報を結果に設定
                if activated {
                    outcome.play_index = Some(index);
                    outcome.play_list = displayed_tracks.clone();
                }

                // --- 右クリックによるコンテキストメニュー ---
                response.context_menu(|ui| {
                    let menu_outcome = render_track_context_menu(
                        ui,
                        track,
                        playlist_targets,
                        "Add to queue",  // キュー追加ボタンのラベル
                        |_, _| {},        // 追加メニュー項目なし
                    );
                    
                    // メニュー操作の結果を収集
                    if let Some(path) = menu_outcome.play_now_path {
                        outcome.play_now_path = Some(path);
                    }
                    if let Some(path) = menu_outcome.next_path {
                        outcome.next_path = Some(path);
                    }
                    if let Some(path) = menu_outcome.enqueue_path {
                        outcome.enqueue_path = Some(path);
                    }
                    if let Some(item) = menu_outcome.add_to_playlist {
                        outcome.add_to_playlist = Some(item);
                    }
                    if let Some(path) = menu_outcome.show_details_path {
                        outcome.show_details_path = Some(path);
                    }
                });
            });
        }
    };

    // --- スクロールエリアの構築と描画 ---
    let mut area = egui::ScrollArea::vertical()
        .id_source(id_source)          // スクロール位置記憶用の一意識別子
        .auto_shrink([false, false]);  // コンテンツに合わせて縮小しない
    
    // 最大高さが指定されていれば適用
    if let Some(height) = max_height {
        area = area.max_height(height);
    }
    
    // スクロールエリア内に行を描画
    area.show(ui, |ui| {
        render_rows(ui);
    });

    outcome // ユーザー操作の結果を返却
}

/// アートワーク付きのトラックリストを描画する。
///
/// この関数は `render_standard_track_list` と同様の機能を提供しつつ、
/// 各行にアートワークと汎用の右端アクションを追加できます。
///
/// # 引数
/// * `ui` - egui の UI ビルダー
/// * `app` - アプリケーション状態
/// * `id_source` - スクロールエリアの識別子
/// * `tracks` - 表示するトラックパスのリスト
/// * `playlist_targets` - 追加先プレイリストの候補リスト
/// * `activation` - クリック動作によるアクティベーション方法
/// * `row_height` - 各行の高さ
/// * `max_height` - リスト全体の最大高さ
/// * `label_for` - 各トラック行のラベルを生成するクロージャ
/// * `artwork_renderer` - アートワークを描画する関数
/// * `trailing_actions` - 各行の右端に追加アクションを描画するクロージャ
#[allow(clippy::too_many_arguments)]
pub fn render_track_list_with_artwork<F, S, P, R, T, M>(
    ui: &mut egui::Ui,
    app: &mut GuiApp,
    id_source: &str,
    tracks: &[String],
    playlist_targets: &[(usize, String)],
    activation: TrackActivation,
    row_height: f32,
    max_height: Option<f32>,
    mut label_for: F,
    mut selected_for_row: S,
    artwork_renderer: fn(&mut egui::Ui, &str, &str, egui::Vec2, bool),
    mut row_prefix: P,
    mut response_handler: R,
    mut trailing_actions: T,
    mut context_menu_extra: M,
) -> TrackListOutcome
where
    F: FnMut(usize, &TrackDisplay) -> String,
    S: FnMut(&GuiApp, usize, &TrackDisplay, &str) -> bool,
    P: FnMut(&mut egui::Ui, &mut GuiApp, usize, &TrackDisplay, &str),
    R: FnMut(&egui::Response, &mut GuiApp, usize, &TrackDisplay, &str, &mut TrackListOutcome) -> bool,
    T: FnMut(&mut egui::Ui, &mut GuiApp, usize, &TrackDisplay, &str, &mut TrackListOutcome),
    M: FnMut(&mut egui::Ui, &mut GuiApp, usize, &TrackDisplay, &str, &mut TrackListOutcome),
{
    let mut outcome = TrackListOutcome::default();
    let mut metadata_cache: HashMap<String, (String, String, String)> = HashMap::new();
    let meta_cache = MetaCache::open_default().ok();
    let displayed_tracks = tracks.to_vec();

    let mut render_rows = |ui: &mut egui::Ui| {
        for (index, track) in tracks.iter().enumerate() {
            let display = resolve_track_display(track, meta_cache.as_ref(), &mut metadata_cache);
            let label = label_for(index, &display);
            let is_playing = app.play_path == *track;
            let selected = selected_for_row(app, index, &display, track);

            let mut frame = egui::Frame::group(ui.style());
            if is_playing {
                frame = frame
                    .fill(ui.visuals().selection.bg_fill)
                    .stroke(egui::Stroke::new(2.0, ui.visuals().selection.stroke.color));
            } else if selected {
                frame = frame.stroke(egui::Stroke::new(1.0, ui.visuals().widgets.active.bg_stroke.color));
            }

            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    row_prefix(ui, app, index, &display, track);
                    ui.add_space(8.0);

                    let art_size = egui::vec2(row_height, row_height);
                    let star_area_height = art_size.y;
                    let vertical_pad = ((star_area_height - 22.0) * 0.5).max(0.0);

                    let is_favorite = perform_is_track_favorite(app, track);
                    let star_label = if is_favorite { "★" } else { "☆" };
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
                        perform_toggle_favorite_action(app, track);
                    }

                    ui.add_space(8.0);

                    artwork_renderer(ui, track, &format!("artwork_{}_{}", id_source, index), art_size, is_playing);
                    ui.add_space(8.0);

                    ui.vertical(|ui| {
                        let response = ui
                            .selectable_label(selected, label)
                            .on_hover_text("Double click to play. Right click for actions.");

                        let handled = response_handler(&response, app, index, &display, track, &mut outcome);

                        if !handled {
                            let activated = match activation {
                                TrackActivation::SingleClick => response.clicked(),
                                TrackActivation::DoubleClick => response.double_clicked(),
                            };

                            if activated {
                                outcome.play_index = Some(index);
                                outcome.play_list = displayed_tracks.clone();
                            }
                        }

                        response.context_menu(|ui| {
                            let menu_outcome = render_track_context_menu(
                                ui,
                                track,
                                playlist_targets,
                                "Add to queue",
                                |_, _| {},
                            );

                            if let Some(path) = menu_outcome.play_now_path {
                                outcome.play_now_path = Some(path);
                            }
                            if let Some(path) = menu_outcome.next_path {
                                outcome.next_path = Some(path);
                            }
                            if let Some(path) = menu_outcome.enqueue_path {
                                outcome.enqueue_path = Some(path);
                            }
                            if let Some(item) = menu_outcome.add_to_playlist {
                                outcome.add_to_playlist = Some(item);
                            }
                            if let Some(path) = menu_outcome.show_details_path {
                                outcome.show_details_path = Some(path);
                            }

                            context_menu_extra(ui, app, index, &display, track, &mut outcome);
                        });
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        trailing_actions(ui, app, index, &display, track, &mut outcome);
                    });
                });
            });

            ui.add_space(4.0);
        }
    };

    let mut area = egui::ScrollArea::vertical()
        .id_source(id_source)
        .auto_shrink([false, false]);
    if let Some(height) = max_height {
        area = area.max_height(height);
    }
    area.show(ui, |ui| {
        render_rows(ui);
    });

    outcome
}