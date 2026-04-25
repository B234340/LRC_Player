//! 歌詞（LRC）ファイルの管理を担当
//! - 歌詞ファイルの検索
//! - 読み込みと優先度ソート

use crate::presentation::app::GuiApp;
use lrc_player_application::infrastructure::lyrics::Lyrics;
use std::fs;

impl GuiApp {
    /// 現在の再生パスに対応する歌詞（LRC）ファイルを検索します。
    /// 
    /// この関数は以下の処理を行います：
    /// 1. 再生パスの有効性チェック
    /// 2. トラックに関連するLRCファイルを検索
    /// 3. 検索結果を表示優先度順にソート
    /// 4. 最も優先度の高いファイルを自動読み込み
    /// 
    /// 検索対象：
    /// - トラックと同じディレクトリ内のLRCファイル
    /// - スキャン設定で指定されたLRC専用ディレクトリ
    /// 
    /// ソート基準：
    /// 1. 表示優先度ランク（高い順）
    ///    - トラック名と完全一致するファイルが最優先
    ///    - アーティスト名を含むファイル
    ///    - その他のファイル
    /// 2. ファイル名（アルファベット順）
    /// 
    /// 再生パスが空の場合は、すべての歌詞状態をクリアします。
    pub(crate) fn discover_lyrics_for_current_play_path(&mut self) {
        // 再生パスが空の場合、全歌詞状態をクリア
        if self.play_path.is_empty() {
            self.lyrics_files.clear();
            self.selected_lyrics_index = None;
            self.loaded_lyrics = None;
            return;
        }

        // トラックに関連するLRCファイルを検索
        // scan_lrc_targets: ユーザーが設定したLRC専用スキャンディレクトリ
        self.lyrics_files = Lyrics::discover_for_track(&self.play_path, &self.scan_lrc_targets);

        // 検索結果を表示優先度でソート
        self.lyrics_files.sort_by(|a, b| {
            // 各ファイルの表示優先度ランクを取得
            // 解析に失敗した場合はランク0（最低優先度）
            let a_rank = Lyrics::from_lrc(a)
                .map(|lyrics| lyrics.display_priority_rank())
                .unwrap_or(0);
            let b_rank = Lyrics::from_lrc(b)
                .map(|lyrics| lyrics.display_priority_rank())
                .unwrap_or(0);

            // 優先度の降順（高い方が先）でソート
            // 同じ優先度の場合はファイル名の昇順
            b_rank
                .cmp(&a_rank)
                .then_with(|| a.file_name().cmp(&b.file_name()))
        });

        // 選択状態と読み込み済み歌詞をリセット
        self.selected_lyrics_index = None;
        self.loaded_lyrics = None;

        // 検索結果が存在する場合、最も優先度の高いファイルを自動読み込み
        if !self.lyrics_files.is_empty() {
            self.load_lyrics_file(0);
        }
    }

    /// 指定されたインデックスの歌詞ファイルを読み込みます。
    /// 
    /// LRCファイルを解析し、内部的な歌詞データ構造に変換します。
    /// 読み込みに成功すると、以下の状態が更新されます：
    /// - `loaded_lyrics`: 解析された歌詞データ
    /// - `selected_lyrics_index`: 選択中の歌詞ファイルのインデックス
    /// 
    /// 読み込みに失敗した場合は、エラーメッセージを標準エラー出力に表示し、
    /// 状態は更新されません。
    /// 
    /// # 引数
    /// * `index` - `lyrics_files`ベクタ内の読み込むファイルのインデックス
    /// 
    /// # パニック
    /// この関数はパニックしません。インデックスが範囲外の場合や
    /// ファイル読み込みに失敗した場合は、エラーを出力して正常に戻ります。
    pub(crate) fn load_lyrics_file(&mut self, index: usize) {
        // 指定インデックスのファイルパスを取得
        if let Some(path) = self.lyrics_files.get(index) {
            // LRCファイルを解析
            match Lyrics::from_lrc(path) {
                Ok(lyrics) => {
                    // 解析成功：状態を更新
                    self.loaded_lyrics = Some(lyrics);
                    self.selected_lyrics_index = Some(index);
                }
                Err(err) => {
                    // 解析失敗：エラーを出力（状態は変更しない）
                    eprintln!("Unable to load lyrics file {}: {}", path.display(), err);
                }
            }
        }
    }

    /// 再生トラックが変更された場合に歌詞オフセットをリセットします。
    ///
    /// 同じトラックを再度再生した場合はリセットしません。
    pub(crate) fn reset_lyrics_offset_if_track_changed(&mut self, new_play_path: &str) {
        if self.play_path != new_play_path {
            self.lyrics_offset_ms = 0;
            self.save_state();
        }
    }

