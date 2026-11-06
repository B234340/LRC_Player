//! Tiny localization hook for the `ui` components.
//!
//! Components receive a [`Tr`] (Copy) and look strings up by key. Japanese and English
//! defaults are built in; the app can plug its own lookup (e.g. the application
//! `localization` module) through [`Tr::with_hook`]. Unknown keys render as the key itself
//! so missing translations are visible instead of blank.

/// UI language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    #[default]
    Ja,
    En,
}

/// Optional override: return `Some` to replace a built-in string.
pub type TrHook = fn(key: &str, lang: Lang) -> Option<&'static str>;

/// Translator handle passed to components.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tr {
    pub lang: Lang,
    pub hook: Option<TrHook>,
}

impl Tr {
    pub const JA: Tr = Tr { lang: Lang::Ja, hook: None };
    pub const EN: Tr = Tr { lang: Lang::En, hook: None };

    pub fn new(lang: Lang) -> Self {
        Self { lang, hook: None }
    }

    pub fn with_hook(lang: Lang, hook: TrHook) -> Self {
        Self { lang, hook: Some(hook) }
    }

    /// Look up `key`.
    pub fn t(&self, key: &'static str) -> &'static str {
        if let Some(hook) = self.hook {
            if let Some(s) = hook(key, self.lang) {
                return s;
            }
        }
        builtin(key, self.lang).unwrap_or(key)
    }
}

/// All keys used by the `ui` components (kept in sync with [`builtin`] by a test).
pub const KEYS: &[&str] = &[
    "app.name",
    "nav.now_playing",
    "nav.library",
    "nav.albums",
    "nav.artists",
    "nav.playlists",
    "nav.folders",
    "nav.settings",
    "nav.collapse",
    "nav.expand",
    "nav.playlists_section",
    "nav.scanning",
    "transport.play",
    "transport.pause",
    "transport.previous",
    "transport.next",
    "transport.shuffle",
    "transport.repeat_off",
    "transport.repeat_all",
    "transport.repeat_one",
    "transport.lyrics",
    "transport.queue",
    "transport.eq",
    "transport.mute",
    "transport.unmute",
    "transport.volume",
    "transport.nothing_playing",
    "transport.open_now_playing",
    "search.placeholder",
    "search.clear",
    "table.number",
    "table.title",
    "table.artist",
    "table.album",
    "table.duration",
    "table.year",
    "table.genre",
    "table.tracks_count",
    "table.no_results",
    "table.no_results_hint",
    "menu.play",
    "menu.play_next",
    "menu.add_to_queue",
    "menu.add_to_playlist",
    "menu.show_album",
    "menu.show_artist",
    "menu.open_folder",
    "menu.details",
    "menu.no_playlists",
    "albums.tracks",
    "albums.play",
    "albums.empty",
    "now.no_track",
    "now.no_lyrics",
    "now.no_lyrics_hint",
    "lyrics.back_to_current",
    "lyrics.offset",
    "lyrics.offset_minus",
    "lyrics.offset_plus",
    "lyrics.offset_reset",
    "lyrics.offset_save",
    "lyrics.font_smaller",
    "lyrics.font_larger",
    "lyrics.shift_hint",
    "queue.title",
    "queue.clear",
    "queue.save_as_playlist",
    "queue.remove",
    "queue.empty",
    "queue.empty_hint",
    "queue.now_playing",
    "queue.up_next",
    "eq.title",
    "eq.enabled",
    "eq.preset",
    "eq.preamp",
    "eq.reset",
    "eq.preset.flat",
    "eq.preset.rock",
    "eq.preset.pop",
    "eq.preset.jazz",
    "eq.preset.classical",
    "eq.preset.vocal",
    "eq.preset.bass_boost",
    "eq.preset.treble_boost",
    "eq.preset.custom",
    "library.empty",
    "library.empty_hint",
    "library.add_folder",
    "toast.close",
    "drop.title",
    "drop.hint",
    "shortcut.play_pause",
    "shortcut.seek",
    "shortcut.prev_next",
    "shortcut.volume",
    "shortcut.mute",
    "shortcut.search",
    "shortcut.lyrics",
    "shortcut.settings",
    "shortcut.escape",
    "settings.appearance",
    "settings.theme",
    "settings.theme_system",
    "settings.theme_dark",
    "settings.theme_light",
    "settings.accent",
];

fn builtin(key: &str, lang: Lang) -> Option<&'static str> {
    let (ja, en) = match key {
        "app.name" => ("LRC Player", "LRC Player"),
        "nav.now_playing" => ("再生中", "Now Playing"),
        "nav.library" => ("ライブラリ", "Library"),
        "nav.albums" => ("アルバム", "Albums"),
        "nav.artists" => ("アーティスト", "Artists"),
        "nav.playlists" => ("プレイリスト", "Playlists"),
        "nav.folders" => ("フォルダー", "Folders"),
        "nav.settings" => ("設定", "Settings"),
        "nav.collapse" => ("サイドバーを折りたたむ", "Collapse sidebar"),
        "nav.expand" => ("サイドバーを展開", "Expand sidebar"),
        "nav.playlists_section" => ("プレイリスト", "Playlists"),
        "nav.scanning" => ("スキャン中…", "Scanning…"),
        "transport.play" => ("再生 (Space)", "Play (Space)"),
        "transport.pause" => ("一時停止 (Space)", "Pause (Space)"),
        "transport.previous" => ("前の曲 (Shift+←)", "Previous (Shift+←)"),
        "transport.next" => ("次の曲 (Shift+→)", "Next (Shift+→)"),
        "transport.shuffle" => ("シャッフル", "Shuffle"),
        "transport.repeat_off" => ("リピート: オフ", "Repeat: off"),
        "transport.repeat_all" => ("リピート: 全曲", "Repeat: all"),
        "transport.repeat_one" => ("リピート: 1曲", "Repeat: one"),
        "transport.lyrics" => ("歌詞 (Ctrl+L)", "Lyrics (Ctrl+L)"),
        "transport.queue" => ("再生キュー", "Queue"),
        "transport.eq" => ("イコライザー", "Equalizer"),
        "transport.mute" => ("ミュート (M)", "Mute (M)"),
        "transport.unmute" => ("ミュート解除 (M)", "Unmute (M)"),
        "transport.volume" => ("音量 (↑/↓)", "Volume (↑/↓)"),
        "transport.nothing_playing" => ("再生していません", "Nothing playing"),
        "transport.open_now_playing" => ("再生中の画面を開く", "Open Now Playing"),
        "search.placeholder" => ("曲名・アーティスト・アルバムを検索", "Search title, artist, album"),
        "search.clear" => ("クリア (Esc)", "Clear (Esc)"),
        "table.number" => ("#", "#"),
        "table.title" => ("タイトル", "Title"),
        "table.artist" => ("アーティスト", "Artist"),
        "table.album" => ("アルバム", "Album"),
        "table.duration" => ("時間", "Time"),
        "table.year" => ("年", "Year"),
        "table.genre" => ("ジャンル", "Genre"),
        "table.tracks_count" => ("曲", "tracks"),
        "table.no_results" => ("一致する曲がありません", "No matching tracks"),
        "table.no_results_hint" => ("別のキーワードで検索してください", "Try a different search"),
        "menu.play" => ("再生", "Play"),
        "menu.play_next" => ("次に再生", "Play next"),
        "menu.add_to_queue" => ("キューに追加", "Add to queue"),
        "menu.add_to_playlist" => ("プレイリストに追加", "Add to playlist"),
        "menu.show_album" => ("アルバムを表示", "Show album"),
        "menu.show_artist" => ("アーティストを表示", "Show artist"),
        "menu.open_folder" => ("フォルダーを開く", "Open containing folder"),
        "menu.details" => ("詳細情報", "Details"),
        "menu.no_playlists" => ("プレイリストがありません", "No playlists"),
        "albums.tracks" => ("曲", "tracks"),
        "albums.play" => ("アルバムを再生", "Play album"),
        "albums.empty" => ("アルバムがありません", "No albums yet"),
        "now.no_track" => ("曲が選択されていません", "No track selected"),
        "now.no_lyrics" => ("歌詞がありません", "No lyrics"),
        "now.no_lyrics_hint" => (
            ".lrc ファイルを曲と同じフォルダーに置くか、ウィンドウにドロップしてください",
            "Put an .lrc file next to the track or drop one onto the window",
        ),
        "lyrics.back_to_current" => ("現在位置へ戻る", "Back to current"),
        "lyrics.offset" => ("オフセット", "Offset"),
        "lyrics.offset_minus" => ("歌詞を早める −100ms (Shift: −500ms)", "Earlier −100 ms (Shift: −500 ms)"),
        "lyrics.offset_plus" => ("歌詞を遅らせる +100ms (Shift: +500ms)", "Later +100 ms (Shift: +500 ms)"),
        "lyrics.offset_reset" => ("オフセットをリセット", "Reset offset"),
        "lyrics.offset_save" => ("オフセットをファイルに保存", "Save offset to file"),
        "lyrics.font_smaller" => ("文字を小さく", "Smaller text"),
        "lyrics.font_larger" => ("文字を大きく", "Larger text"),
        "lyrics.shift_hint" => ("Shift で 500ms 単位", "Hold Shift for 500 ms steps"),
        "queue.title" => ("再生キュー", "Queue"),
        "queue.clear" => ("クリア", "Clear"),
        "queue.save_as_playlist" => ("プレイリストとして保存", "Save as playlist"),
        "queue.remove" => ("キューから削除", "Remove from queue"),
        "queue.empty" => ("キューは空です", "The queue is empty"),
        "queue.empty_hint" => ("ライブラリから曲をダブルクリックして再生", "Double-click a track in the library to play it"),
        "queue.now_playing" => ("再生中", "Now playing"),
        "queue.up_next" => ("次に再生", "Up next"),
        "eq.title" => ("イコライザー", "Equalizer"),
        "eq.enabled" => ("有効", "Enabled"),
        "eq.preset" => ("プリセット", "Preset"),
        "eq.preamp" => ("プリアンプ", "Preamp"),
        "eq.reset" => ("リセット", "Reset"),
        "eq.preset.flat" => ("フラット", "Flat"),
        "eq.preset.rock" => ("ロック", "Rock"),
        "eq.preset.pop" => ("ポップ", "Pop"),
        "eq.preset.jazz" => ("ジャズ", "Jazz"),
        "eq.preset.classical" => ("クラシック", "Classical"),
        "eq.preset.vocal" => ("ボーカル", "Vocal"),
        "eq.preset.bass_boost" => ("低音強調", "Bass Boost"),
        "eq.preset.treble_boost" => ("高音強調", "Treble Boost"),
        "eq.preset.custom" => ("カスタム", "Custom"),
        "library.empty" => ("ライブラリは空です", "Your library is empty"),
        "library.empty_hint" => (
            "音楽フォルダーを追加するとスキャンが始まります",
            "Add a music folder to start scanning",
        ),
        "library.add_folder" => ("フォルダーを追加", "Add folder"),
        "toast.close" => ("閉じる", "Dismiss"),
        "drop.title" => ("ドロップして再生", "Drop to play"),
        "drop.hint" => (
            "音楽ファイルやフォルダーはキューに追加、.lrc は再生中の曲の歌詞になります",
            "Audio files and folders are queued; an .lrc attaches lyrics to the current track",
        ),
        "shortcut.play_pause" => ("再生 / 一時停止", "Play / pause"),
        "shortcut.seek" => ("5 秒戻る / 進む", "Seek −5 s / +5 s"),
        "shortcut.prev_next" => ("前の曲 / 次の曲", "Previous / next track"),
        "shortcut.volume" => ("音量を上げる / 下げる", "Volume up / down"),
        "shortcut.mute" => ("ミュート切り替え", "Toggle mute"),
        "shortcut.search" => ("検索", "Search"),
        "shortcut.lyrics" => ("歌詞パネル", "Lyrics panel"),
        "shortcut.settings" => ("設定", "Settings"),
        "shortcut.escape" => ("ポップアップを閉じる / 検索をクリア", "Close popups / clear search"),
        "settings.appearance" => ("外観", "Appearance"),
        "settings.theme" => ("テーマ", "Theme"),
        "settings.theme_system" => ("システム", "System"),
        "settings.theme_dark" => ("ダーク", "Dark"),
        "settings.theme_light" => ("ライト", "Light"),
        "settings.accent" => ("アクセントカラー", "Accent color"),
        _ => return None,
    };
    Some(match lang {
        Lang::Ja => ja,
        Lang::En => en,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_both_languages() {
        for key in KEYS {
            assert!(builtin(key, Lang::Ja).is_some(), "missing ja: {key}");
            assert!(builtin(key, Lang::En).is_some(), "missing en: {key}");
        }
        assert_eq!(Tr::EN.t("nav.library"), "Library");
        assert_eq!(Tr::JA.t("nav.library"), "ライブラリ");
        assert_eq!(Tr::JA.t("does.not.exist"), "does.not.exist");
        let tr = Tr::with_hook(Lang::En, |k, _| (k == "nav.library").then_some("Music"));
        assert_eq!(tr.t("nav.library"), "Music");
    }
}
