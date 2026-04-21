use crate::infrastructure::lyrics::Lyrics;
use eframe::egui;

const LINE_SWITCH_ANIMATION_MS: u64 = 250;
const HIGHLIGHT_START_DELAY_MS: u64 = 100;
const HIGHLIGHT_EARLY_FINISH_MS: u64 = LINE_SWITCH_ANIMATION_MS + HIGHLIGHT_START_DELAY_MS;
const TIMING_GAP_MS_PER_SPACE: u64 = 100;
const TIMING_GAP_MAX_SPACES: usize = 10;
const TIMING_GAP_SPACE_SCALE: f32 = 0.1;
const TIMING_GAP_SPACE_CHAR: char = '\u{2009}'; // thin space

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

fn animated_center_line_index(lyrics: &Lyrics, current_time_ms: u64, current_index: usize) -> f32 {
    let Some(next_line) = lyrics.lines.get(current_index + 1) else {
        return current_index as f32;
    };

    let switch_start = next_line.time_ms.saturating_sub(LINE_SWITCH_ANIMATION_MS);
    if current_time_ms <= switch_start {
        return current_index as f32;
    }

    let span = next_line.time_ms.saturating_sub(switch_start).max(1);
    let elapsed = current_time_ms.saturating_sub(switch_start).min(span);
    let ratio = elapsed as f32 / span as f32;
    current_index as f32 + ratio
}

fn adjusted_highlight_time_ms(lyrics: &Lyrics, line_index: usize, current_time_ms: u64) -> u64 {
    let Some(line) = lyrics.lines.get(line_index) else {
        return current_time_ms;
    };
    let Some(next_line) = lyrics.lines.get(line_index + 1) else {
        return current_time_ms;
    };

    let source_start = line.time_ms;
    let source_end = next_line.time_ms;
    if source_end <= source_start {
        return current_time_ms;
    }

    // 切替後100msはハイライトを開始せず、次行切替の300ms前までにハイライトが完了するよう圧縮する。
    let effective_start = source_start.saturating_add(HIGHLIGHT_START_DELAY_MS);
    let target_end = source_end.saturating_sub(HIGHLIGHT_EARLY_FINISH_MS);
    if target_end <= effective_start {
        return source_end;
    }
    if current_time_ms <= source_start {
        return current_time_ms;
    }
    if current_time_ms <= effective_start {
        return source_start;
    }
    if current_time_ms >= target_end {
        return source_end;
    }

    let elapsed = current_time_ms.saturating_sub(effective_start);
    let target_span = target_end.saturating_sub(effective_start).max(1);
    let source_span = source_end.saturating_sub(source_start);
    source_start + elapsed.saturating_mul(source_span) / target_span
}

fn timing_gap_spaces(delta_ms: u64) -> usize {
    if delta_ms < TIMING_GAP_MS_PER_SPACE {
        0
    } else {
        (delta_ms / TIMING_GAP_MS_PER_SPACE).min(TIMING_GAP_MAX_SPACES as u64) as usize
    }
}

fn scaled_gap_text(gap_spaces: usize) -> String {
    if gap_spaces == 0 {
        return String::new();
    }

    let scaled = ((gap_spaces as f32) * TIMING_GAP_SPACE_SCALE)
        .round()
        .max(1.0) as usize;
    TIMING_GAP_SPACE_CHAR.to_string().repeat(scaled)
}

