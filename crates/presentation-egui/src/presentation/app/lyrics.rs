//! 歌詞（LRC）ファイルの管理を担当
//! - 歌詞候補（ワーカーで 1 回だけ解析・ランク付け済み）の選択
//! - オフセット（ユーザー調整 + `[offset:]` タグ）は `LyricsTimeline` が 1 回だけ適用
//! - オフセットの LRC への保存（アトミック置換 + `.bak`）

use lrc_player_application::infrastructure::lyrics::{
    save_lrc_offset, Lyrics, LyricsCandidate, LyricsTimeline,
};

use crate::presentation::app::GuiApp;

/// Largest user offset accepted from the UI.
pub(crate) const MAX_LYRICS_OFFSET_MS: i64 = 30_000;

/// Lyric-clock time for the overlay at playback `position_ms`.
///
/// The timeline applies the file's `[offset:]` tag plus the user offset exactly once; callers
/// must not add `lyrics_offset_ms` themselves.
pub(crate) fn overlay_time_ms(timeline: Option<&LyricsTimeline>, position_ms: u64) -> u64 {
    match timeline {
        Some(t) => t.lyric_time_ms(position_ms).max(0) as u64,
        None => position_ms,
    }
}

impl GuiApp {
    /// Installs ranked candidates from the worker and loads the best one.
    pub(crate) fn set_lyrics_candidates(&mut self, candidates: Vec<LyricsCandidate>) {
        self.lyrics_files = candidates.iter().map(|c| c.path.clone()).collect();
        self.lyrics_candidates = candidates;
        self.selected_lyrics_index = None;
        self.loaded_lyrics = None;
        self.loaded_timeline = None;
        self.lrc_layout.invalidate();
        if !self.lyrics_candidates.is_empty() {
            self.load_lyrics_file(0);
        }
    }

    /// Selects candidate `index` (already parsed; no file I/O).
    pub(crate) fn load_lyrics_file(&mut self, index: usize) {
        let Some(candidate) = self.lyrics_candidates.get(index) else {
            return;
        };
        match (&candidate.lyrics, &candidate.timeline) {
            (Some(lyrics), Some(timeline)) => {
                let mut timeline = timeline.clone();
                timeline.set_user_offset_ms(self.lyrics_offset_ms);
                self.loaded_lyrics = Some(lyrics.clone());
                self.loaded_timeline = Some(timeline);
                self.selected_lyrics_index = Some(index);
                self.lrc_layout.invalidate();
            }
            _ => {
                let message = candidate
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("Unable to load lyrics file {}", candidate.path.display()));
                tracing::warn!("{message}");
                self.play_status = Some(message);
            }
        }
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

    /// Bakes the current offset (plus any `[offset:]` tag) into the selected `.lrc`
    /// (atomic replace, previous file kept as `.bak`) and resets the user offset to 0.
    pub(crate) fn save_lyrics_offset_to_selected_file(&mut self) {
        let Some(index) = self.selected_lyrics_index else {
            self.play_status = Some("No lyrics file selected.".to_string());
            return;
        };
        let Some(path) = self.lyrics_files.get(index).cloned() else {
            self.play_status = Some("Selected lyrics file was not found.".to_string());
            return;
        };

        let saved = match save_lrc_offset(&path, self.lyrics_offset_ms) {
            Ok(saved) => saved,
            Err(err) => {
                self.play_status = Some(format!("Unable to save lyrics offset: {err:#}"));
                return;
            }
        };
        if saved.backup_path.is_none() {
            self.play_status = Some("Lyrics offset is 0 ms; nothing to save.".to_string());
            return;
        }

        // Reload the rewritten file (single small file) with a zero user offset.
        let reloaded = (Lyrics::from_lrc(&path), LyricsTimeline::from_lrc(&path));
        self.set_lyrics_offset(0);
        if let (Ok(lyrics), Ok(timeline)) = reloaded {
            if let Some(candidate) = self.lyrics_candidates.get_mut(index) {
                candidate.lyrics = Some(lyrics);
                candidate.timeline = Some(timeline);
                candidate.error = None;
            }
            self.load_lyrics_file(index);
        }
        self.play_status = Some(format!(
            "Saved lyrics offset to {} (updated {} timestamp tags; backup: {}).",
            path.display(),
            saved.shifted_tags,
            saved
                .backup_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        ));
    }
}

#[cfg(test)]
#[path = "tests/test_lyrics_offset.rs"]
mod tests;
