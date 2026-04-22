#[cfg(test)]
mod tests {
    use lrc_player::config::*;

    #[test]
    fn device_specific_taps_override_global_setting() {
        let mut cfg = Config {
            resampler_taps: 64,
            ..Default::default()
        };
        cfg.set_taps_for_device("Device A".into(), 128);

        assert_eq!(cfg.taps_for_device(None), 64);
        assert_eq!(cfg.taps_for_device(Some(&"Device A".into())), 128);
        assert_eq!(cfg.taps_for_device(Some(&"Device B".into())), 64);
    }
}