fn timed_display_text(line: &crate::infrastructure::lyrics::LyricLine) -> String {
    if line.segments.len() <= 1 {
        return line.text.clone();
    }

    let mut text = String::new();
    for (idx, segment) in line.segments.iter().enumerate() {
        text.push_str(&segment.text);
        if let Some(next) = line.segments.get(idx + 1) {
            let delta_ms = next.time_ms.saturating_sub(segment.time_ms);
            let spaces = timing_gap_spaces(delta_ms);
            if spaces > 0 {
                text.push_str(&scaled_gap_text(spaces));
            }
        }
    }
    text
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
    let display_text = timed_display_text(line);

    // テキスト全体の幅を計算（ハイライト最大値の基準）
    let full_text_width = ui.fonts(|fonts| {
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &display_text,
            0.0,
            egui::TextFormat::simple(font_id.clone(), text_color),
        );
        fonts.layout_job(job).size().x
    });

    // セグメントが1つ以下の場合は全体幅を返す（部分ハイライト不可）
    if line.segments.len() <= 1 {
        return full_text_width;
    }

    let mut segment_end_times: Vec<Option<u64>> = Vec::with_capacity(line.segments.len());
    let mut segment_widths: Vec<f32> = Vec::with_capacity(line.segments.len());

    for (seg_index, segment) in line.segments.iter().enumerate() {
        let next_segment = line.segments.get(seg_index + 1);
        let seg_end = next_segment.map(|next| next.time_ms);
        let gap_spaces = next_segment
            .map(|next| timing_gap_spaces(next.time_ms.saturating_sub(segment.time_ms)))
            .unwrap_or(0);

        let mut timed_segment_text = segment.text.clone();
        if gap_spaces > 0 {
            timed_segment_text.push_str(&scaled_gap_text(gap_spaces));
        }

        let width = ui.fonts(|fonts| {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                &timed_segment_text,
                0.0,
                egui::TextFormat::simple(font_id.clone(), text_color),
            );
            fonts.layout_job(job).size().x
        });

        segment_end_times.push(seg_end);
        segment_widths.push(width);
    }

    let mut prefix_width = 0.0;
    let mut highlight_width = 0.0;

    for (seg_index, segment) in line.segments.iter().enumerate() {
        let seg_start = segment.time_ms;
        let seg_end = segment_end_times[seg_index];
        let segment_width = segment_widths[seg_index];

        if let Some(end_ms) = seg_end {
            if current_time_ms >= end_ms {
                prefix_width += segment_width;
                continue;
            }
        }

        if current_time_ms < seg_start {
            highlight_width = prefix_width;
        } else if let Some(end_ms) = seg_end {
            let elapsed = current_time_ms.saturating_sub(seg_start);
            let duration = end_ms.saturating_sub(seg_start).max(1);
            let ratio = (elapsed as f32 / duration as f32).clamp(0.0, 1.0);
            highlight_width = prefix_width + segment_width * ratio;
        } else {
            highlight_width = full_text_width;
        }
        break;
    }

    highlight_width.min(full_text_width)
}

fn is_ascii_or_symbol(c: char) -> bool {
    c.is_ascii() && !c.is_ascii_whitespace()
}

fn is_timing_gap_whitespace(c: char) -> bool {
    c == TIMING_GAP_SPACE_CHAR
}

fn is_break_whitespace(c: char) -> bool {
    c.is_whitespace() && !is_timing_gap_whitespace(c)
}

fn is_preferred_boundary(prev: char, next: char) -> bool {
    is_ascii_or_symbol(prev) != is_ascii_or_symbol(next)
}

