use crate::presentation::components::lrc_overlay::render_lrc_overlay;
use crate::presentation::app::GuiApp;
use eframe::egui;
use std::time::{Duration, Instant};

fn truncate_end(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_owned();
    }
    if max_chars <= 3 {
        return "...".to_owned();
    }

    let keep = max_chars - 3;
    let start: String = chars[..keep].iter().collect();
    format!("{}...", start)
}

/// 再生コントロール画面のUIを描画する関数。
/// 
/// この画面には以下の要素が含まれる：
/// - 再生コントロール（前の曲、再生/一時停止、次の曲）
/// - ファイルパス入力と操作
/// - 再生モード選択（シーケンシャル、リピート、シャッフル）
/// - 歌詞ファイルの選択と表示（LRCオーバーレイ機能）
/// - アルバムアートワーク表示
/// - シークバーと再生位置表示
/// - キュー操作
pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    // ========== ヘッダー部分 ==========
    ui.heading("Play");

    // キーボードショートカットの処理を登録
    // - スペースキー：再生/一時停止の切り替え
    // - ←/→キー：10秒単位でのシーク（長押しで連続シーク）
    app.handle_play_keyboard_shortcuts(ui.ctx());
    ui.label("Space: play/pause, ←/→: seek 10s (hold for continuous seek)");
    ui.separator();

    // ========== 再生モード選択 ==========
    // 再生モードの種類：
    // - Sequential：順番に再生（最後まで再生したら停止）
    // - Repeat One：1曲リピート
    // - Repeat All：全曲リピート
    // - Shuffle：シャッフル再生
    let mut mode_changed = false;
    ui.horizontal(|ui| {
        ui.label("Playback mode:");
        let button_size = ui.spacing().interact_size * 2.0;
        
        // 各モードの選択ボタンを作成（selectable_valueは選択状態を視覚的に表示するボタン）
        if ui.add_sized(
                button_size,
                egui::SelectableLabel::new(
                    app.playback_mode == lrc_player_domain::model::PlaybackMode::Sequential,
                    "Sequential",
                ),
            ).clicked()
        {
            app.playback_mode = lrc_player_domain::model::PlaybackMode::Sequential;
            mode_changed = true;
        }

        if ui.add_sized(
            button_size,
            egui::SelectableLabel::new(
                app.playback_mode == lrc_player_domain::model::PlaybackMode::RepeatOne,
                "Repeat One",
            ),).clicked()
        {
            app.playback_mode = lrc_player_domain::model::PlaybackMode::RepeatOne;
            mode_changed = true;
        }

        if ui.add_sized(
            button_size,
            egui::SelectableLabel::new(
                app.playback_mode == lrc_player_domain::model::PlaybackMode::RepeatAll,
                "Repeat All",
            ),).clicked()
        {
            app.playback_mode = lrc_player_domain::model::PlaybackMode::RepeatAll;
            mode_changed = true;
        }

        if ui.add_sized(
            button_size,
            egui::SelectableLabel::new(
                app.playback_mode == lrc_player_domain::model::PlaybackMode::Shuffle,
                "Shuffle",
            ),).clicked()
        {
            app.playback_mode = lrc_player_domain::model::PlaybackMode::Shuffle;
            mode_changed = true;
        }
    });
    
    // モードが変更された場合は設定を永続化
    if mode_changed {
        app.save_state();
    }

    // ========== 歌詞表示セクション ==========
    ui.separator();
    ui.heading("Lyrics");

    // LRCオーバーレイ：アルバムアートワーク上に歌詞を重ねて表示する機能
    let mut lrc_overlay_changed = false;
    ui.horizontal(|ui| {
        lrc_overlay_changed = ui
            .checkbox(&mut app.lrc_overlay_enabled, "LRC overlay")
            .changed();
    });
    if lrc_overlay_changed {
        app.save_state();
    }
    
    if app.lyrics_files.is_empty() {
        ui.label("No lyrics files discovered for the current track.");
    } else {
        // 発見された歌詞ファイルの一覧を表示
        for index in 0..app.lyrics_files.len() {
            let file_path = app.lyrics_files[index].clone();
            let file_name = file_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("lyrics");
            let full_path = file_path.display().to_string();
            let full_label = format!("{} — {}", file_name, full_path);
            let display_label = truncate_end(&full_label, 96);
            
            ui.horizontal(|ui| {
                let load_button_width = 48.0;
                let select_width =
                    (ui.available_width() - load_button_width - ui.spacing().item_spacing.x).max(80.0);
                let button_height = ui.spacing().interact_size.y;
                let (select_rect, select_response) = ui.allocate_exact_size(
                    egui::vec2(select_width, button_height),
                    egui::Sense::click(),
                );
                let is_selected = app.selected_lyrics_index == Some(index);
                let fill = if is_selected {
                    ui.visuals().selection.bg_fill
                } else {
                    ui.visuals().widgets.inactive.bg_fill
                };
                ui.painter().rect_filled(select_rect, 4.0, fill);
                let text_style = egui::TextStyle::Button.resolve(ui.style());
                let text_pos = select_rect.left_center() + egui::vec2(8.0, 0.0);
                ui.painter().text(
                    text_pos,
                    egui::Align2::LEFT_CENTER,
                    display_label,
                    text_style,
                    ui.visuals().text_color(),
                );
                if select_response.clicked() {
                    app.load_lyrics_file(index);
                }
                if select_response.hovered() {
                    ui.painter().rect_stroke(select_rect, 4.0, egui::Stroke::new(1.0, ui.visuals().widgets.hovered.bg_fill));
                }
                // 「Load」ボタン：明示的な読み込み用（選択時の自動読み込みの補助）
                if ui
                    .add_sized([load_button_width, ui.spacing().interact_size.y], egui::Button::new("Load"))
                    .clicked()
                {
                    app.load_lyrics_file(index);
                }
            });
        }
    }

    // ========== 現在再生中のトラック情報表示 ==========
    if !app.play_path.trim().is_empty() {
        // アートワーク画像を必要に応じて読み込み
        app.ensure_play_artwork(ui.ctx());
        
        // 表示用テクスチャの選択
        // LRCオーバーレイ有効時はぼかし処理済みの画像を優先（歌詞の可読性向上のため）
        let display_texture = if app.lrc_overlay_enabled {
            app.play_artwork_blur_handle.as_ref().or(app.play_artwork_handle.as_ref())
        } else {
            app.play_artwork_handle.as_ref()
        };

        if let Some(texture) = display_texture {
            let available = ui.available_size();
            let available_width = available.x.max(1.0);
            // 画面内で他UIを除いた残り領域をアートワークに使うため、
            // 下部の各ウィジェットの実際の高さを計算する。
            let text_height = ui.text_style_height(&egui::TextStyle::Body);
            let item_spacing = ui.spacing().item_spacing.y;
            let control_height = ui.spacing().interact_size.y.max(text_height) * 2.0;
            let separator_height = 8.0;
            let mut bottom_height = 0.0;

            // Duration / unknown
            bottom_height += text_height + item_spacing;

            // Lyrics offset row
            bottom_height += control_height + item_spacing;

            // Seek slider row
            bottom_height += control_height + item_spacing;

            // Current position label
            bottom_height += text_height + item_spacing;

            // Queue command buttons
            bottom_height += control_height + item_spacing;

            // Queue count label
            bottom_height += text_height;

            // 分離線の高さ
            bottom_height += separator_height * 2.0;

            // アートワーク直後の余白
            bottom_height += 12.0;

            let max_side = available_width
                .min((available.y - bottom_height).max(1.0))
                .clamp(1.0, 1080.0);
            let image_size = egui::vec2(max_side, max_side);
            let response = ui
                .allocate_ui_with_layout(
                    egui::vec2(available_width, image_size.y),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| ui.image(texture, image_size),
                )
                .inner;

            // LRCオーバーレイが有効な場合、アートワーク上に歌詞を重ねて表示
            if app.lrc_overlay_enabled {
                let rect = response.rect;
                // 半透明の黒いオーバーレイで歌詞の可読性を向上
                ui.painter().rect_filled(rect, 0.0, egui::Color32::from_black_alpha(120));
                if let Some(lyrics) = &app.loaded_lyrics {
                    // 歌詞オフセットを適用した現在再生時間を計算
                    let current_time_ms = if app.lyrics_offset_ms.is_negative() {
                        // 負のオフセット：再生位置から減算（歌詞を遅らせる）
                        app.play_position_ms.saturating_sub((-app.lyrics_offset_ms) as u64)
                    } else {
                        // 正のオフセット：再生位置に加算（歌詞を早める）
                        app.play_position_ms.saturating_add(app.lyrics_offset_ms as u64)
                    };
                    render_lrc_overlay(ui, rect, lyrics, current_time_ms);
                }
            }

            ui.add_space(12.0);
        }

        // 再生時間とシークバーの表示
        if let Some(duration_ms) = app.play_duration_ms {
            // 総再生時間を「分:秒.ミリ秒」形式で表示
            let mins = duration_ms / 60000;
            let secs = (duration_ms / 1000) % 60;
            let millis = duration_ms % 1000;
            ui.label(format!("Duration: {:02}:{:02}.{:03}", mins, secs, millis));

            // 歌詞オフセット調整ボタン（±100ms / ±250ms）
            let mut lyrics_offset_changed = false;
            ui.horizontal(|ui| {
                if ui.button("-250 ms").clicked() {
                    app.lyrics_offset_ms = app.lyrics_offset_ms.saturating_sub(250).clamp(-30_000, 30_000);
                    lyrics_offset_changed = true;
                }
                if ui.button("-100 ms").clicked() {
                    app.lyrics_offset_ms = app.lyrics_offset_ms.saturating_sub(100).clamp(-30_000, 30_000);
                    lyrics_offset_changed = true;
                }
                ui.label(format!("Offset: {:+} ms", app.lyrics_offset_ms));
                if ui.button("+100 ms").clicked() {
                    app.lyrics_offset_ms = app.lyrics_offset_ms.saturating_add(100).clamp(-30_000, 30_000);
                    lyrics_offset_changed = true;
                }
                if ui.button("+250 ms").clicked() {
                    app.lyrics_offset_ms = app.lyrics_offset_ms.saturating_add(250).clamp(-30_000, 30_000);
                    lyrics_offset_changed = true;
                }
            });
            if lyrics_offset_changed {
                app.save_state();
            }

            ui.horizontal(|ui| {
                let save_enabled = app.selected_lyrics_index.is_some() && app.lyrics_offset_ms != 0;
                if ui
                    .add_enabled(save_enabled, egui::Button::new("Save offset to LRC"))
                    .clicked()
                {
                    app.save_lyrics_offset_to_selected_file();
                }

                let reset_enabled = app.lyrics_offset_ms != 0;
                if ui.add_enabled(reset_enabled, egui::Button::new("Reset offset")).clicked() {
                    app.lyrics_offset_ms = 0;
                    app.save_state();
                }

                if !save_enabled {
                    ui.label("Select lyrics file and set non-zero offset to save.");
                }
            });

            // シークバー（再生位置調整用スライダー）
            let slider = egui::Slider::new(&mut app.play_position_ms, 0..=duration_ms).text("Position ms");
            let response = ui
                .scope(|ui| {
                    ui.spacing_mut().slider_width = ui.available_width();  // 横幅いっぱいに表示
                    ui.add(slider)
                })
                .inner;
            
            // シーク位置が変更された場合、シーク処理をキューに追加し状態を保存
            if response.changed() {
                app.queue_seek_position(app.play_position_ms);
                app.save_state();
            }

            // 長押しによる連続シーク処理
            // スライダーをクリックしたまま350ms以上経過すると連続シークモードに移行
            let pointer_down = response.is_pointer_button_down_on();
            let mut slider_interacting = response.dragged() || response.has_focus();
            if pointer_down {
                // 長押しの開始時間を記録
                if app.seek_long_press_elapsed().is_none() {
                    app.set_seek_long_press_start(Some(Instant::now()));
                } else if !app.is_seek_interacting()
                    && app.seek_long_press_elapsed().unwrap() >= Duration::from_millis(350)
                {
                    // 350ms以上押し続けたら連続シークとして扱う
                    slider_interacting = true;
                }
            } else {
                // ボタンが離されたら長押し状態を解除
                app.set_seek_long_press_start(None);
            }

            app.set_seek_interacting(slider_interacting);

            // 現在の再生位置を「分:秒.ミリ秒」形式で表示
            let pos = app.play_position_ms.min(duration_ms);
            let mins = pos / 60000;
            let secs = (pos / 1000) % 60;
            let millis = pos % 1000;
            ui.label(format!("Current position: {:02}:{:02}.{:03}", mins, secs, millis));
        } else {
            ui.label("Duration: unknown. Discover metadata to enable seek.");
        }
    }

    // ========== 再生コントロールボタン ==========
    let button_size = ui.spacing().interact_size * 2.0;
    let spacing_x = ui.spacing().item_spacing.x;
    let total_width = button_size.x * 3.0 + spacing_x * 2.0;
    let (row_rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width().max(1.0), button_size.y), egui::Sense::hover());
    let viewport_rect = ui.max_rect().intersect(ui.clip_rect());
    let center_x = viewport_rect.center().x;
    let start_x = (center_x - total_width * 0.5).clamp(row_rect.min.x, row_rect.max.x - total_width);
    let mut x = start_x;

    // 「前の曲」ボタン：キューまたは履歴から前の曲を再生
    let prev_rect = egui::Rect::from_min_size(egui::pos2(x, row_rect.min.y), button_size);
    if ui.put(prev_rect, egui::Button::new("Previous")).clicked() {
        app.play_previous();
    }
    x += button_size.x + spacing_x;

    // 再生/一時停止ボタン：現在の状態に応じてラベルを動的に変更
    let play_label = match app.playback_state {
        lrc_player_domain::model::PlaybackState::Playing => "Pause",   // 再生中 → 一時停止
        lrc_player_domain::model::PlaybackState::Paused => "Resume",   // 一時停止中 → 再開
        _ => "Play",  // 停止中/バッファリング中など → 再生開始
    };
    let play_rect = egui::Rect::from_min_size(egui::pos2(x, row_rect.min.y), button_size);
    if ui.put(play_rect, egui::Button::new(play_label)).clicked() {
        app.toggle_play_pause();
    }
    x += button_size.x + spacing_x;

    // 「次の曲」ボタン：キューまたは次のファイルを再生
    let next_rect = egui::Rect::from_min_size(egui::pos2(x, row_rect.min.y), button_size);
    if ui.put(next_rect, egui::Button::new("Next")).clicked() {
        app.play_next();
    }

    ui.separator();
    
    // ========== キュー操作ボタン ==========
    ui.horizontal(|ui| {
        // 「Play selected queue item」ボタン：キューで選択中の項目を再生
        if ui.button("Play selected queue item").clicked() {
            app.play_selected_queue_item();
        }
        // 「Clear play path」ボタン：再生パスと関連データをすべてクリア
        if ui.button("Clear play path").clicked() {
            app.play_path.clear();
            app.lyrics_files.clear();
            app.loaded_lyrics = None;
            app.selected_lyrics_index = None;
            app.play_duration_ms = None;
            app.play_title = None;
            app.play_artist = None;
            app.play_album = None;
            app.play_artwork_handle = None;
            app.play_artwork_source = None;
            app.lyrics_offset_ms = 0;
            app.play_position_ms = 0;
            app.save_state();
        }
    });

    ui.separator();
    // キュー内のトラック数を表示
    ui.label(format!("{} tracks in queue.", app.app_vm.queue.tracks.len()));
}
