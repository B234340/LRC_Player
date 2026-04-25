use lrc_player_application::infrastructure::persistence::MetaCache;
use crate::presentation::actions::click;
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

/// 行内ボタンの表示エリアの高さを決定する。
///
/// 行の高さとボタンサイズのうち大きい方を採用し、
/// ボタンが十分に収まる高さを保証します。
pub fn button_area_height(row_height: f32, button_size: f32) -> f32 {
    row_height.max(button_size)
}

/// 行内ボタンの表示エリアの幅を決定する。
///
/// ボタンサイズに対して最低幅を保証します。
pub fn button_area_width(button_size: f32, min_width: f32) -> f32 {
    button_size.max(min_width)
}

/// ドラッグハンドルボタンを描画し、そのレスポンスを返します。
///
/// `handle_size` はタブ側で指定できるようにし、`min_width` は必要な最小幅を保証します。
pub fn render_handle_button(
    ui: &mut egui::Ui,
    row_height: f32,
    handle_label: &str,
    handle_size: f32,
    min_width: f32,
) -> egui::Response {
    let handle_width = button_area_width(handle_size, min_width);
    let handle_area_height = button_area_height(row_height, handle_size);
    let handle_pad_y = button_area_vertical_padding(handle_area_height, handle_size);

    let handle_text_size = (handle_size * 0.75).max(10.0);
    ui.allocate_ui_with_layout(
        egui::vec2(handle_width, handle_area_height),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.add_space(handle_pad_y);
            ui.add_sized(
                [handle_size, handle_size],
                egui::Button::new(egui::RichText::new(handle_label).size(handle_text_size)),
            )
        },
    )
    .inner
}

/// お気に入りトグルボタンを描画し、そのレスポンスを返します。
///
/// `button_size` はタブ側で指定できます。`min_width` は最低幅を保証します。
pub fn render_favorite_toggle(
    ui: &mut egui::Ui,
    app: &mut GuiApp,
    track_path: &str,
    row_height: f32,
    button_size: f32,
    min_width: f32,
) -> egui::Response {
    let is_favorite = perform_is_track_favorite(app, track_path);
    let star_label = if is_favorite { "♥" } else { "♡" };
    // Snap to whole pixels to avoid subtle 1px vertical distortion from fractional sizes.
    let button_side = button_size.max(min_width).round();
    let button_area_height = button_area_height(row_height, button_side).round();
    let star_text_size = (button_side * 0.75).max(10.0);

    let (area_rect, _) =
        ui.allocate_exact_size(egui::vec2(button_side, button_area_height), egui::Sense::hover());
    let button_rect = egui::Rect::from_center_size(area_rect.center(), egui::vec2(button_side, button_side));

    let response = ui
        .scope(|ui| {
            // Keep this button strictly square regardless of global spacing defaults.
            ui.spacing_mut().interact_size = egui::vec2(button_side, button_side);
            ui.spacing_mut().button_padding = egui::vec2(0.0, 0.0);
            ui.put(
                button_rect,
                egui::Button::new(egui::RichText::new(star_label).size(star_text_size))
                    .min_size(egui::vec2(button_side, button_side)),
            )
        })
        .inner
        .on_hover_text("Toggle favorite");

    if response.clicked() {
        perform_toggle_favorite_action(app, track_path);
    }

    response
}

/// 行内ボタンの上下パディングを計算する。
///
/// ボタンサイズをエリア高さの中央に配置するための余白を返します。
pub fn button_area_vertical_padding(area_height: f32, button_size: f32) -> f32 {
    ((area_height - button_size) * 0.5).max(0.0)
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
pub fn render_standard_track_list<F, S>(
    ui: &mut egui::Ui,
    app: &mut GuiApp,
    id_source: &str,
    tracks: &[String],
    playlist_targets: &[(usize, String)],
    activation: TrackActivation,
    row_height: f32,
    favorite_button_size: f32,
    max_height: Option<f32>,
    mut label_for: F,
    mut selected_for_row: S,
) -> TrackListOutcome
where
    F: FnMut(usize, &TrackDisplay) -> String,
    S: FnMut(usize) -> bool,
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
                let _ = render_favorite_toggle(ui, app, track, row_height, favorite_button_size, 28.0);
                    ui.add_space(4.0); // star button spacing
                
                // --- トラック情報ラベル ---
                let selected = selected_for_row(index)
                    || click::selected_track_index(ui, id_source) == Some(index);
                let response = ui.selectable_label(selected, label);

                // 行クリックで選択/選択解除を切り替え
                if response.clicked() {
                    if selected {
                        click::clear_selected_track_index(ui, id_source);
                    } else {
                        click::set_selected_track_index(ui, id_source, index);
                    }
                }

                // アクティベーション方法に応じたクリック判定
                click::process_track_activation(
                    &response,
                    activation,
                    index,
                    &displayed_tracks,
                    &mut outcome,
                );

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
/// * `handle_renderer` - 各行の左端に描画するハンドル／ドラッグボタン
/// * `row_prefix` - ハンドルの次に描画する任意のプレフィックス領域
/// * `trailing_actions` - 各行の右端に追加アクションを描画するクロージャ
#[allow(clippy::too_many_arguments)]
pub fn render_track_list_with_artwork<F, S, H, P, R, T, M>(
    ui: &mut egui::Ui,
    app: &mut GuiApp,
    id_source: &str,
    tracks: &[String],
    playlist_targets: &[(usize, String)],
    activation: TrackActivation,
    row_height: f32,
    favorite_button_size: f32,
    max_height: Option<f32>,
    mut label_for: F,
    mut selected_for_row: S,
    artwork_renderer: fn(&mut egui::Ui, &str, &str, egui::Vec2, bool),
    mut handle_renderer: H,
    mut row_prefix: P,
    mut response_handler: R,
    mut trailing_actions: T,
    mut context_menu_extra: M,
) -> TrackListOutcome
where
    F: FnMut(usize, &TrackDisplay) -> String,
    S: FnMut(&GuiApp, usize, &TrackDisplay, &str) -> bool,
    H: FnMut(&mut egui::Ui, &mut GuiApp, usize, &TrackDisplay, &str),
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
            let selected = selected_for_row(app, index, &display, track)
                || click::selected_track_index(ui, id_source) == Some(index);

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
                    ui.add_space(8.0);
                    handle_renderer(ui, app, index, &display, track);
                    // ui.add_space(8.0);
                    row_prefix(ui, app, index, &display, track);
                    ui.add_space(8.0);

                    let _ = render_favorite_toggle(ui, app, track, row_height, favorite_button_size, 28.0);
                    ui.add_space(8.0);

                    let art_size = egui::vec2(row_height, row_height);
                    artwork_renderer(ui, track, &format!("artwork_{}_{}", id_source, index), art_size, is_playing);
                    ui.add_space(8.0);

                    ui.vertical(|ui| {
                        let response = ui
                            .selectable_label(selected, label)
                            .on_hover_text("Double click to play. Right click for actions.");

                        // 行クリックで選択/選択解除を切り替え
                        if response.clicked() {
                            if selected {
                                click::clear_selected_track_index(ui, id_source);
                            } else {
                                click::set_selected_track_index(ui, id_source, index);
                            }
                        }

                        let handled = response_handler(&response, app, index, &display, track, &mut outcome);

                        if !handled {
                            click::process_track_activation(
                                &response,
                                activation,
                                index,
                                &displayed_tracks,
                                &mut outcome,
                            );
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