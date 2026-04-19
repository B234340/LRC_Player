use crate::presentation::egui_app::GuiApp;
use eframe::egui;
use std::collections::HashMap;

/// アルバムタイトルを指定された文字数制限と行数で折り返す関数
/// 
/// # 引数
/// * `title` - 折り返すアルバムタイトル
/// * `max_chars_per_line` - 1行あたりの最大文字数
/// * `max_lines` - 最大行数
/// 
/// # 戻り値
/// 改行で区切られた折り返し済みの文字列
fn wrap_album_title(title: &str, max_chars_per_line: usize, max_lines: usize) -> String {
    // 空文字列または制限値が0の場合はそのまま返す
    if title.is_empty() || max_chars_per_line == 0 || max_lines == 0 {
        return title.to_string();
    }

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();

    // 単語ごとに分割して処理
    for word in title.split_whitespace() {
        // 次の単語を追加した場合の文字数を計算
        let next_len = if current.is_empty() {
            word.chars().count()
        } else {
            current.chars().count() + 1 + word.chars().count()  // +1はスペース分
        };

        // 制限文字数以内なら現在の行に追加
        if next_len <= max_chars_per_line {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
            continue;
        }

        // 現在の行を保存して新しい行を開始
        if !current.is_empty() {
            lines.push(current);
            current = String::new();
        }

        // 単語が制限文字数を超える場合は文字単位で分割
        let mut chunk = String::new();
        for ch in word.chars() {
            if chunk.chars().count() >= max_chars_per_line {
                lines.push(chunk);
                chunk = String::new();
            }
            chunk.push(ch);
        }
        if !chunk.is_empty() {
            current = chunk;
        }

        // 最大行数に達したら処理を中断
        if lines.len() + 1 >= max_lines {
            break;
        }
    }

    // 残りの文字列を追加
    if lines.len() < max_lines && !current.is_empty() {
        lines.push(current);
    }

    // 最大行数を超えた分を切り詰め
    if lines.len() > max_lines {
        lines.truncate(max_lines);
    }

    // 切り詰めが発生したかどうかを判定
    let was_truncated = title.chars().count() > lines.join(" ").chars().count();
    if lines.len() == max_lines {
        if let Some(last) = lines.last_mut() {
            // 最終行が切り詰められた場合、末尾に「...」を追加
            if was_truncated && last.chars().count() > 3 {
                while last.chars().count() > max_chars_per_line.saturating_sub(3) {
                    last.pop();
                }
                last.push_str("...");
            }
        }
    }

    // 結果を改行で結合して返す
    if lines.is_empty() {
        title.to_string()
    } else {
        lines.join("\n")
    }
}

/// アルバムIDをアルバムタイトルと所有者（アルバムアーティスト）に分割する関数
/// 
/// アルバムIDは「タイトル\0所有者」の形式で保存されている
fn decode_album_id(album_id: &str) -> (&str, &str) {
    album_id.split_once('\0').unwrap_or((album_id, ""))
}

