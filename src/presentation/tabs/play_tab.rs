use crate::infrastructure::lyrics::Lyrics;
use crate::presentation::egui_app::GuiApp;
use eframe::egui;
use std::path::Path;
use std::time::{Duration, Instant};

/// 現在の再生時間に対応する歌詞の行インデックスを検索する関数
/// 
/// # 引数
/// * `lyrics` - 歌詞データ（LRCフォーマットをパースした構造体）
/// * `current_time_ms` - 現在の再生時間（ミリ秒単位）
/// 
/// # 戻り値
/// 現在の再生時間に対応する歌詞行のインデックス。見つからない場合はNone
/// 
/// # アルゴリズム
/// 歌詞行を後ろから検索し、現在時刻以下の最大のタイムスタンプを持つ行を見つける
/// これにより、曲の途中で一時停止した場合でも適切な行を表示できる
fn find_current_lyric_line_index(lyrics: &Lyrics, current_time_ms: u64) -> Option<usize> {
    lyrics
        .lines
        .iter()
        .enumerate()
        .rev() // 後ろから検索することで、current_time_ms以下の最大の時間を持つ行を見つける
        .find(|(_, line)| line.time_ms <= current_time_ms)
        .map(|(index, _)| index)
}

/// 歌詞行のハイライト表示幅を計算する関数
/// 歌詞のセグメント情報に基づいて、現在の再生位置に応じた進行状況を幅として返す
/// カラオケのような進行表示を実現するための補助関数
/// 
/// # 引数
/// * `ui` - eguiのUIビルダー（フォントレイアウト計算に必要）
/// * `line` - 歌詞行データ（テキストとタイミング情報を含む）
/// * `current_time_ms` - 現在の再生時間（ミリ秒）
/// * `font_id` - 使用するフォント（テキスト幅計算に使用）
/// 
/// # 戻り値
/// ハイライト表示する幅（ピクセル単位）
/// セグメントの進行度に応じて、部分的にハイライトされる
fn highlight_width_for_line(
    ui: &egui::Ui,
    line: &crate::infrastructure::lyrics::LyricLine,
    current_time_ms: u64,
    font_id: egui::FontId,
) -> f32 {
    let text_color = ui.style().visuals.text_color();
    
    // テキスト全体の幅を計算（ハイライト最大値の基準）
    let full_text_width = ui.fonts(|fonts| {
        let mut job = egui::text::LayoutJob::default();
        job.append(&line.text, 0.0, egui::TextFormat::simple(font_id.clone(), text_color));
        fonts.layout_job(job).size().x
    });

    // セグメントが1つ以下の場合は全体幅を返す（部分ハイライト不可）
    if line.segments.len() <= 1 {
        return full_text_width;
    }

    let mut prefix_text = String::new();
    let mut highlight_width = 0.0;
    
    // 各セグメントを順に処理
    for (seg_index, segment) in line.segments.iter().enumerate() {
        let seg_start = segment.time_ms;
        let seg_end = line.segments.get(seg_index + 1).map(|next| next.time_ms);

        // 現在時間がセグメント終了を過ぎている場合は、そのセグメントのテキストをprefixに追加
        // （既に完全に表示済みの部分）
        if let Some(end_ms) = seg_end {
            if current_time_ms >= end_ms {
                prefix_text.push_str(&segment.text);
                continue;
            }
        }

        // prefix部分の描画幅を計算（既に表示済みの固定部分）
        let prefix_width = ui.fonts(|fonts| {
            let mut job = egui::text::LayoutJob::default();
            job.append(&prefix_text, 0.0, egui::TextFormat::simple(font_id.clone(), text_color));
            fonts.layout_job(job).size().x
        });

        // 現在のセグメント内での進行度に応じてハイライト幅を計算
        if current_time_ms < seg_start {
            // まだセグメントが始まっていない場合
            highlight_width = prefix_width;
        } else if let Some(end_ms) = seg_end {
            // セグメント内の場合：経過時間の割合に応じて幅を計算
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
            // 最終セグメントの場合（終了時間が不明）は全体をハイライト
            highlight_width = full_text_width;
        }
        break;
    }

    // 全体幅を超えないように制限（丸め誤差対策）
    highlight_width.min(full_text_width)
}

fn is_ascii_or_symbol(c: char) -> bool {
    c.is_ascii() && !c.is_ascii_whitespace()
}

