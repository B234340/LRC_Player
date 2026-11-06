//! Track lists (virtualized).
//!
//! Rows are built only for the visible range (`ScrollArea::show_rows`); display metadata
//! comes from the app's [`TrackMetaCache`](crate::presentation::app::TrackMetaCache) (one
//! batched query for rows not seen before, then hash lookups) and favorites from a hash set.
//! Nothing here opens the database or touches the file system per frame.

use crate::presentation::actions::click;
use crate::presentation::actions::track::{
    perform_is_track_favorite,
    perform_toggle_favorite_action,
};
use crate::presentation::components::track_context_menu::render_track_context_menu;
use crate::presentation::components::track_display::render_track_artwork;
use crate::presentation::app::{ArtworkMode, GuiApp};
use eframe::egui;
use std::ops::Range;
use std::time::Duration;

/// Resolves display metadata for the visible rows (one batched query for unseen rows).
fn ensure_row_metadata(ui: &egui::Ui, app: &mut GuiApp, tracks: &[String], range: Range<usize>) {
    let end = range.end.min(tracks.len());
    let start = range.start.min(end);
    if !app.track_meta.ensure(app.meta.as_ref(), &tracks[start..end]) {
        // Shared connection busy (rare): show file names now, retry shortly.
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }
}

/// Applies a pending "scroll to row" request for this list.
fn take_scroll_offset(
    app: &mut GuiApp,
    id_source: &str,
    row_stride: f32,
    viewport_height: f32,
) -> Option<f32> {
    match &app.track_list_scroll_request {
        Some((id, index)) if id == id_source => {
            let index = *index;
            app.track_list_scroll_request = None;
            Some((index as f32 * row_stride - viewport_height * 0.5 + row_stride * 0.5).max(0.0))
        }
        _ => None,
    }
}

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
    let mut outcome = TrackListOutcome::default();
    let favorite_side = favorite_button_size.max(28.0).round();
    let row_h = button_area_height(row_height, favorite_side)
        .max(ui.text_style_height(&egui::TextStyle::Button) + ui.spacing().button_padding.y * 2.0)
        .round();
    let stride = row_h + ui.spacing().item_spacing.y;

    let mut area = egui::ScrollArea::vertical()
        .id_salt(id_source)
        .auto_shrink([false, false]);
    if let Some(height) = max_height {
        area = area.max_height(height);
    }
    let viewport = max_height.unwrap_or_else(|| ui.available_height());
    if let Some(offset) = take_scroll_offset(app, id_source, stride, viewport) {
        area = area.vertical_scroll_offset(offset);
    }

    area.show_rows(ui, row_h, tracks.len(), |ui, range| {
        ensure_row_metadata(ui, app, tracks, range.clone());
        for index in range {
            let track = &tracks[index];
            let display = app.track_meta.display(track);
            let label = label_for(index, &display);
            let width = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::vec2(width, row_h),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_height(row_h);
                    let _ = render_favorite_toggle(ui, app, track, row_height, favorite_button_size, 28.0);
                    ui.add_space(4.0);

                    let selected = selected_for_row(index)
                        || click::selected_track_index(ui, id_source) == Some(index);
                    let is_playing = app.play_path == *track;
                    let text = if is_playing {
                        egui::RichText::new(label).strong()
                    } else {
                        egui::RichText::new(label)
                    };
                    let response = ui.selectable_label(selected || is_playing, text);

                    if response.clicked() {
                        if selected {
                            click::clear_selected_track_index(ui, id_source);
                        } else {
                            click::set_selected_track_index(ui, id_source, index);
                        }
                    }

                    click::process_track_activation(&response, activation, index, tracks, &mut outcome);

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
                    });
                },
            );
        }
    });

    outcome
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
    artwork_mode: ArtworkMode,
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
    let spacing_y = ui.spacing().item_spacing.y;
    // Rows have a uniform height; it is measured once (frame margins depend on the style)
    // and remembered so `show_rows` can skip off-screen rows.
    let stride_id = egui::Id::new(("track_list_row_stride", id_source));
    let estimated_stride = row_height + 14.0 + 4.0 + spacing_y * 2.0;
    let stride = ui
        .ctx()
        .data(|d| d.get_temp::<f32>(stride_id))
        .unwrap_or(estimated_stride)
        .max(1.0);
    let mut measured_stride: Option<f32> = None;

    let mut area = egui::ScrollArea::vertical()
        .id_salt(id_source)
        .auto_shrink([false, false]);
    if let Some(height) = max_height {
        area = area.max_height(height);
    }
    let viewport = max_height.unwrap_or_else(|| ui.available_height());
    if let Some(offset) = take_scroll_offset(app, id_source, stride, viewport) {
        area = area.vertical_scroll_offset(offset);
    }

    area.show_rows(ui, (stride - spacing_y).max(1.0), tracks.len(), |ui, range| {
        ensure_row_metadata(ui, app, tracks, range.clone());
        let mut previous_top: Option<f32> = None;
        for index in range {
            let track = &tracks[index];
            let row_top = ui.cursor().top();
            if let Some(prev) = previous_top {
                measured_stride.get_or_insert(row_top - prev);
            }
            previous_top = Some(row_top);
            let display_arc = app.track_meta.display(track);
            let display: &TrackDisplay = &display_arc;

            let label = label_for(index, display);
            let is_playing = app.play_path == *track;
            let selected = selected_for_row(app, index, display, track)
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
                    ui.set_min_height(row_height);
                    ui.add_space(8.0);
                    handle_renderer(ui, app, index, display, track);
                    // ui.add_space(8.0);
                    row_prefix(ui, app, index, display, track);
                    ui.add_space(8.0);

                    let _ = render_favorite_toggle(ui, app, track, row_height, favorite_button_size, 28.0);
                    ui.add_space(8.0);

                    let art_size = egui::vec2(row_height, row_height);
                    render_track_artwork(ui, app, track, artwork_mode, art_size, is_playing);
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

                        let handled = response_handler(&response, app, index, display, track, &mut outcome);

                        if !handled {
                            click::process_track_activation(
                                &response,
                                activation,
                                index,
                                tracks,
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

                            context_menu_extra(ui, app, index, display, track, &mut outcome);
                        });
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        trailing_actions(ui, app, index, display, track, &mut outcome);
                    });
                });
            });

            ui.add_space(4.0);
        }
    });

    if let Some(measured) = measured_stride {
        if (measured - stride).abs() > 0.5 && measured > 1.0 {
            ui.ctx().data_mut(|d| d.insert_temp(stride_id, measured));
            ui.ctx().request_repaint();
        }
    }

    outcome
}