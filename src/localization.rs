use crate::config::Language;

pub fn translate(key: &str, lang: Language) -> &'static str {
    match lang {
        Language::En => match key {
            "title" => "LRC Player",
            "output_device" => "Output device:",
            "filter" => "Filter:",
            "save_device" => "Save device",
            "resampler_taps" => "Resampler taps:",
            "save_taps" => "Save taps",
            "play_demo" => "Play demo path:",
            "play" => "Play",
            "send_scan" => "Send scan request",
            "no_matching_devices" => "(no matching devices)",
            "current_playback" => "Current playback controls are not yet connected.",
            "settings_header" => "Application settings and preferences.",
            "playlists_header" => "Saved playlists will appear here.",
            "queue_header" => "Playback queue will be shown here.",
            "library_albums" => "Albums:",
            "library_tracks" => "Tracks:",
            "scan_status_none" => "No scan activity recorded yet.",
            "language" => "Language:",
            _ => "",
        },
        Language::Ja => match key {
            "title" => "LRC プレイヤー",
            "output_device" => "出力デバイス:",
            "filter" => "フィルター:",
            "save_device" => "デバイスを保存",
            "resampler_taps" => "リサンプラータップ:",
            "save_taps" => "タップを保存",
            "play_demo" => "デモ再生パス:",
            "play" => "再生",
            "send_scan" => "スキャン要求を送信",
            "no_matching_devices" => "(一致するデバイスがありません)",
            "current_playback" => "現在の再生コントロールはまだ接続されていません。",
            "settings_header" => "アプリ設定とプリファレンス。",
            "playlists_header" => "保存されたプレイリストがここに表示されます。",
            "queue_header" => "再生キューがここに表示されます。",
            "library_albums" => "アルバム:",
            "library_tracks" => "トラック:",
            "scan_status_none" => "まだスキャン活動が記録されていません。",
            "language" => "言語:",
            _ => "",
        },
        Language::Zh => match key {
            "title" => "LRC 播放器",
            "output_device" => "输出设备:",
            "filter" => "筛选:",
            "save_device" => "保存设备",
            "resampler_taps" => "重采样抽头:",
            "save_taps" => "保存抽头",
            "play_demo" => "播放演示路径:",
            "play" => "播放",
            "send_scan" => "发送扫描请求",
            "no_matching_devices" => "(没有匹配的设备)",
            "current_playback" => "当前播放控件尚未连接。",
            "settings_header" => "应用设置和首选项。",
            "playlists_header" => "已保存的播放列表将显示在此处。",
            "queue_header" => "播放队列将显示在此处。",
            "library_albums" => "专辑:",
            "library_tracks" => "曲目:",
            "scan_status_none" => "尚未记录扫描活动。",
            "language" => "语言:",
            _ => "",
        },
    }
}

pub fn supported_languages() -> &'static [Language] {
    &[Language::En, Language::Ja, Language::Zh]
}

pub fn language_name(lang: Language) -> &'static str {
    match lang {
        Language::En => "English",
        Language::Ja => "日本語",
        Language::Zh => "中文",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Language;

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