fn is_preferred_boundary(prev: char, next: char) -> bool {
    is_ascii_or_symbol(prev) != is_ascii_or_symbol(next)
}

fn boundary_split_index(text: &str, idx: usize, ch: char, prev_char: Option<char>) -> usize {
    if ch.is_whitespace() {
        let next_non_ws = text[idx..].chars().skip(1).find(|c| !c.is_whitespace());
        if let Some(next_char) = next_non_ws {
            if next_char.is_ascii_punctuation() {
                let next_idx = text[idx..]
                    .char_indices()
                    .skip(1)
                    .find(|(_, c)| !c.is_whitespace())
                    .map(|(off, _)| idx + off)
                    .unwrap_or(idx);
                return next_idx + next_char.len_utf8();
            }
        }
        return idx;
    }
    if let Some(prev) = prev_char {
        if is_preferred_boundary(prev, ch) && ch.is_ascii_punctuation() {
            return idx + ch.len_utf8();
        }
    }
    idx
}

fn split_into_two_rows(
    ui: &egui::Ui,
    text: &str,
    font: egui::FontId,
    max_width: f32,
) -> Vec<String> {
    let text_color = egui::Color32::WHITE;
    let total_width = ui.fonts(|fonts| {
        let mut job = egui::text::LayoutJob::default();
        job.append(text, 0.0, egui::TextFormat::simple(font.clone(), text_color));
        fonts.layout_job(job).size().x
    });

    if total_width <= max_width {
        return vec![text.to_string()];
    }

    let center_x = total_width / 2.0;
    let tolerance = total_width / 5.0;

    let mut best_whitespace_split: Option<usize> = None;
    let mut best_whitespace_dist = f32::MAX;
    let mut fallback_whitespace_split: Option<usize> = None;
    let mut fallback_whitespace_dist = f32::MAX;

    let mut prev_char: Option<char> = None;
    for (idx, ch) in text.char_indices() {
        let is_candidate = if ch.is_whitespace() {
            true
        } else if let Some(prev) = prev_char {
            is_preferred_boundary(prev, ch)
        } else {
            false
        };

        if !is_candidate {
            prev_char = Some(ch);
            continue;
        }

        let split_idx = boundary_split_index(text, idx, ch, prev_char);
        let prefix = &text[..split_idx];
        let prefix_width = ui.fonts(|fonts| {
            let mut job = egui::text::LayoutJob::default();
            job.append(prefix, 0.0, egui::TextFormat::simple(font.clone(), text_color));
            fonts.layout_job(job).size().x
        });
        if prefix_width > max_width {
            prev_char = Some(ch);
            continue;
        }

        let dist = (prefix_width - center_x).abs();
        if dist <= tolerance && dist < best_whitespace_dist {
            best_whitespace_dist = dist;
            best_whitespace_split = Some(split_idx);
        }
        if dist < fallback_whitespace_dist {
            fallback_whitespace_dist = dist;
            fallback_whitespace_split = Some(split_idx);
        }
        prev_char = Some(ch);
    }

    if let Some(split_idx) = best_whitespace_split.or(fallback_whitespace_split) {
        let prefix = text[..split_idx].trim_end().to_string();
        let remainder = if split_idx < text.len() && text[split_idx..].chars().next().map(|ch| ch.is_whitespace()).unwrap_or(false) {
            let ws_len = text[split_idx..].chars().next().map(|ch| ch.len_utf8()).unwrap_or(0);
            text[split_idx + ws_len..].trim_start().to_string()
        } else {
            text[split_idx..].to_string()
        };
        if remainder.is_empty() {
            return vec![prefix];
        }
        return vec![prefix, remainder];
    }

    // 空白がない場合は、中央付近の文字境界で分割する
    let mut best_split = None;
    let mut best_dist = f32::MAX;
    for (idx, ch) in text.char_indices() {
        let prefix = &text[..idx + ch.len_utf8()];
        let prefix_width = ui.fonts(|fonts| {
            let mut job = egui::text::LayoutJob::default();
            job.append(prefix, 0.0, egui::TextFormat::simple(font.clone(), text_color));
            fonts.layout_job(job).size().x
        });
        if prefix_width > max_width {
            break;
        }

        let dist = (prefix_width - center_x).abs();
        if dist < best_dist {
            best_dist = dist;
            best_split = Some(idx + ch.len_utf8());
        }
    }

    if let Some(split_idx) = best_split {
        let prefix = text[..split_idx].to_string();
        let remainder = text[split_idx..].trim_start().to_string();
        if remainder.is_empty() {
            return vec![prefix];
        }
        return vec![prefix, remainder];
    }

    // 最後の手段として最大幅まで切る
    let mut prefix_end = 0;
    for (idx, ch) in text.char_indices() {
        let candidate = &text[..idx + ch.len_utf8()];
        let width = ui.fonts(|fonts| {
            let mut job = egui::text::LayoutJob::default();
            job.append(candidate, 0.0, egui::TextFormat::simple(font.clone(), text_color));
            fonts.layout_job(job).size().x
        });
        if width > max_width {
            break;
        }
        prefix_end = idx + ch.len_utf8();
    }

    if prefix_end == 0 {
        return vec![text.to_string()];
    }

    let prefix = text[..prefix_end].trim_end().to_string();
    let remainder = text[prefix_end..].trim_start().to_string();
    if remainder.is_empty() {
        vec![prefix]
    } else {
        vec![prefix, remainder]
    }
}

