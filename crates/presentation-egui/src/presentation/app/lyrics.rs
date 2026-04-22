//! 歌詞（LRC）ファイルの管理を担当
//! - 歌詞ファイルの検索
//! - 読み込みと優先度ソート

use crate::presentation::app::GuiApp;
use lrc_player_application::infrastructure::lyrics::Lyrics;

impl GuiApp {
    pub(crate) fn discover_lyrics_for_current_play_path(&mut self) {
        if self.play_path.is_empty() {
            self.lyrics_files.clear();
            self.selected_lyrics_index = None;
            self.loaded_lyrics = None;
            return;
        }

        self.lyrics_files = Lyrics::discover_for_track(&self.play_path, &self.scan_lrc_targets);

        self.lyrics_files.sort_by(|a, b| {
            let a_rank = Lyrics::from_lrc(a)
                .map(|lyrics| lyrics.display_priority_rank())
                .unwrap_or(0);
            let b_rank = Lyrics::from_lrc(b)
                .map(|lyrics| lyrics.display_priority_rank())
                .unwrap_or(0);

            b_rank
                .cmp(&a_rank)
                .then_with(|| a.file_name().cmp(&b.file_name()))
        });

        self.selected_lyrics_index = None;
        self.loaded_lyrics = None;

        if !self.lyrics_files.is_empty() {
            self.load_lyrics_file(0);
        }
    }

    pub(crate) fn load_lyrics_file(&mut self, index: usize) {
        if let Some(path) = self.lyrics_files.get(index) {
            match Lyrics::from_lrc(path) {
                Ok(lyrics) => {
                    self.loaded_lyrics = Some(lyrics);
                    self.selected_lyrics_index = Some(index);
                }
                Err(err) => {
                    eprintln!("Unable to load lyrics file {}: {}", path.display(), err);
                }
            }
        }
    }
}
