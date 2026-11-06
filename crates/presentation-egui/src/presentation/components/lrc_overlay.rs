//! Lyrics overlay drawn over the artwork.
//!
//! Layout work is cached in [`LrcLayoutCache`]: row splits (the expensive part, with prefix
//! measurements) and karaoke segment widths are computed once per (lyrics, width, font size),
//! and galleys are built lazily only for lines that are on screen and then reused. Lines are
//! drawn at two fixed font sizes (body / heading) and cross-faded instead of animating the
//! font size continuously, so egui never rasterizes a new glyph size per frame.

use std::sync::Arc;

use lrc_player_application::infrastructure::lyrics::Lyrics;
use eframe::egui;

const LINE_SWITCH_ANIMATION_MS: u64 = 250;
const HIGHLIGHT_START_DELAY_MS: u64 = 100;
const HIGHLIGHT_EARLY_FINISH_MS: u64 = LINE_SWITCH_ANIMATION_MS + HIGHLIGHT_START_DELAY_MS;
const TIMING_GAP_MS_PER_SPACE: u64 = 100;
const TIMING_GAP_MAX_SPACES: usize = 15;
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
    // Lines are sorted by time: O(log n).
    lyrics
        .lines
        .partition_point(|line| line.time_ms <= current_time_ms)
        .checked_sub(1)
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