fn find_split_at_char_width(
    ui: &egui::Ui,
    text: &str,
    font: egui::FontId,
    target_width: f32,
    max_width: f32,
) -> Option<usize> {
    let mut best_idx = None;
    let mut best_dist = f32::MAX;

    for (idx, ch) in text.char_indices() {
        let candidate = &text[..idx + ch.len_utf8()];
        let width = ui.fonts(|fonts| {
            let mut job = egui::text::LayoutJob::default();
            job.append(candidate, 0.0, egui::TextFormat::simple(font.clone(), egui::Color32::WHITE));
            fonts.layout_job(job).size().x
        });

        if width > max_width {
            break;
        }

        let dist = (width - target_width).abs();
        if dist < best_dist {
            best_dist = dist;
            best_idx = Some(idx + ch.len_utf8());
        }
    }

    best_idx
}

fn split_into_three_rows(
    ui: &egui::Ui,
    text: &str,
    font: egui::FontId,
    max_width: f32,
) -> Option<Vec<String>> {
    let total_width = ui.fonts(|fonts| {
        let mut job = egui::text::LayoutJob::default();
        job.append(text, 0.0, egui::TextFormat::simple(font.clone(), egui::Color32::WHITE));
        fonts.layout_job(job).size().x
    });

    if total_width <= max_width {
        return Some(vec![text.to_string()]);
    }

    let target_one = total_width / 3.0;
    let target_two = total_width * 2.0 / 3.0;

    let mut whitespace_positions: Vec<(usize, f32)> = Vec::new();
    let mut prev_char: Option<char> = None;
    for (idx, ch) in text.char_indices() {
        let is_boundary = if ch.is_whitespace() {
            true
        } else if let Some(prev) = prev_char {
            is_preferred_boundary(prev, ch)
        } else {
            false
        };
        if is_boundary {
            let split_idx = boundary_split_index(text, idx, ch, prev_char);
            let prefix = &text[..split_idx];
            let width = ui.fonts(|fonts| {
                let mut job = egui::text::LayoutJob::default();
                job.append(prefix, 0.0, egui::TextFormat::simple(font.clone(), egui::Color32::WHITE));
                fonts.layout_job(job).size().x
            });
            whitespace_positions.push((split_idx, width));
        }
        prev_char = Some(ch);
    }

    let mut best_combo: Option<(usize, usize)> = None;
    let mut best_error = f32::MAX;

    for (first_idx, first_width) in &whitespace_positions {
        if *first_width > max_width {
            continue;
        }

        for (second_idx, second_width) in &whitespace_positions {
            if *second_idx <= *first_idx {
                continue;
            }
            let first_segment_width = *first_width;
            let second_segment_width = *second_width - *first_width;
            let third_segment_width = total_width - *second_width;
            if first_segment_width > max_width
                || second_segment_width > max_width
                || third_segment_width > max_width
            {
                continue;
            }

            let error = (first_width - target_one).abs() + (second_width - target_two).abs();
            if error < best_error {
                best_error = error;
                best_combo = Some((*first_idx, *second_idx));
            }
        }
    }

    if let Some((first_idx, second_idx)) = best_combo {
        let first_len = first_idx;
        let second_len = second_idx;
        let first = text[..first_len].trim_end().to_string();
        let second = text[first_len..second_len].trim().to_string();
        let third = text[second_len..].trim_start().to_string();
        return Some(vec![first, second, third]);
    }

    let first_split = find_split_at_char_width(ui, text, font.clone(), target_one, max_width);
    if let Some(first_len) = first_split {
        let remainder = &text[first_len..];
        let second_split = find_split_at_char_width(
            ui,
            remainder,
            font.clone(),
            target_one,
            max_width,
        );
        if let Some(offset) = second_split {
            let second_len = first_len + offset;
            let first = text[..first_len].to_string();
            let second = text[first_len..second_len].trim().to_string();
            let third = text[second_len..].trim_start().to_string();
            return Some(vec![first, second, third]);
        }
    }

    None
}

