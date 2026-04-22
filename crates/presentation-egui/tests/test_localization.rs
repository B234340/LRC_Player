#[cfg(test)]
mod tests {
    use lrc_player::localization::*;
    use lrc_player::config::Language;

    #[test]
    fn translate_returns_expected_strings() {
        assert_eq!(translate("title", Language::En), "LRC Player");
        assert_eq!(translate("title", Language::Ja), "LRC プレイヤー");
        assert_eq!(translate("title", Language::Zh), "LRC 播放器");
        assert_eq!(translate("play", Language::Ja), "再生");
    }

    #[test]
    fn language_name_matches_language() {
        assert_eq!(language_name(Language::En), "English");
        assert_eq!(language_name(Language::Ja), "日本語");
        assert_eq!(language_name(Language::Zh), "中文");
    }
}
