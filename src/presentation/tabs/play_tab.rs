use crate::presentation::egui_app::GuiApp;
use eframe::egui;
use std::path::Path;
use std::time::{Duration, Instant};

/// 再生画面のUIを描画する関数
/// 
/// # 引数
/// * `ui` - eguiのUIビルダー
/// * `app` - アプリケーションの状態を保持するGuiAppインスタンス
pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    // --- ヘッダー部分 ---
    ui.heading("Play");
    ui.label("Enter a file path or select a queued track, then press Play.");
    ui.add_space(4.0);
    
    // キーボードショートカットの処理（スペースキーで再生/一時停止、矢印キーでシーク）
    app.handle_play_keyboard_shortcuts(ui.ctx());
    ui.label("Space: play/pause, ←/→: seek 10s (hold for continuous seek)");
    ui.separator();

    // --- 再生コントロールボタン（前の曲、再生/一時停止、次の曲）---
    ui.horizontal(|ui| {
        if ui.button("Previous").clicked() {
            app.play_previous();  // 前の曲を再生
        }
        
        // 再生状態に応じてボタンのラベルを動的に変更
        let play_label = match app.playback_state {
            crate::domain::model::PlaybackState::Playing => "Pause",   // 再生中なら「一時停止」
            crate::domain::model::PlaybackState::Paused => "Resume",   // 一時停止中なら「再開」
            _ => "Play",  // それ以外（停止中など）は「再生」
        };
        if ui.button(play_label).clicked() {
            app.toggle_play_pause();  // 再生/一時停止を切り替え
        }
        
        if ui.button("Next").clicked() {
            app.play_next();  // 次の曲を再生
        }
    });

    // --- ファイルパス入力と操作ボタン ---
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.play_path);  // 再生するファイルのパス入力欄
        
        if ui.button("Play path").clicked() {
            app.play_position_ms = 0;  // 再生位置を先頭にリセット
            app.playback_state = crate::domain::model::PlaybackState::Buffering;  // バッファリング状態に設定
            app.discover_lyrics_for_current_play_path();  // 歌詞ファイルを探索
            app.request_play();  // 再生をリクエスト
        }
        
        if ui.button("Discover metadata").clicked() {
            app.refresh_play_metadata();  // メタデータ（曲名、アーティスト名など）を更新
            app.discover_lyrics_for_current_play_path();  // 歌詞ファイルを再探索
        }
    });

    // 再生状態とステータスの表示
    ui.label(format!("Playback state: {}", app.playback_state));
    if let Some(status) = &app.play_status {
        ui.label(format!("Status: {}", status));
    }

    // --- 再生モード選択（シーケンシャル、リピート1曲、リピート全曲、シャッフル）---
    let mut mode_changed = false;
    ui.horizontal(|ui| {
        ui.label("Playback mode:");
        
        // 各再生モードの選択ボタンを作成
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, crate::domain::model::PlaybackMode::Sequential, "Sequential")
            .changed();
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, crate::domain::model::PlaybackMode::RepeatOne, "Repeat One")
            .changed();
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, crate::domain::model::PlaybackMode::RepeatAll, "Repeat All")
            .changed();
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, crate::domain::model::PlaybackMode::Shuffle, "Shuffle")
            .changed();
    });
    
    // モードが変更されたら状態を保存
    if mode_changed {
        app.save_state();
    }

    // --- 現在再生中のトラック情報表示 ---
    if !app.play_path.trim().is_empty() {
        app.ensure_play_artwork(ui.ctx());  // アートワーク画像を読み込み
        
        // アートワーク画像の表示（アルバムアート）
        if let Some(texture) = &app.play_artwork_handle {
            let available = ui.available_size();
            // 画像サイズを計算（画面サイズに応じて調整、最小128px、最大640px）
            let max_side = (available.x.min(available.y) * 0.45).clamp(128.0, 640.0);
            let mut image_size = texture.size_vec2();
            let scale = (max_side / image_size.x).min(max_side / image_size.y).min(1.0);
            image_size *= scale;
            ui.image(texture, image_size);
            ui.add_space(8.0);
        }

        // トラック情報の表示
        let path = Path::new(&app.play_path);
        ui.label(format!("Track path: {}", app.play_path));
        ui.label(format!("Exists: {}", path.exists()));  // ファイルの存在確認
        
        // メタデータが取得できていれば表示
        if let Some(title) = &app.play_title {
            ui.label(format!("Title: {}", title));
        }
        if let Some(artist) = &app.play_artist {
            ui.label(format!("Artist: {}", artist));
        }
        if let Some(album) = &app.play_album {
            ui.label(format!("Album: {}", album));
        }
        if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
            ui.label(format!("Filename: {}", filename));
        }
        
        // 再生時間とシークバーの表示
        if let Some(duration_ms) = app.play_duration_ms {
            // 総再生時間を「分:秒.ミリ秒」形式で表示
            let mins = duration_ms / 60000;
            let secs = (duration_ms / 1000) % 60;
            let millis = duration_ms % 1000;
            ui.label(format!("Duration: {:02}:{:02}.{:03}", mins, secs, millis));

            // 歌詞のオフセット調整スライダー
            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut app.lyrics_offset_ms, -30_000..=30_000).text("Lyrics offset ms"));
                ui.label(format!("({:+} ms)", app.lyrics_offset_ms));
            });

            // シークバー（再生位置調整）
            let slider = egui::Slider::new(&mut app.play_position_ms, 0..=duration_ms).text("Position ms");
            let response = ui
                .scope(|ui| {
                    ui.spacing_mut().slider_width = ui.available_width();  // スライダーを横幅いっぱいに
                    ui.add(slider)
                })
                .inner;
            
            // シーク位置が変更されたら、シークを実行して状態を保存
            if response.changed() {
                app.queue_seek_position(app.play_position_ms);
                app.save_state();
            }

            // 長押しによる連続シーク処理
            let pointer_down = response.is_pointer_button_down_on();
            let mut slider_interacting = response.dragged() || response.has_focus();
            if pointer_down {
                // 長押しの開始時間を記録
                if app.seek_long_press_elapsed().is_none() {
                    app.set_seek_long_press_start(Some(Instant::now()));
                } else if !app.is_seek_interacting()
                    && app.seek_long_press_elapsed().unwrap() >= Duration::from_millis(350)  // 350ms以上押し続けたら連続シークモード
                {
                    slider_interacting = true;
                }
            } else {
                app.set_seek_long_press_start(None);  // ボタンが離されたら長押し状態を解除
            }

            app.set_seek_interacting(slider_interacting);

            // 現在の再生位置を表示
            let pos = app.play_position_ms.min(duration_ms);
            let mins = pos / 60000;
            let secs = (pos / 1000) % 60;
            let millis = pos % 1000;
            ui.label(format!("Current position: {:02}:{:02}.{:03}", mins, secs, millis));
        } else {
            ui.label("Duration: unknown. Discover metadata to enable seek.");
        }
    }

    ui.separator();
    
    // --- キュー内の選択項目表示 ---
    if let Some(index) = app.selected_queue_index {
        if let Some(track) = app.app_vm.queue.tracks.get(index) {
            ui.label(format!("Selected queue item: {}", track));
        }
    } else {
        ui.label("No queue item selected.");
    }

    ui.separator();
    
    // --- キュー操作ボタン ---
    ui.horizontal(|ui| {
        if ui.button("Play selected queue item").clicked() {
            app.play_selected_queue_item();  // キューで選択中の項目を再生
        }
        if ui.button("Clear play path").clicked() {
            // 再生パスと関連データをすべてクリア
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
    ui.label(format!("{} tracks in queue.", app.app_vm.queue.tracks.len()));

    // --- 歌詞表示セクション ---
    ui.separator();
    ui.heading("Lyrics");
    
    if app.lyrics_files.is_empty() {
        ui.label("No lyrics files discovered for the current track.");
    } else {
        // 見つかった歌詞ファイルの一覧を表示
        for index in 0..app.lyrics_files.len() {
            let file_path = app.lyrics_files[index].clone();
            let file_name = file_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("lyrics");
            let label = format!("{} — {}", file_name, file_path.display());
            
            ui.horizontal(|ui| {
                // 歌詞ファイルを選択
                if ui
                    .selectable_label(app.selected_lyrics_index == Some(index), label)
                    .clicked()
                {
                    app.load_lyrics_file(index);
                }
                // 「Load」ボタンで明示的に読み込み
                if ui.button("Load").clicked() {
                    app.load_lyrics_file(index);
                }
            });
        }
    }

    // 読み込まれた歌詞の表示
    if let Some(lyrics) = &app.loaded_lyrics {
        ui.separator();
        ui.label("Lyrics preview:");

        // 歌詞オフセットを適用した現在の再生時間を計算
        let current_time_ms = if app.lyrics_offset_ms.is_negative() {
            app.play_position_ms.saturating_sub((-app.lyrics_offset_ms) as u64)
        } else {
            app.play_position_ms.saturating_add(app.lyrics_offset_ms as u64)
        };
        
        // 現在の再生時間に対応する歌詞の行を特定（逆順で検索）
        let highlighted_line_index = lyrics
            .lines
            .iter()
            .enumerate()
            .rev()  // 後ろから検索して最初に見つかったもの = 現在時間以下の最大の時間を持つ行
            .find(|(_, line)| line.time_ms <= current_time_ms)
            .map(|(index, _)| index);

        // スクロール可能な歌詞表示エリア
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(300.0)
            .show(ui, |ui| {
                for (index, line) in lyrics.lines.iter().enumerate() {
                    let is_current_line = Some(index) == highlighted_line_index;
                    let label_text = line.text.clone();

                    let response = if is_current_line {
                        // 現在の行の場合、セグメント単位での進行状況表示
                        if line.segments.len() > 1 {
                            // フォントと色の設定
                            let font_id = egui::TextStyle::Body.resolve(ui.style());
                            let text_color = ui.style().visuals.text_color();
                            let highlight_color = egui::Color32::from_rgb(40, 40, 80);  // ハイライト背景色

                            let mut prefix_text = String::new();
                            let mut highlight_width = 0.0;
                            
                            // テキスト全体の幅を計算
                            let full_text_width = ui.fonts(|fonts| {
                                let mut job = egui::text::LayoutJob::default();
                                job.append(&line.text, 0.0, egui::TextFormat::simple(font_id.clone(), text_color));
                                fonts.layout_job(job).size().x
                            });

                            // 各セグメントの進行状況を計算
                            for (seg_index, segment) in line.segments.iter().enumerate() {
                                let seg_start = segment.time_ms;
                                let seg_end = line.segments.get(seg_index + 1).map(|next| next.time_ms);

                                // すでに終了したセグメントはprefix_textに追加
                                if let Some(end_ms) = seg_end {
                                    if current_time_ms >= end_ms {
                                        prefix_text.push_str(&segment.text);
                                        continue;
                                    }
                                }

                                // prefix_textまでの幅を計算
                                let prefix_width = ui.fonts(|fonts| {
                                    let mut job = egui::text::LayoutJob::default();
                                    job.append(&prefix_text, 0.0, egui::TextFormat::simple(font_id.clone(), text_color));
                                    fonts.layout_job(job).size().x
                                });

                                // ハイライト幅を計算（現在のセグメント内での進行度に応じて）
                                if current_time_ms < seg_start {
                                    highlight_width = prefix_width;
                                } else if let Some(end_ms) = seg_end {
                                    let elapsed = current_time_ms.saturating_sub(seg_start);
                                    let duration = end_ms.saturating_sub(seg_start).max(1);
                                    let ratio = (elapsed as f32 / duration as f32).clamp(0.0, 1.0);
                                    let segment_width = ui.fonts(|fonts| {
                                        let mut job = egui::text::LayoutJob::default();
                                        job.append(&segment.text, 0.0, egui::TextFormat::simple(font_id.clone(), text_color));
                                        fonts.layout_job(job).size().x
                                    });
                                    highlight_width = prefix_width + segment_width * ratio;
                                } else {
                                    highlight_width = full_text_width;
                                }
                                break;
                            }

                            // カスタム描画でハイライト付きテキストを表示
                            let text_size = egui::Vec2::new(full_text_width, ui.fonts(|fonts| fonts.row_height(&font_id)));
                            let (rect, response) = ui.allocate_exact_size(text_size, egui::Sense::hover());
                            let highlight_rect = egui::Rect::from_min_size(rect.min, egui::vec2(highlight_width, rect.height()));
                            ui.painter().rect_filled(highlight_rect, 0.0, highlight_color);
                            ui.painter().text(rect.min, egui::Align2::LEFT_TOP, &line.text, font_id.clone(), text_color);
                            response
                        } else {
                            // セグメント情報がない場合は単純なハイライト表示
                            ui.label(
                                egui::RichText::new(label_text)
                                    .color(egui::Color32::WHITE)
                                    .background_color(egui::Color32::from_rgb(40, 40, 80)),
                            )
                        }
                    } else {
                        // 現在の行でない場合は通常表示
                        ui.label(label_text)
                    };

                    // 現在の行が表示範囲内に入るように自動スクロール
                    if is_current_line {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                }
            });
    }
}