fn split_into_rows(
    ui: &egui::Ui,
    text: &str,
    font: egui::FontId,
    max_width: f32,
) -> Vec<String> {
    let rows = split_into_two_rows(ui, text, font.clone(), max_width);
    if rows.len() != 2 {
        return rows;
    }

    let remainder = &rows[1];
    let remainder_width = ui.fonts(|fonts| {
        let mut job = egui::text::LayoutJob::default();
        job.append(remainder, 0.0, egui::TextFormat::simple(font.clone(), egui::Color32::WHITE));
        fonts.layout_job(job).size().x
    });

    if remainder_width <= max_width {
        return rows;
    }

    if let Some(three_rows) = split_into_three_rows(ui, text, font.clone(), max_width) {
        if three_rows.len() == 3 {
            return three_rows;
        }
    }

    rows
}

/// LRCオーバーレイを描画する関数
/// アルバムアートの上に歌詞を半透明のオーバーレイとして表示する
/// ミュージックビデオのような視覚効果を実現
/// 
/// # 引数
/// * `ui` - eguiのUIビルダー
/// * `rect` - 描画領域の矩形（通常はアルバムアート画像の領域）
/// * `lyrics` - 歌詞データ
/// * `current_time_ms` - 現在の再生時間（ミリ秒）
fn render_lrc_overlay(ui: &mut egui::Ui, rect: egui::Rect, lyrics: &Lyrics, current_time_ms: u64) {
    // 現在の歌詞行を特定
    let highlighted_index = find_current_lyric_line_index(lyrics, current_time_ms);
    let mut overlay_lines = Vec::new();

    // 表示する行を選択（現在行とその前後2行ずつ、合計最大5行）
    if let Some(index) = highlighted_index {
        let start = index.saturating_sub(2);
        let end = (index + 2).min(lyrics.lines.len() - 1);
        for i in start..=end {
            overlay_lines.push((lyrics.lines[i].clone(), i == index));
        }
    } else {
        // 現在行が見つからない場合は先頭から5行表示
        for line in lyrics.lines.iter().take(5) {
            overlay_lines.push((line.clone(), false));
        }
    }

    // 半透明の黒背景を描画（歌詞の可読性向上）
    let background = egui::Color32::from_black_alpha(150);
    ui.painter().rect_filled(rect, 0.0, background);

    // フォントサイズを計算（画面サイズに応じて動的に調整）
    let base_font_size = (rect.height() * 0.04).clamp(16.0, 44.0);
    let heading_font_size = (base_font_size * 2.0).clamp(18.0, 52.0); // 現在行は大きく
    let body_font_size = base_font_size;
    let wrap_width = rect.width() - 48.0; // 左右のパディングを確保
    let line_spacing = 6.0;
    let non_highlight_line_height = ui.ctx().fonts(|fonts| {
        let mut job = egui::text::LayoutJob::default();
        job.append(
            "あ",
            0.0,
            egui::TextFormat::simple(
                egui::FontId::proportional(body_font_size),
                egui::Color32::WHITE,
            ),
        );
        fonts.layout_job(job).size().y
    });
    let extra_spacing = non_highlight_line_height; // ハイライト行の前後に1行分のスペースを追加

    // 各行の Galley（テキストレイアウト情報）を先に作成してトータル高さを計算する
    // これにより中央揃えが正確になる
    let mut render_rows = Vec::new();
    for (line_index, (line, is_current)) in overlay_lines.into_iter().enumerate() {
        let font = if is_current {
            egui::FontId::proportional(heading_font_size)
        } else {
            egui::FontId::proportional(body_font_size)
        };
        let text_color = if is_current {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_white_alpha(210) // 非アクティブ行は少し透明
        };

        let row_texts = split_into_rows(ui, &line.text, font.clone(), wrap_width);
        let row_count = row_texts.len();
        for (row_index, row_text) in row_texts.into_iter().enumerate() {
            let mut job = egui::text::LayoutJob::default();
            job.append(&row_text, 0.0, egui::TextFormat::simple(font.clone(), text_color));
            job.wrap.max_width = wrap_width;
            job.break_on_newline = true;

            let galley = ui.ctx().fonts(|fonts| fonts.layout_job(job));
            let align_right = (row_count == 2 && row_index == 1) || (row_count == 3 && row_index == 2);
            let align_center = row_count == 3 && row_index == 1;
            render_rows.push((galley, line.clone(), is_current, font.clone(), align_right, align_center, line_index));
        }
    }

    // 行間スペースを含めた総高さを計算
    let mut total_height = 0.0;
    for i in 0..render_rows.len() {
        let galley_size = render_rows[i].0.size();
        total_height += galley_size.y;
        if i + 1 < render_rows.len() {
            let current_is_current = render_rows[i].2;
            let next_is_current = render_rows[i + 1].2;
            total_height += line_spacing + if current_is_current || next_is_current {
                extra_spacing
            } else {
                0.0
            };
        }
    }

    // 中央揃えで描画開始位置を計算
    let mut y = rect.center().y - total_height / 2.0;
    let mut current_highlight_line_index: Option<usize> = None;
    let mut current_line_remaining: f32 = 0.0;
    for i in 0..render_rows.len() {
        let (galley, line, is_current, font, align_right, align_center, line_index) = &render_rows[i];
        let galley_size = galley.size();
        let mut text_pos = egui::pos2(rect.min.x + 24.0, y);
        if *align_right {
            text_pos.x = rect.min.x + 24.0 + wrap_width - galley_size.x;
        } else if *align_center {
            text_pos.x = rect.min.x + 24.0 + (wrap_width - galley_size.x) / 2.0;
        }

        // 現在行の場合は、進行度に応じたハイライトを描画
        if *is_current {
            if current_highlight_line_index != Some(*line_index) {
                current_highlight_line_index = Some(*line_index);
                current_line_remaining = highlight_width_for_line(ui, &line, current_time_ms, font.clone());
            }

            let mut remaining = current_line_remaining;
            for row in &galley.rows {
                if remaining <= 0.0 {
                    break;
                }
                let row_width = row.rect.width();
                let current_width = remaining.min(row_width);
                if current_width > 0.0 {
                    let highlight_rect = egui::Rect::from_min_size(
                        text_pos + row.rect.min.to_vec2(),
                        egui::vec2(current_width, row.rect.height()),
                    );
                    ui.painter().rect_filled(
                        highlight_rect,
                        0.0,
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 80),
                    );
                }
                remaining -= row_width;
            }
            current_line_remaining = remaining;
        }

        // テキストを描画
        ui.painter().galley(text_pos, galley.clone());

        // 次の行のY位置を更新
        y += galley_size.y;
        if i + 1 < render_rows.len() {
            let current_is_current = render_rows[i].2;
            let next_is_current = render_rows[i + 1].2;
            y += line_spacing + if current_is_current || next_is_current {
                extra_spacing
            } else {
                0.0
            };
        }
    }
}

