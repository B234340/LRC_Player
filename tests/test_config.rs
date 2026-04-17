#[cfg(test)]
mod tests {
    use directories::ProjectDirs;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use lrc_player::config::*;

    #[test]
    fn device_specific_taps_override_global_setting() {
        let mut cfg = Config::default();
        cfg.resampler_taps = 64;
        cfg.set_taps_for_device("Device A".into(), 128);

        assert_eq!(cfg.taps_for_device(None), 64);
        assert_eq!(cfg.taps_for_device(Some(&"Device A".into())), 128);
        assert_eq!(cfg.taps_for_device(Some(&"Device B".into())), 64);
    }
}
