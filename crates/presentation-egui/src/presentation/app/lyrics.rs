//! 歌詞（LRC）ファイルの管理を担当
//! - 歌詞候補（ワーカーで 1 回だけ解析・ランク付け済み）の選択
//! - オフセット（ユーザー調整 + `[offset:]` タグ）は `LyricsTimeline` が 1 回だけ適用
//! - オフセットの LRC への保存（アトミック置換 + `.bak`）
//! - 歌詞テキストの行ずらし / 文字ずらし（タイムスタンプは固定、曲ごとに永続化）
//! - ドロップされた `.lrc` を再生中の曲に割り当てる（永続化）

use std::path::Path;

use lrc_player_application::dto::LyricsShiftDto;
use lrc_player_application::infrastructure::lyrics::{
    replace_file_with_backup, save_lrc_offset, Lyrics, LyricsCandidate, LyricsTimeline,
};

use crate::presentation::app::GuiApp;
use crate::presentation::ui::views::LyricLine;

/// Largest user offset accepted from the UI.
pub(crate) const MAX_LYRICS_OFFSET_MS: i64 = 30_000;

/// Lyric-clock time at playback `position_ms`.
///
/// The timeline applies the file's `[offset:]` tag plus the user offset exactly once; callers
/// must not add `lyrics_offset_ms` themselves.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn overlay_time_ms(timeline: Option<&LyricsTimeline>, position_ms: u64) -> u64 {
    match timeline {
        Some(t) => t.lyric_time_ms(position_ms).max(0) as u64,
        None => position_ms,
    }
}

/// Lines for the lyrics view, borrowed from the timeline.
pub(crate) fn lyric_lines(timeline: &LyricsTimeline, karaoke: bool) -> Vec<LyricLine<'_>> {
    timeline
        .lines()
        .iter()
        .map(|l| LyricLine { text: &l.text, start_ms: l.start_ms, karaoke: karaoke && l.is_karaoke() })
        .collect()
}

impl GuiApp {
    /// Installs ranked candidates from the worker and loads the best one. A `.lrc` attached
    /// by drag & drop for this track takes precedence.
    pub(crate) fn set_lyrics_candidates(&mut self, mut candidates: Vec<LyricsCandidate>) {
        if let Some(over) = self.lyrics_overrides.get(&self.play_path) {
            let over = Path::new(over);
            if let Some(pos) = candidates.iter().position(|c| c.path == over) {
                let c = candidates.remove(pos);
                candidates.insert(0, c);
            } else if over.is_file() {
                let mut ranked = Lyrics::rank_candidates([over.to_path_buf()]);
                if let Some(c) = ranked.pop() {
                    candidates.insert(0, c);
                }
            }
        }
        self.lyrics_candidates = candidates;
        self.selected_lyrics_index = None;
        self.loaded_timeline = None;
        self.lyrics_revision += 1;
        if !self.lyrics_candidates.is_empty() {
            self.load_lyrics_file(0);
        }
    }