fn boundary_split_index(text: &str, idx: usize, ch: char, prev_char: Option<char>) -> usize {
    if is_break_whitespace(ch) {
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
        job.append(
            text,
            0.0,
            egui::TextFormat::simple(font.clone(), text_color),
        );
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
        let is_candidate = if is_break_whitespace(ch) {
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
            job.append(
                prefix,
                0.0,
                egui::TextFormat::simple(font.clone(), text_color),
            );
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
        let remainder = if split_idx < text.len()
            && text[split_idx..]
                .chars()
                .next()
                .map(|ch| ch.is_whitespace())
                .unwrap_or(false)
        {
            let ws_len = text[split_idx..]
                .chars()
                .next()
                .map(|ch| ch.len_utf8())
                .unwrap_or(0);
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
            job.append(
                prefix,
                0.0,
                egui::TextFormat::simple(font.clone(), text_color),
            );
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
            job.append(
                candidate,
                0.0,
                egui::TextFormat::simple(font.clone(), text_color),
            );
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
            job.append(
                candidate,
                0.0,
                egui::TextFormat::simple(font.clone(), egui::Color32::WHITE),
            );
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
        job.append(
            text,
            0.0,
            egui::TextFormat::simple(font.clone(), egui::Color32::WHITE),
        );
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
        let is_boundary = if is_break_whitespace(ch) {
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
                job.append(
                    prefix,
                    0.0,
                    egui::TextFormat::simple(font.clone(), egui::Color32::WHITE),
                );
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
        let second_split =
            find_split_at_char_width(ui, remainder, font.clone(), target_one, max_width);
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

fn split_into_rows(ui: &egui::Ui, text: &str, font: egui::FontId, max_width: f32) -> Vec<String> {
    let mut rows = Vec::new();
    let normalized_text = text.replace("\r\n", "\n").replace('\r', "");

    for paragraph in normalized_text.split('\n') {
        if paragraph.is_empty() {
            rows.push(String::new());
            continue;
        }

        let paragraph_rows = split_into_two_rows(ui, paragraph, font.clone(), max_width);
        if paragraph_rows.len() == 2 {
            let remainder = &paragraph_rows[1];
            let remainder_width = ui.fonts(|fonts| {
                let mut job = egui::text::LayoutJob::default();
                job.append(
                    remainder,
                    0.0,
                    egui::TextFormat::simple(font.clone(), egui::Color32::WHITE),
                );
                fonts.layout_job(job).size().x
            });

            if remainder_width <= max_width {
                rows.extend(paragraph_rows);
            } else if let Some(three_rows) = split_into_three_rows(ui, paragraph, font.clone(), max_width) {
                if three_rows.len() == 3 {
                    rows.extend(three_rows);
                } else {
                    rows.extend(paragraph_rows);
                }
            } else {
                rows.extend(paragraph_rows);
            }
        } else {
            rows.extend(paragraph_rows);
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
pub fn render_lrc_overlay(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    lyrics: &Lyrics,
    current_time_ms: u64,
) {
    let highlighted_index = find_current_lyric_line_index(lyrics, current_time_ms);
    let animated_center_line = highlighted_index
        .map(|index| animated_center_line_index(lyrics, current_time_ms, index))
        .unwrap_or(0.0);
    let overlay_lines: Vec<_> = lyrics
        .lines
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, line)| (line, index))
        .collect();

    // 半透明の黒背景を描画（歌詞の可読性向上）
    let background = egui::Color32::from_black_alpha(150);
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, background);

    // フォントサイズを計算（画面サイズに応じて動的に調整）
    let base_font_size = rect.height() * 0.04;
    let heading_font_size = base_font_size * 1.75; // 現在行は大きく
    let body_font_size = base_font_size;
    let wrap_width = rect.width() - 24.0; // 左右のパディングを確保
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
    let normal_spacing = non_highlight_line_height * 0.2; // 通常行の前後に0.2行分のスペースを追加
    let highlight_spacing = non_highlight_line_height; // ハイライト行前後に1行分のスペースを追加

    // 各行の Galley（テキストレイアウト情報）を先に作成してトータル高さを計算する
    // これにより中央揃えが正確になる
    let mut render_rows = Vec::new();
    for (line, absolute_index) in overlay_lines {
        let distance = (absolute_index as f32 - animated_center_line).abs();
        let emphasis = (1.0 - distance).clamp(0.0, 1.0);
        let emphasis = emphasis * emphasis * (3.0 - 2.0 * emphasis); // smoothstep
        let font_size = body_font_size + (heading_font_size - body_font_size) * emphasis;
        let alpha = (170.0 + 85.0 * emphasis).round() as u8;
        let font = egui::FontId::proportional(font_size);
        let text_color = egui::Color32::from_white_alpha(alpha);

        let display_text = timed_display_text(&line);
        // 行の分割はハイライト時に決定し、非ハイライトでも同じ分割を使う。
        let split_font = egui::FontId::proportional(heading_font_size);
        let row_texts = split_into_rows(ui, &display_text, split_font, wrap_width);
        let row_count = row_texts.len();
        let is_current_line = highlighted_index == Some(absolute_index);
        for (row_index, row_text) in row_texts.into_iter().enumerate() {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                &row_text,
                0.0,
                egui::TextFormat::simple(font.clone(), text_color),
            );
            job.wrap.max_width = wrap_width;
            job.break_on_newline = true;

            let galley = ui.ctx().fonts(|fonts| fonts.layout_job(job));
            let align_right =
                (row_count == 2 && row_index == 1) || (row_count == 3 && row_index == 2);
            let align_center = row_count == 3 && row_index == 1;
            render_rows.push((
                galley,
                line.clone(),
                is_current_line,
                font.clone(),
                align_right,
                align_center,
                absolute_index,
                row_index,
                row_count,
            ));
        }
    }

    // 行間は、中央付近に来る行ほど normal -> highlight へ滑らかに拡大する。
    let boundary_spacing = |left_line_index: usize, right_line_index: usize| {
        if left_line_index == right_line_index {
            return normal_spacing;
        }

        let left_dist = (left_line_index as f32 - animated_center_line).abs();
        let right_dist = (right_line_index as f32 - animated_center_line).abs();
        let left_emphasis = (1.0 - left_dist).clamp(0.0, 1.0);
        let right_emphasis = (1.0 - right_dist).clamp(0.0, 1.0);
        let t = left_emphasis.max(right_emphasis);
        let eased_t = t * t * (3.0 - 2.0 * t);

        normal_spacing + (highlight_spacing - normal_spacing) * eased_t
    };

    let mut row_tops = Vec::with_capacity(render_rows.len());
    let mut current_row_y = 0.0;
    for i in 0..render_rows.len() {
        row_tops.push(current_row_y);
        let galley_size = render_rows[i].0.size();
        current_row_y += galley_size.y;
        if i + 1 < render_rows.len() {
            let current_line_index = render_rows[i].6;
            let next_line_index = render_rows[i + 1].6;
            current_row_y += boundary_spacing(current_line_index, next_line_index);
        }
    }

    let mut group_positions = Vec::new();
    let mut group_start = 0;
    while group_start < render_rows.len() {
        let line_index = render_rows[group_start].6;
        let group_top = row_tops[group_start];
        let mut group_height = 0.0;
        let mut idx = group_start;
        while idx < render_rows.len() && render_rows[idx].6 == line_index {
            group_height += render_rows[idx].0.size().y;
            if idx + 1 < render_rows.len() && render_rows[idx + 1].6 == line_index {
                group_height += normal_spacing;
            }
            idx += 1;
        }
        group_positions.push((line_index, group_top, group_height));
        group_start = idx;
    }

    let current_group_offset = highlighted_index.and_then(|current_index| {
        group_positions
            .iter()
            .find(|(line_index, _, _)| *line_index == current_index)
            .map(|(_, top, height)| rect.center().y - height / 2.0 - top)
    });
    let next_group_offset = highlighted_index.and_then(|current_index| {
        group_positions
            .iter()
            .find(|(line_index, _, _)| *line_index == current_index + 1)
            .map(|(_, top, height)| rect.center().y - height / 2.0 - top)
    });

    let y_offset = if let Some(current_index) = highlighted_index {
        let base_offset = current_group_offset.unwrap_or(0.0);
        let target_offset = next_group_offset.unwrap_or(base_offset);
        let t = (animated_center_line - current_index as f32).clamp(0.0, 1.0);
        base_offset + (target_offset - base_offset) * t
    } else {
        0.0
    };

    let mut current_highlight_line_index: Option<usize> = None;
    let mut current_line_remaining: f32 = 0.0;
    for i in 0..render_rows.len() {
        let (
            galley,
            line,
            is_current,
            font,
            align_right,
            align_center,
            line_index,
            row_index,
            row_count,
        ) = &render_rows[i];
        let galley_size = galley.size();
        let base_x = rect.min.x + 12.0;
        let center_x = rect.min.x + 12.0 + (wrap_width - galley_size.x) / 2.0;
        let mut text_pos = egui::pos2(center_x, y_offset + row_tops[i]);
        let is_next = highlighted_index.map(|idx| idx + 1) == Some(*line_index);
        if *is_current && *row_count > 1 {
            let target_x = if *row_index == 0 {
                base_x
            } else if *align_center {
                center_x
            } else if *align_right {
                rect.min.x + 12.0 + wrap_width - galley_size.x
            } else {
                center_x
            };
            if animated_center_line > (*line_index) as f32 {
                let progress = (animated_center_line - (*line_index) as f32).clamp(0.0, 1.0);
                let ease = progress * progress * (3.0 - 2.0 * progress);
                text_pos.x = target_x + (center_x - target_x) * ease;
            } else {
                text_pos.x = target_x;
            }
        } else if is_next && *row_count > 1 {
            let target_x = if *row_index == 0 {
                base_x
            } else if *align_center {
                center_x
            } else if *align_right {
                rect.min.x + 12.0 + wrap_width - galley_size.x
            } else {
                center_x
            };
            let start_x = center_x;
            let distance = (*line_index as f32 - animated_center_line).abs();
            let emphasis = (1.0_f32 - distance).clamp(0.0, 1.0);
            let ease = emphasis * emphasis * (3.0 - 2.0 * emphasis);
            text_pos.x = start_x + (target_x - start_x) * ease;
        }

        let row_y = y_offset + row_tops[i];
        if row_y + galley_size.y < rect.min.y || row_y > rect.max.y {
            continue;
        }

        // 現在行の場合は、進行度に応じたハイライトを描画
        if *is_current {
            if current_highlight_line_index != Some(*line_index) {
                current_highlight_line_index = Some(*line_index);
                let adjusted_time_ms =
                    adjusted_highlight_time_ms(lyrics, *line_index, current_time_ms);
                current_line_remaining =
                    highlight_width_for_line(ui, line, adjusted_time_ms, font.clone());
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
                    painter.rect_filled(
                        highlight_rect,
                        0.0,
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 80),
                    );
                }
                remaining -= row_width;
            }
            current_line_remaining = remaining;
        }

        // 太字らしい見た目にするため、現在行は複製描画で重みを付ける
        if *is_current {
            let bold_offsets = [egui::vec2(-0.6, 0.0), egui::vec2(0.6, 0.0)];
            for offset in bold_offsets {
                painter.galley(text_pos + offset, galley.clone());
            }
        }
        painter.galley(text_pos, galley.clone());
    }
}
