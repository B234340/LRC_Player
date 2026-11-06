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
            ..Default::default()
        };
        file.save(&original).unwrap();

        let loaded = file.load().unwrap();
        assert_eq!(loaded.resampler_taps, original.resampler_taps);
        assert_eq!(
            loaded.preferred_output_device,
            original.preferred_output_device
        );
    }

    #[test]
    fn old_config_without_resampler_loads_with_fft_default() {
        let json = r#"{"resampler_taps":32,"preferred_output_device":null,"language":"ja","theme":"dark","log_level":"debug"}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.resampler, lrc_player::config::ResamplerSetting::Fft);
        assert_eq!(cfg.theme, Theme::Dark);
        assert_eq!(cfg.log_level.as_filter_str(), "debug");
    }
}