    /// Selects candidate `index` (already parsed; no file I/O).
    pub(crate) fn load_lyrics_file(&mut self, index: usize) {
        let Some(candidate) = self.lyrics_candidates.get(index) else {
            return;
        };
        match &candidate.timeline {
            Some(timeline) => {
                let shift = self.lyrics_shift;
                let mut timeline = if shift.is_zero() {
                    timeline.clone()
                } else {
                    timeline.with_text_shift(shift.lines, shift.chars)
                };
                timeline.set_user_offset_ms(self.lyrics_offset_ms);
                self.loaded_timeline = Some(timeline);
                self.selected_lyrics_index = Some(index);
                self.lyrics_revision += 1;
            }
            None => {
                let message = candidate
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("{}", candidate.path.display()));
                tracing::warn!("{message}");
                let tr = self.tr();
                self.toast_warning(tr.f("toast.lyrics_load_failed", &[&message]));
            }
        }
    }

    /// Attaches a dropped `.lrc` to the current track (remembered per track).
    pub(crate) fn attach_lrc(&mut self, lrc: &Path) {
        let tr = self.tr();
        if self.play_path.is_empty() {
            self.toast_warning(tr.t("toast.lrc_no_track").to_string());
            return;
        }
        let mut ranked = Lyrics::rank_candidates([lrc.to_path_buf()]);
        let Some(candidate) = ranked.pop() else { return };
        if candidate.timeline.as_ref().is_none_or(LyricsTimeline::is_empty) {
            let err = candidate.error.unwrap_or_else(|| tr.t("toast.lrc_empty").to_string());
            self.toast_error(tr.f("toast.lyrics_load_failed", &[&err]));
            return;
        }
        self.lyrics_overrides
            .insert(self.play_path.clone(), lrc.to_string_lossy().into_owned());
        self.lyrics_candidates.retain(|c| c.path != lrc);
        self.lyrics_candidates.insert(0, candidate);
        self.load_lyrics_file(0);
        self.save_state();
        let name = lrc.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.toast_success(tr.f("toast.lrc_attached", &[&name]));
    }

    /// Sets the user lyrics offset for the current track (positive = lyrics earlier).
    pub(crate) fn set_lyrics_offset(&mut self, offset_ms: i64) {
        let offset_ms = offset_ms.clamp(-MAX_LYRICS_OFFSET_MS, MAX_LYRICS_OFFSET_MS);
        self.lyrics_offset_ms = offset_ms;
        if let Some(timeline) = &mut self.loaded_timeline {
            timeline.set_user_offset_ms(offset_ms);
        }
        if !self.play_path.is_empty() {
            if offset_ms == 0 {
                self.lyrics_offsets.remove(&self.play_path);
            } else {
                self.lyrics_offsets.insert(self.play_path.clone(), offset_ms);
            }
        }
        self.save_state();
    }

    /// Sets the lyric text shift for the current track (clamped to what the lyrics allow) and
    /// rebuilds the displayed timeline from the original.
    pub(crate) fn set_lyrics_shift(&mut self, lines: i32, chars: i32) {
        let base = self
            .selected_lyrics_index
            .and_then(|i| self.lyrics_candidates.get(i))
            .and_then(|c| c.timeline.as_ref());
        let (max_lines, max_chars) = base.map_or((0, 0), |t| {
            (t.len().saturating_sub(1) as i32, t.karaoke_char_count().saturating_sub(1) as i32)
        });
        let shift = LyricsShiftDto { lines: lines.clamp(-max_lines, max_lines), chars: chars.clamp(-max_chars, max_chars) };
        if shift == self.lyrics_shift {
            return;
        }
        self.lyrics_shift = shift;
        if !self.play_path.is_empty() {
            if shift.is_zero() {
                self.lyrics_shifts.remove(&self.play_path);
            } else {
                self.lyrics_shifts.insert(self.play_path.clone(), shift);
            }
        }
        if let Some(index) = self.selected_lyrics_index {
            self.load_lyrics_file(index);
        }
        self.save_state();
    }

    /// Writes the displayed lyrics (text shift and offset applied) to the selected `.lrc`
    /// (atomic replace, previous file kept as `.bak`), then resets shift and offset to 0.
    fn save_lyrics_shift_to_selected_file(&mut self, path: std::path::PathBuf, index: usize) {
        let tr = self.tr();
        let Some(timeline) = &self.loaded_timeline else { return };
        let contents = timeline.to_lrc_string();
        if let Err(err) = replace_file_with_backup(&path, contents.as_bytes()) {
            self.toast_error(tr.f("toast.offset_save_failed", &[&format!("{err:#}")]));
            return;
        }
        let reloaded = LyricsTimeline::from_lrc(&path);
        self.lyrics_shift = LyricsShiftDto::default();
        self.lyrics_shifts.remove(&self.play_path);
        self.set_lyrics_offset(0);
        if let Ok(timeline) = reloaded {
            if let Some(candidate) = self.lyrics_candidates.get_mut(index) {
                candidate.timeline = Some(timeline);
                candidate.error = None;
            }
            self.load_lyrics_file(index);
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.toast_success(tr.f("toast.lyrics_saved", &[&name]));
    }

    /// Bakes the current offset (plus any `[offset:]` tag) into the selected `.lrc`
    /// (atomic replace, previous file kept as `.bak`) and resets the user offset to 0.
    /// With a text shift, the whole file is rewritten from the shifted lyrics instead.
    pub(crate) fn save_lyrics_offset_to_selected_file(&mut self) {
        let tr = self.tr();
        let Some(path) = self
            .selected_lyrics_index
            .and_then(|i| self.lyrics_candidates.get(i))
            .map(|c| c.path.clone())
        else {
            self.toast_warning(tr.t("toast.no_lyrics_file").to_string());
            return;
        };
        let index = self.selected_lyrics_index.unwrap_or(0);
        if !self.lyrics_shift.is_zero() {
            self.save_lyrics_shift_to_selected_file(path, index);
            return;
        }

        let saved = match save_lrc_offset(&path, self.lyrics_offset_ms) {
            Ok(saved) => saved,
            Err(err) => {
                self.toast_error(tr.f("toast.offset_save_failed", &[&format!("{err:#}")]));
                return;
            }
        };
        if saved.backup_path.is_none() {
            self.toast_info(tr.t("toast.offset_zero").to_string());
            return;
        }

        // Reload the rewritten file (single small file) with a zero user offset.
        let reloaded = LyricsTimeline::from_lrc(&path);
        self.set_lyrics_offset(0);
        if let Ok(timeline) = reloaded {
            if let Some(candidate) = self.lyrics_candidates.get_mut(index) {
                candidate.timeline = Some(timeline);
                candidate.error = None;
            }
            self.load_lyrics_file(index);
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.toast_success(tr.f("toast.offset_saved", &[&name, &saved.shifted_tags]));
    }
}

#[cfg(test)]
#[path = "tests/test_lyrics_offset.rs"]
mod tests;
