#[cfg(test)]
mod tests {
    use lrc_player::application::dto::StateDto;
    use lrc_player::infrastructure::persistence::state_file::StateFile;
    use anyhow::Result;
    use lrc_player::application::repositories::state_repository::*;
    use tempfile::tempdir;

    #[test]
    fn state_repository_roundtrips_state() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        let store = StateFile::open_with_path(path).unwrap();

        let original = StateDto {
            last_tab: Some("scan".into()),
            last_play_path: None,
            last_play_position_ms: None,
            last_scan_status: None,
            scan_scanned: None,
            scan_parsed: None,
            scan_registered: None,
            scan_targets: Vec::new(),
            scan_excludes: Vec::new(),
            queue_tracks: Vec::new(),
            selected_queue_index: None,
            favorite_directories: Vec::new(),
        };
        store.save(&original).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.last_tab, original.last_tab);
    }
}