/// アルバム一覧画面のUIを描画する関数
pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    // アプリケーション状態からアルバム情報を取得
    let albums = app.app_vm.library.albums.clone();
    let selected_album = app.app_vm.library.selected_album.clone();
    let selected_album_tracks = app.app_vm.library.selected_album_tracks.clone();
    
    // 遅延実行するアクションを保持する変数群
    let mut action: Option<Option<String>> = None;  // アルバム選択アクション
    let mut pending_play_index: Option<usize> = None;  // 再生開始インデックス
    let mut pending_play_list: Vec<String> = Vec::new();  // 再生リスト
    let mut pending_play_now_path: Option<String> = None;  // 即時再生するトラックパス
    let mut pending_next_path: Option<String> = None;  // 次に再生するトラックパス
    let mut pending_enqueue_path: Option<String> = None;  // キューに追加するトラックパス
    let mut pending_add_to_playlist: Option<(usize, String)> = None;  // プレイリストに追加するトラック
    let mut pending_show_details_path: Option<String> = None;  // 詳細表示するトラックパス
    
    // メタデータキャッシュを開く
    let meta_cache = crate::infrastructure::persistence::MetaCache::open_default().ok();
    let mut track_metadata_cache: HashMap<String, (String, String)> = HashMap::new();
    
    // プレイリストの一覧を作成（インデックスと名前のペア）
    let playlist_targets: Vec<(usize, String)> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(idx, p)| (idx, p.name.clone()))
        .collect();

    // ヘッダー表示
    ui.heading("Albums");
    ui.label(format!("{} albums in library", albums.len()));
    ui.separator();

    if albums.is_empty() {
        ui.label("No albums available.");
    } else {
        let content_rect = ui.max_rect();  // コンテンツ領域全体の矩形
        let mut selected_card_rect: Option<egui::Rect> = None;  // 選択中カードの矩形
        let mut grid_visible_rect: Option<egui::Rect> = None;  // 表示中のグリッド領域

        // カードレイアウトの寸法設定
        let card_width = 170.0_f32;  // カードの幅
        let grid_spacing_x = 10.0_f32;  // カード間の横方向余白
        let grid_spacing_y = 10.0_f32;  // カード間の縦方向余白
        
        // Frame::groupは内部マージンを追加するため、右端のクリッピングを避けるために
        // カード外側の余分な幅を予約する
        let card_outer_extra_x = 16.0_f32;
        let card_outer_width = card_width + card_outer_extra_x;
        let card_inner_padding = 8.0_f32;  // カード内部のパディング
        let card_button_height = 44.0_f32;  // ボタンの高さ
        // カード1行分の高さ（画像 + 余白 + ボタン + パディング）
        let card_row_height = (card_width - 16.0) + 6.0 + card_button_height + card_inner_padding * 2.0 + 8.0;
        
        // 1行に表示できるカード数を計算（利用可能な幅に基づく）
        let cols = ((ui.available_width() + grid_spacing_x) / (card_outer_width + grid_spacing_x))
            .floor()
            .max(1.0) as usize;

        // 総行数を計算
        let row_count = (albums.len() + cols - 1) / cols;
        let mut visible_album_titles: Vec<String> = Vec::new();  // 表示中のアルバムタイトル一覧

        // スクロール可能な領域でアルバムカードを表示
        egui::ScrollArea::vertical()
            .id_source("album_cards_scroll")
            .auto_shrink([false, false])
            .show_rows(ui, card_row_height + grid_spacing_y, row_count, |ui, row_range| {
                grid_visible_rect = Some(ui.clip_rect());  // 現在表示中の領域を記録
                
                // 各行を描画
                for row in row_range {
                    ui.horizontal(|ui| {
                        // 各行のカラム（カード）を描画
                        for col in 0..cols {
                            let index = row * cols + col;
                            if index >= albums.len() {
                                break;  // アルバム数を超えたら終了
                            }

                            let album = &albums[index];
                            visible_album_titles.push(album.title.clone());
                            let is_selected = selected_album.as_deref() == Some(album.id.as_str());

                            // 選択状態に応じてフレームのスタイルを変更
                            let frame = if is_selected {
                                egui::Frame::group(ui.style())
                                    .fill(ui.visuals().selection.bg_fill)  // 選択時の背景色
                                    .stroke(egui::Stroke::new(1.5, ui.visuals().selection.stroke.color))
                            } else {
                                egui::Frame::group(ui.style())
                            };

                            // カードの描画
                            let card_response = frame.show(ui, |ui| {
                                ui.set_min_width(card_width);
                                ui.set_max_width(card_width);
                                ui.vertical(|ui| {
                                    // アルバムアートワークの表示
                                    let art_size = egui::vec2(card_width - 16.0, card_width - 16.0);
                                    if let Some(texture) = app.album_artwork_texture(album.title.as_str()) {
                                        ui.image(&texture, art_size);  // アートワーク画像を表示
                                    } else {
                                        // アートワークがない場合はプレースホルダーを表示
                                        let (rect, _) = ui.allocate_exact_size(art_size, egui::Sense::hover());
                                        let art_bg = if is_selected {
                                            ui.visuals().selection.bg_fill
                                        } else {
                                            ui.visuals().faint_bg_color
                                        };
                                        ui.painter().rect_filled(rect, 8.0, art_bg);

                                        let text_color = if is_selected {
                                            ui.visuals().selection.stroke.color
                                        } else {
                                            ui.visuals().text_color()
                                        };
                                        ui.painter().text(
                                            rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "ART",  // プレースホルダーテキスト
                                            egui::TextStyle::Button.resolve(ui.style()),
                                            text_color,
                                        );
                                    }

                                    ui.add_space(6.0);
                                    
                                    // アルバムタイトルボタン（折り返し処理済み）
                                    let wrapped_title = wrap_album_title(album.title.as_str(), 22, 2);
                                    let button = egui::Button::new(wrapped_title);
                                    if ui
                                        .add_sized([card_width - 16.0, 44.0], button)
                                        .clicked()
                                    {
                                        // クリックでアルバム選択状態を切り替え
                                        if is_selected {
                                            action = Some(None);  // 選択解除
                                        } else {
                                            action = Some(Some(album.id.clone()));  // 選択
                                        }
                                    }
                                });
                            });

                            // 選択中のカードの矩形を保存
                            if is_selected {
                                selected_card_rect = Some(card_response.response.rect);
                            }

                            // カード間の横方向スペース
                            if col + 1 < cols {
                                ui.add_space(grid_spacing_x);
                            }
                        }
                    });
                    ui.add_space(grid_spacing_y);  // 行間のスペース
                }
            });

        // 表示中のアルバムのアートワークを非同期で読み込み
        if row_count > 0 {
            app.queue_album_artwork_for_visible(&visible_album_titles);
            app.process_album_artwork_queue(ui.ctx(), 3);  // 一度に3枚まで処理
        }

        // 選択中のアルバムカードが表示領域内にあるか確認
        let selected_is_visible = match (selected_card_rect, grid_visible_rect) {
            (Some(card), Some(viewport)) => card.intersects(viewport),
            _ => false,
        };

        // 選択中のアルバムが表示されている場合、詳細パネルを表示
        if selected_is_visible {
            if let Some(selected) = selected_album.as_deref() {
                // 詳細パネルの寸法を計算
                let detail_max_size = card_width * 2.0 + grid_spacing_x;
                let panel_width = if cols <= 3 {
                    card_width
                } else {
                    detail_max_size
                }
                .min(content_rect.width() - 20.0)
                .max(card_width);

                let mut panel_height = (content_rect.height() * 0.62)
                    .clamp(card_width, detail_max_size)
                    .min(content_rect.height() - 20.0);
                let min_panel_height = 120.0_f32;
                let gap = 10.0;  // カードとパネル間の隙間

                // パネルの基準位置（選択カードの位置）
                let anchor_rect = selected_card_rect.unwrap_or_else(|| {
                    egui::Rect::from_min_size(
                        content_rect.left_top(),
                        egui::vec2(card_width, card_width),
                    )
                });

                let mut panel_x;
                let mut panel_y;
                let mut show_overlay = true;
                let is_single_col = cols == 1;  // 1列表示かどうか

                // パネルの配置位置を計算
                if is_single_col {
                    // 1列表示の場合：上下どちらかに配置
                    panel_x = anchor_rect.left();
                    let below_space = content_rect.bottom() - anchor_rect.bottom();
                    let above_space = anchor_rect.top() - content_rect.top();
                    let available_side = below_space.max(above_space) - gap;
                    
                    if available_side < min_panel_height {
                        show_overlay = false;  // 十分なスペースがない場合は表示しない
                    }
                    panel_height = panel_height.min(available_side.max(min_panel_height));

                    // 下側のスペースが広ければ下に、そうでなければ上に配置
                    if below_space >= above_space {
                        panel_y = anchor_rect.bottom() + gap;
                    } else {
                        panel_y = anchor_rect.top() - panel_height - gap;
                    }
                } else {
                    // 複数列表示の場合：主に右側に配置
                    panel_x = anchor_rect.right() + gap;
                    if panel_x + panel_width > content_rect.right() {
                        panel_x = anchor_rect.left() - panel_width - gap;  // 右に配置できない場合は左に
                    }
                    panel_x = panel_x.clamp(
                        content_rect.left(),
                        (content_rect.right() - panel_width).max(content_rect.left()),
                    );

                    panel_y = anchor_rect.top();
                    if panel_y + panel_height > content_rect.bottom() {
                        panel_y = (content_rect.bottom() - panel_height).max(content_rect.top());
                    }
                    panel_y = panel_y.max(content_rect.top());

                    // パネルがカードと重なる場合の調整
                    let panel_rect = egui::Rect::from_min_size(
                        egui::pos2(panel_x, panel_y),
                        egui::vec2(panel_width, panel_height),
                    );
                    if panel_rect.intersects(anchor_rect) {
                        let below_space = content_rect.bottom() - anchor_rect.bottom();
                        let above_space = anchor_rect.top() - content_rect.top();
                        let available_side = below_space.max(above_space) - gap;
                        
                        if available_side < min_panel_height {
                            show_overlay = false;
                        } else {
                            panel_height = panel_height.min(available_side.max(min_panel_height));
                            panel_x = anchor_rect.left().clamp(
                                content_rect.left(),
                                (content_rect.right() - panel_width).max(content_rect.left()),
                            );
                            // 上下の空きスペースに応じて配置
                            if below_space >= above_space {
                                panel_y = anchor_rect.bottom() + gap;
                            } else {
                                panel_y = anchor_rect.top() - panel_height - gap;
                            }
                        }
                    }
                }

                // 詳細オーバーレイパネルの表示
                if show_overlay {
                    let ctx = ui.ctx();
                    egui::Window::new("Album Details")
                        .id(egui::Id::new("album_detail_overlay"))
                        .title_bar(false)  // タイトルバーなし
                        .collapsible(false)  // 折りたたみ不可
                        .resizable(false)  // リサイズ不可
                        .default_size(egui::vec2(panel_width, panel_height))
                        .fixed_pos(egui::pos2(panel_x, panel_y))  // 固定位置
                        .show(ctx, |ui| {
                            ui.set_width(panel_width);
                            ui.set_min_height(panel_height);
                            ui.heading("Album Details");
                            ui.separator();
                            
                            // アルバム情報の表示
                            let (album_title, album_owner) = decode_album_id(selected);
                            if album_owner.is_empty() {
                                ui.label(format!("Album: {}", album_title));
                            } else {
                                ui.label(format!("Album: {}", album_title));
                                ui.label(format!("Album artist: {}", album_owner));
                            }
                            ui.add_space(4.0);

                            if selected_album_tracks.is_empty() {
                                ui.label("No tracks found for this album.");
                            } else {
                                ui.label(format!("{} tracks", selected_album_tracks.len()));
                                ui.label("Double left click: create queue from this list and play. Right click: common actions.");
                                ui.add_space(6.0);
                                
                                // トラック一覧のスクロール領域
                                egui::ScrollArea::vertical()
                                    .id_source("album_detail_tracks_scroll")
                                    .auto_shrink([false, false])
                                    .max_height((panel_height - 84.0).max(120.0))
                                    .show(ui, |ui| {
                                        for (i, track) in selected_album_tracks.iter().enumerate() {
                                            ui.horizontal(|ui| {
                                                // トラックの表示タイトルを取得（ファイル名から抽出）
                                                let fallback_title = std::path::Path::new(&track.title)
                                                    .file_name()
                                                    .and_then(|name| name.to_str())
                                                    .unwrap_or(track.title.as_str())
                                                    .to_string();
                                                
                                                // メタデータキャッシュからトラック情報を取得
                                                let display_meta = track_metadata_cache
                                                    .entry(track.title.clone())
                                                    .or_insert_with(|| {
                                                        meta_cache
                                                            .as_ref()
                                                            .and_then(|cache| {
                                                                cache
                                                                    .track_metadata_by_location(&track.title)
                                                                    .ok()
                                                                    .flatten()
                                                            })
                                                            .map(|(title, artist, _)| {
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
                                                                (title, artist)
                                                            })
                                                            .unwrap_or((fallback_title.clone(), "Unknown artist".to_string()))
                                                    })
                                                    .clone();
                                                let (display_title, display_artist) = display_meta;

                                                // トラック情報のラベル
                                                let label = format!("{:>2}. {} - {}", i + 1, display_title, display_artist);
                                                
                                                // お気に入りスターの表示
                                                let is_favorite = app.is_track_favorite(&track.title);
                                                let star_label = if is_favorite { "★" } else { "☆" };
                                                let toggle_response = ui
                                                    .allocate_ui_with_layout(
                                                        egui::vec2(24.0, 22.0),
                                                        egui::Layout::top_down(egui::Align::Center),
                                                        |ui| {
                                                            ui.add_sized([22.0, 22.0], egui::Button::new(star_label))
                                                        },
                                                    )
                                                    .inner
                                                    .on_hover_text("Toggle favorite");
                                                
                                                if toggle_response.clicked() {
                                                    app.toggle_favorite_track(&track.title);
                                                }
                                                
                                                ui.add_space(4.0);
                                                let response = ui.selectable_label(false, label);
                                                
                                                // ダブルクリックで再生開始
                                                if response.double_clicked() {
                                                    pending_play_index = Some(i);
                                                    pending_play_list = selected_album_tracks
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
                                                    for (target_index, target_name) in &playlist_targets {
                                                        if ui.button(target_name).clicked() {
                                                            pending_add_to_playlist =
                                                                Some((*target_index, track.title.clone()));
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
                                        }
                                    });
                                }
                            });
                        }
                    }
                }
            }

    // 保留中のアクションを実行
    if let Some(next) = action {
        app.select_album(next);
    }
    if let Some(index) = pending_play_index {
        app.play_from_displayed_list(&pending_play_list, index);
    }
    if let Some(path) = pending_play_now_path {
        app.play_track_now(&path);
    }
    if let Some(path) = pending_next_path {
        app.add_track_next_in_queue(&path);
    }
    if let Some(path) = pending_enqueue_path {
        app.enqueue_track_path(&path);
    }
    if let Some((target_index, track)) = pending_add_to_playlist {
        app.add_track_to_playlist(target_index, &track);
    }
    if let Some(path) = pending_show_details_path {
        app.open_track_details(&path);
    }
}