/// 再生画面のUIを描画する関数
/// アプリケーションのメイン再生インターフェース
/// 
/// # 引数
/// * `ui` - eguiのUIビルダー
/// * `app` - アプリケーションの状態を保持するGuiAppインスタンス
/// 
/// # 機能概要
/// - 再生コントロール（再生/一時停止、前の曲、次の曲）
/// - ファイルパス入力と再生開始
/// - メタデータ表示（曲名、アーティスト、アルバム、アルバムアート）
/// - 歌詞表示（LRCファイル対応）
/// - シークバーと再生位置表示
/// - 再生モード選択（シーケンシャル、リピート、シャッフル）
/// - LRCオーバーレイ表示の切り替え
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
            app.play_previous();  // 前の曲を再生（キューまたは履歴から）
        }
        
        // 再生状態に応じてボタンのラベルを動的に変更
        let play_label = match app.playback_state {
            crate::domain::model::PlaybackState::Playing => "Pause",   // 再生中なら「一時停止」
            crate::domain::model::PlaybackState::Paused => "Resume",   // 一時停止中なら「再開」
            _ => "Play",  // それ以外（停止中、バッファリング中など）は「再生」
        };
        if ui.button(play_label).clicked() {
            app.toggle_play_pause();  // 再生/一時停止を切り替え
        }
        
        if ui.button("Next").clicked() {
            app.play_next();  // 次の曲を再生（キューまたは次のファイル）
        }
    });

    // --- ファイルパス入力と操作ボタン ---
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.play_path);  // 再生するファイルのパス入力欄
        
        if ui.button("Play path").clicked() {
            app.play_position_ms = 0;  // 再生位置を先頭にリセット
            app.playback_state = crate::domain::model::PlaybackState::Buffering;  // バッファリング状態に設定
            app.discover_lyrics_for_current_play_path();  // 同じディレクトリの歌詞ファイルを探索
            app.request_play();  // 再生をリクエスト（別スレッドで処理）
        }
        
        if ui.button("Discover metadata").clicked() {
            app.refresh_play_metadata();  // メタデータ（曲名、アーティスト名など）を再読み込み
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
    
    // モードが変更されたら状態を永続化（設定ファイルに保存）
    if mode_changed {
        app.save_state();
    }

    // --- LRCオーバーレイ設定（アルバムアート上に歌詞を重ねる機能のON/OFF）---
    let mut lrc_overlay_changed = false;
    ui.horizontal(|ui| {
        lrc_overlay_changed = ui
            .checkbox(&mut app.lrc_overlay_enabled, "LRC overlay")
            .changed();
    });
    if lrc_overlay_changed {
        app.save_state();
    }

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
                // 歌詞ファイルを選択（ハイライト表示）
                if ui
                    .selectable_label(app.selected_lyrics_index == Some(index), label)
                    .clicked()
                {
                    app.load_lyrics_file(index);
                }
                // 「Load」ボタンで明示的に読み込み（選択と同時に読み込む設計の補助）
                if ui.button("Load").clicked() {
                    app.load_lyrics_file(index);
                }
            });
        }
    }

    // --- 現在再生中のトラック情報表示 ---
    if !app.play_path.trim().is_empty() {
        app.ensure_play_artwork(ui.ctx());  // アートワーク画像を読み込み（なければ読み込む）
        
        // アートワーク画像の表示（アルバムアート）
        // LRCオーバーレイが有効な場合はぼかし処理済みの画像を優先的に使用
        let display_texture = if app.lrc_overlay_enabled {
            app.play_artwork_blur_handle.as_ref().or(app.play_artwork_handle.as_ref())
        } else {
            app.play_artwork_handle.as_ref()
        };

        if let Some(texture) = display_texture {
            let available = ui.available_size();
            // アートワークの表示サイズを計算（画面サイズの80%を最大に、最小320px、最大1080px）
            let max_side = (available.x.min(available.y) * 0.9).clamp(320.0, 1080.0);
            let mut image_size = texture.size_vec2();
            let scale = (max_side / image_size.x).min(max_side / image_size.y);
            image_size *= scale;
            let response = ui.image(texture, image_size);

            // LRCオーバーレイが有効な場合、アートワークの上に歌詞を重ねて表示
            if app.lrc_overlay_enabled {
                let rect = response.rect;
                // 半透明の黒いオーバーレイを追加（歌詞の可読性向上）
                ui.painter().rect_filled(rect, 0.0, egui::Color32::from_black_alpha(120));
                if let Some(lyrics) = &app.loaded_lyrics {
                    // 歌詞オフセットを適用した現在時間を計算
                    let current_time_ms = if app.lyrics_offset_ms.is_negative() {
                        app.play_position_ms.saturating_sub((-app.lyrics_offset_ms) as u64)
                    } else {
                        app.play_position_ms.saturating_add(app.lyrics_offset_ms as u64)
                    };
                    render_lrc_overlay(ui, rect, lyrics, current_time_ms);
                }
            }

            ui.add_space(12.0);
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

            // 歌詞のオフセット調整スライダー（±30秒の範囲で調整可能）
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

            // 長押しによる連続シーク処理（矢印キーでのシークと同様の動作）
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
            // 再生パスと関連データをすべてクリア（現在の再生状態をリセット）
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
}