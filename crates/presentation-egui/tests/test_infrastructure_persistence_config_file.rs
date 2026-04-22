#[cfg(test)]
mod tests {
    use lrc_player::config::{Config, Language, LogLevel, Theme};
    use lrc_player::infrastructure::persistence::config_file::*;
    use tempfile::tempdir;

    #[test]
    fn config_file_roundtrips_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let file = ConfigFile::open_with_path(path).unwrap();

        let original = Config {
            resampler_taps: 64,
            preferred_output_device: Some("usb_audio".into()),
            device_settings: std::collections::HashMap::new(),
            language: Language::En,
            theme: Theme::System,
            log_level: LogLevel::Info,
        };
        file.save(&original).unwrap();

        let loaded = file.load().unwrap();
        assert_eq!(loaded.resampler_taps, original.resampler_taps);
        assert_eq!(
            loaded.preferred_output_device,
            original.preferred_output_device
        );
    }
}
