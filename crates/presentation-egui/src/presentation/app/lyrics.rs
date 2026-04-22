//! 歌詞（LRC）ファイルの管理を担当
//! - 歌詞ファイルの検索
//! - 読み込みと優先度ソート

use crate::presentation::app::GuiApp;
use lrc_player_application::infrastructure::lyrics::Lyrics;

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
}