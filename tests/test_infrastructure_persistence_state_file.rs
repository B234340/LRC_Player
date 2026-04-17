#[cfg(test)]
mod tests {
    use lrc_player::application::dto::StateDto;
    use lrc_player::infrastructure::persistence::json_store::JsonStore;
    use anyhow::{Context, Result};
    use directories::ProjectDirs;
    use std::path::{Path, PathBuf};
    use lrc_player::infrastructure::persistence::state_file::*;
    use tempfile::tempdir;

    #[test]
    fn state_file_roundtrips_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        let file = StateFile::open_with_path(path).unwrap();

        let original = StateDto {
            last_tab: Some("playlist".into()),
            last_play_path: Some("/tmp/track1.mp3".into()),
            last_play_position_ms: Some(120_000),
            last_scan_status: None,
            scan_scanned: None,
            scan_parsed: None,
            scan_registered: None,
            scan_targets: Vec::new(),
            scan_excludes: Vec::new(),
            queue_tracks: vec!["/tmp/track1.mp3".into(), "/tmp/track2.mp3".into()],
            selected_queue_index: Some(1),
            favorite_directories: Vec::new(),
        };
        file.save(&original).unwrap();

        let loaded = file.load().unwrap();
        assert_eq!(loaded.last_tab, original.last_tab);
    }
}