fn timed_display_text(line: &lrc_player_application::infrastructure::lyrics::LyricLine) -> String {
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
    let total_width = ui.fonts_mut(|fonts| {
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
        if !is_break_whitespace(ch) {
            prev_char = Some(ch);
            continue;
        }

        let split_idx = boundary_split_index(text, idx, ch, prev_char);
        let prefix = &text[..split_idx];
        let prefix_width = ui.fonts_mut(|fonts| {
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
        let prefix_width = ui.fonts_mut(|fonts| {
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
        let width = ui.fonts_mut(|fonts| {
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
        let width = ui.fonts_mut(|fonts| {
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
    let total_width = ui.fonts_mut(|fonts| {
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
        if !is_break_whitespace(ch) {
            prev_char = Some(ch);
            continue;
        }
        let split_idx = boundary_split_index(text, idx, ch, prev_char);
        let prefix = &text[..split_idx];
        let width = ui.fonts_mut(|fonts| {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                prefix,
                0.0,
                egui::TextFormat::simple(font.clone(), egui::Color32::WHITE),
            );
            fonts.layout_job(job).size().x
        });
        whitespace_positions.push((split_idx, width));
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
            let remainder_width = ui.fonts_mut(|fonts| {
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

/// Karaoke highlight geometry of one line at the heading font size.
struct HighlightInfo {
    full_width: f32,
    segment_widths: Vec<f32>,
}

#[derive(Default)]
struct CachedLine {
    rows: Vec<String>,
    body: Vec<Option<Arc<egui::Galley>>>,
    heading: Vec<Option<Arc<egui::Galley>>>,
    highlight: Option<HighlightInfo>,
}

#[derive(Clone, Copy, PartialEq)]
struct LayoutKey {
    generation: u64,
    wrap_width: i32,
    body_px: i32,
    heading_px: i32,
    pixels_per_point_bits: u32,
}

/// Per-lyrics layout cache for [`render_lrc_overlay`]. Call [`LrcLayoutCache::invalidate`]
/// when different lyrics are loaded.
#[derive(Default)]
pub struct LrcLayoutCache {
    generation: u64,
    key: Option<LayoutKey>,
    lines: Vec<CachedLine>,
    last_atlas_fill: f32,
    /// Galleys built since the cache was (re)keyed (diagnostics / tests).
    galleys_built: usize,
}

impl LrcLayoutCache {
    /// Forget everything (new lyrics).
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.key = None;
        self.lines.clear();
    }

    pub fn galleys_built(&self) -> usize {
        self.galleys_built
    }

    fn prepare(&mut self, ui: &egui::Ui, lyrics: &Lyrics, key: LayoutKey, heading: &egui::FontId, wrap_width: f32) {
        // egui rebuilds its font atlas when it gets full; cached galleys then point at stale
        // glyph positions, so drop them (row splits stay valid).
        let fill = ui.ctx().fonts(|f| f.font_atlas_fill_ratio());
        let atlas_rebuilt = fill + 1e-4 < self.last_atlas_fill;
        self.last_atlas_fill = fill;
        if self.key == Some(key) && self.lines.len() == lyrics.lines.len() {
            if atlas_rebuilt {
                for line in &mut self.lines {
                    line.body.iter_mut().for_each(|g| *g = None);
                    line.heading.iter_mut().for_each(|g| *g = None);
                }
            }
            return;
        }
        self.key = Some(key);
        self.galleys_built = 0;
        self.lines = lyrics
            .lines
            .iter()
            .map(|line| {
                let display_text = timed_display_text(line);
                let rows = split_into_rows(ui, &display_text, heading.clone(), wrap_width);
                let n = rows.len();
                CachedLine {
                    rows,
                    body: vec![None; n],
                    heading: vec![None; n],
                    highlight: None,
                }
            })
            .collect();
    }

    fn galley(&mut self, ui: &egui::Ui, line: usize, row: usize, heading: bool, font: &egui::FontId, wrap_width: f32) -> Option<Arc<egui::Galley>> {
        let cached = self.lines.get_mut(line)?;
        let slot = if heading {
            cached.heading.get_mut(row)?
        } else {
            cached.body.get_mut(row)?
        };
        if slot.is_none() {
            let mut job = egui::text::LayoutJob::default();
            // Placeholder color: the real color (with animated alpha) is applied when painting.
            job.append(
                &cached.rows[row],
                0.0,
                egui::TextFormat::simple(font.clone(), egui::Color32::PLACEHOLDER),
            );
            job.wrap.max_width = wrap_width;
            job.break_on_newline = true;
            *slot = Some(ui.fonts_mut(|f| f.layout_job(job)));
            self.galleys_built += 1;
        }
        slot.clone()
    }

    fn highlight_info(&mut self, ui: &egui::Ui, lyrics: &Lyrics, line: usize, font: &egui::FontId) -> Option<&HighlightInfo> {
        let cached = self.lines.get_mut(line)?;
        if cached.highlight.is_none() {
            let lyric = lyrics.lines.get(line)?;
            let measure = |text: &str| {
                ui.fonts_mut(|fonts| {
                    let mut job = egui::text::LayoutJob::default();
                    job.append(text, 0.0, egui::TextFormat::simple(font.clone(), egui::Color32::WHITE));
                    fonts.layout_job(job).size().x
                })
            };
            let full_width = measure(&timed_display_text(lyric));
            let segment_widths = if lyric.segments.len() > 1 {
                lyric
                    .segments
                    .iter()
                    .enumerate()
                    .map(|(i, segment)| {
                        let gap = lyric
                            .segments
                            .get(i + 1)
                            .map(|next| timing_gap_spaces(next.time_ms.saturating_sub(segment.time_ms)))
                            .unwrap_or(0);
                        let mut text = segment.text.clone();
                        text.push_str(&scaled_gap_text(gap));
                        measure(&text)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            cached.highlight = Some(HighlightInfo {
                full_width,
                segment_widths,
            });
        }
        cached.highlight.as_ref()
    }
}

/// Highlight width (at the heading size) for `line` at lyric time `current_time_ms`.
fn highlight_width(line: &lrc_player_application::infrastructure::lyrics::LyricLine, info: &HighlightInfo, current_time_ms: u64) -> f32 {
    if line.segments.len() <= 1 || info.segment_widths.len() != line.segments.len() {
        return info.full_width;
    }
    let mut prefix = 0.0;
    for (i, segment) in line.segments.iter().enumerate() {
        let width = info.segment_widths[i];
        let end = line.segments.get(i + 1).map(|next| next.time_ms);
        if let Some(end) = end {
            if current_time_ms >= end {
                prefix += width;
                continue;
            }
        }
        if current_time_ms < segment.time_ms {
            return prefix.min(info.full_width);
        }
        return match end {
            Some(end) => {
                let elapsed = current_time_ms.saturating_sub(segment.time_ms);
                let duration = end.saturating_sub(segment.time_ms).max(1);
                let ratio = (elapsed as f32 / duration as f32).clamp(0.0, 1.0);
                (prefix + width * ratio).min(info.full_width)
            }
            None => info.full_width,
        };
    }
    prefix.min(info.full_width)
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// LRCオーバーレイを描画する関数
///
/// `current_time_ms` is the lyric-clock time (offsets already applied once by the caller).
pub fn render_lrc_overlay(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    lyrics: &Lyrics,
    current_time_ms: u64,
    cache: &mut LrcLayoutCache,
) {
    let highlighted_index = find_current_lyric_line_index(lyrics, current_time_ms);
    let animated_center_line = highlighted_index
        .map(|index| animated_center_line_index(lyrics, current_time_ms, index))
        .unwrap_or(0.0);

    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(150));

    // Two fixed, whole-pixel font sizes (no per-frame size animation).
    let body_px = (rect.height() * 0.04).round().max(6.0);
    let heading_px = (body_px * 1.75).round();
    let body_font = egui::FontId::proportional(body_px);
    let heading_font = egui::FontId::proportional(heading_px);
    let wrap_width = (rect.width() - 24.0).max(1.0).round();
    let key = LayoutKey {
        generation: cache.generation,
        wrap_width: wrap_width as i32,
        body_px: body_px as i32,
        heading_px: heading_px as i32,
        pixels_per_point_bits: ui.ctx().pixels_per_point().to_bits(),
    };
    cache.prepare(ui, lyrics, key, &heading_font, wrap_width);
    if cache.lines.is_empty() {
        return;
    }

    let (body_row_h, heading_row_h) = ui.fonts_mut(|f| (f.row_height(&body_font), f.row_height(&heading_font)));
    let normal_spacing = body_row_h * 0.2;
    let highlight_spacing = body_row_h;
    let emphasis_of = |line: usize| smoothstep(1.0 - (line as f32 - animated_center_line).abs());
    let row_height_of = |line: usize| body_row_h + (heading_row_h - body_row_h) * emphasis_of(line);
    let boundary_spacing = |left: usize, right: usize| {
        let t = emphasis_of(left).max(emphasis_of(right));
        normal_spacing + (highlight_spacing - normal_spacing) * t
    };

    // Group (line) tops relative to the first line: arithmetic only, no text layout.
    let line_count = cache.lines.len();
    let mut group_tops = Vec::with_capacity(line_count);
    let mut group_heights = Vec::with_capacity(line_count);
    let mut y = 0.0;
    for i in 0..line_count {
        let rows = cache.lines[i].rows.len().max(1);
        let h = row_height_of(i) * rows as f32 + normal_spacing * (rows - 1) as f32;
        group_tops.push(y);
        group_heights.push(h);
        y += h;
        if i + 1 < line_count {
            y += boundary_spacing(i, i + 1);
        }
    }
    let group_offset = |i: usize| rect.center().y - group_heights[i] / 2.0 - group_tops[i];
    let y_offset = match highlighted_index {
        Some(current) if current < line_count => {
            let base = group_offset(current);
            let target = if current + 1 < line_count { group_offset(current + 1) } else { base };
            let t = (animated_center_line - current as f32).clamp(0.0, 1.0);
            base + (target - base) * t
        }
        _ => 0.0,
    };

    let base_x = rect.min.x + 12.0;
    for line_index in 0..line_count {
        let top = y_offset + group_tops[line_index];
        let bottom = top + group_heights[line_index];
        if bottom < rect.min.y || top > rect.max.y {
            continue;
        }
        let emphasis = emphasis_of(line_index);
        let alpha = 170.0 + 85.0 * emphasis;
        let is_current = highlighted_index == Some(line_index);
        let is_next = highlighted_index.map(|i| i + 1) == Some(line_index);
        let row_count = cache.lines[line_index].rows.len();
        let row_h = row_height_of(line_index);

        let mut highlight_remaining = 0.0;
        if is_current {
            let adjusted = adjusted_highlight_time_ms(lyrics, line_index, current_time_ms);
            if let Some(info) = cache.highlight_info(ui, lyrics, line_index, &heading_font) {
                let scale = (body_px + (heading_px - body_px) * emphasis) / heading_px;
                highlight_remaining = highlight_width(&lyrics.lines[line_index], info, adjusted) * scale;
            }
        }

        for row in 0..row_count {
            let row_top = top + row as f32 * (row_h + normal_spacing);
            // Draw the body-size galley fading out and the heading-size galley fading in.
            let layers = [(false, 1.0 - emphasis), (true, emphasis)];
            for (is_heading, weight) in layers {
                if weight <= 0.01 {
                    continue;
                }
                let font = if is_heading { &heading_font } else { &body_font };
                let Some(galley) = cache.galley(ui, line_index, row, is_heading, font, wrap_width) else {
                    continue;
                };
                let size = galley.size();
                let center_x = base_x + (wrap_width - size.x) / 2.0;
                let align_right = (row_count == 2 && row == 1) || (row_count == 3 && row == 2);
                let align_center = row_count == 3 && row == 1;
                let target_x = if row == 0 {
                    base_x
                } else if align_center {
                    center_x
                } else if align_right {
                    base_x + wrap_width - size.x
                } else {
                    center_x
                };
                let mut x = center_x;
                if row_count > 1 && is_current {
                    x = if animated_center_line > line_index as f32 {
                        let ease = smoothstep(animated_center_line - line_index as f32);
                        target_x + (center_x - target_x) * ease
                    } else {
                        target_x
                    };
                } else if row_count > 1 && is_next {
                    x = center_x + (target_x - center_x) * emphasis;
                }
                // Vertically center the galley in the interpolated row box.
                let pos = egui::pos2(x, row_top + (row_h - size.y) / 2.0);

                if is_current && is_heading == (emphasis >= 0.5) && highlight_remaining > 0.0 {
                    for galley_row in &galley.rows {
                        if highlight_remaining <= 0.0 {
                            break;
                        }
                        let row_rect = galley_row.rect();
                        let w = highlight_remaining.min(row_rect.width());
                        if w > 0.0 {
                            painter.rect_filled(
                                egui::Rect::from_min_size(pos + row_rect.min.to_vec2(), egui::vec2(w, row_rect.height())),
                                0.0,
                                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 80),
                            );
                        }
                        highlight_remaining -= row_rect.width();
                    }
                }

                let color = egui::Color32::from_white_alpha((alpha * weight).round().clamp(0.0, 255.0) as u8);
                if is_current && is_heading {
                    for dx in [-0.6, 0.6] {
                        painter.galley(pos + egui::vec2(dx, 0.0), galley.clone(), color);
                    }
                }
                painter.galley(pos, galley, color);
            }
        }
    }
}