    /// 選択中のLRCファイルへ現在の歌詞オフセットを恒久反映します。
    ///
    /// - `[]` と `<>` のタイムタグをすべて走査
    /// - `lyrics_offset_ms` だけ時刻を加算/減算
    /// - 反映後は `lyrics_offset_ms` を 0 に戻し、歌詞を再読み込み
    pub(crate) fn save_lyrics_offset_to_selected_file(&mut self) {
        let Some(index) = self.selected_lyrics_index else {
            self.play_status = Some("No lyrics file selected.".to_string());
            return;
        };

        let Some(path) = self.lyrics_files.get(index).cloned() else {
            self.play_status = Some("Selected lyrics file was not found.".to_string());
            return;
        };

        if self.lyrics_offset_ms == 0 {
            self.play_status = Some("Lyrics offset is 0 ms; nothing to save.".to_string());
            return;
        }

        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) => {
                self.play_status = Some(format!(
                    "Unable to read lyrics file {}: {}",
                    path.display(),
                    err
                ));
                return;
            }
        };

        let (shifted, replaced_count) = shift_lrc_timestamps(&contents, self.lyrics_offset_ms);
        if replaced_count == 0 {
            self.play_status = Some(format!(
                "No timestamp tags found in {}.",
                path.display()
            ));
            return;
        }

        if let Err(err) = fs::write(&path, shifted) {
            self.play_status = Some(format!(
                "Unable to write lyrics file {}: {}",
                path.display(),
                err
            ));
            return;
        }

        self.lyrics_offset_ms = 0;
        self.load_lyrics_file(index);
        self.save_state();
        self.play_status = Some(format!(
            "Saved lyrics offset to {} (updated {} timestamp tags).",
            path.display(),
            replaced_count
        ));
    }
}

fn shift_lrc_timestamps(contents: &str, offset_ms: i64) -> (String, usize) {
    let mut output = String::with_capacity(contents.len());
    let mut replaced_count = 0usize;
    let mut i = 0usize;

    while i < contents.len() {
        let remaining = &contents[i..];
        let (open, close) = if remaining.starts_with('[') {
            ('[', ']')
        } else if remaining.starts_with('<') {
            ('<', '>')
        } else {
            let ch = remaining
                .chars()
                .next()
                .expect("remaining is non-empty while iterating");
            output.push(ch);
            i += ch.len_utf8();
            continue;
        };

        let search_start = i + open.len_utf8();
        let Some(end_rel) = contents[search_start..].find(close) else {
            let ch = remaining
                .chars()
                .next()
                .expect("remaining is non-empty while iterating");
            output.push(ch);
            i += ch.len_utf8();
            continue;
        };

        let end = search_start + end_rel;
        let tag = &contents[search_start..end];
        if let Some(ms) = parse_lrc_timestamp(tag) {
            let shifted_ms = apply_signed_offset(ms, offset_ms);
            output.push(open);
            output.push_str(&format_lrc_timestamp(shifted_ms));
            output.push(close);
            replaced_count += 1;
            i = end + close.len_utf8();
            continue;
        }

        output.push(open);
        i = search_start;
    }

    (output, replaced_count)
}

fn parse_lrc_timestamp(tag: &str) -> Option<u64> {
    let mut parts = tag.splitn(2, ':');
    let minutes = parts.next()?.parse::<u64>().ok()?;
    let seconds_part = parts.next()?;

    let mut sec_parts = seconds_part.splitn(2, '.');
    let seconds = sec_parts.next()?.parse::<u64>().ok()?;
    if seconds >= 60 {
        return None;
    }

    let millis = match sec_parts.next() {
        Some(frac) => {
            if frac.is_empty() || frac.len() > 3 || !frac.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let normalized = match frac.len() {
                1 => format!("{}00", frac),
                2 => format!("{}0", frac),
                _ => frac.to_string(),
            };
            normalized.parse::<u64>().ok()?
        }
        None => 0,
    };

    Some(minutes * 60_000 + seconds * 1_000 + millis)
}

fn apply_signed_offset(ms: u64, offset_ms: i64) -> u64 {
    // どんな場合でも負の値にならないように、0でクランプする。
    if offset_ms >= 0 {
        ms.saturating_add(offset_ms as u64)
    } else {
        ms.saturating_sub(offset_ms.unsigned_abs())
    }
}

fn format_lrc_timestamp(ms: u64) -> String {
    let minutes = ms / 60_000;
    let seconds = (ms / 1_000) % 60;
    let millis = ms % 1_000;
    format!("{:02}:{:02}.{:03}", minutes, seconds, millis)
}

#[cfg(test)]
#[path = "tests/test_lyrics_offset.rs"]
mod tests;
