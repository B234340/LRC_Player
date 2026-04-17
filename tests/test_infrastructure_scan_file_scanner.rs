#[cfg(test)]
mod tests {
    use std::fs;
    use std::fs::File;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use lrc_player::infrastructure::scan::file_scanner::*;
    use tempfile::tempdir;

    #[test]
    fn enumerate_paths_finds_supported_audio_files() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("subdir")).unwrap();
        File::create(root.join("track.mp3")).unwrap();
        File::create(root.join("subdir").join("song.flac")).unwrap();
        File::create(root.join("hidden.mp3")).unwrap();
        File::create(root.join(".secret.wav")).unwrap();

        let scanner = FileScanner::new();
        let paths = scanner.enumerate_paths(root, &[]);
        let normalized: Vec<_> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();

        assert!(normalized.contains(&"track.mp3".to_string()));
        assert!(normalized.contains(&"song.flac".to_string()));
        assert!(!normalized.contains(&".secret.wav".to_string()));
    }

    #[test]
    fn enumerate_paths_applies_exclude_paths() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("subdir")).unwrap();
        File::create(root.join("track.mp3")).unwrap();
        File::create(root.join("subdir").join("song.flac")).unwrap();
        File::create(root.join("exclude_me.mp3")).unwrap();

        let scanner = FileScanner::new();
        let excludes = vec![root.join("exclude_me.mp3")];
        let paths = scanner.enumerate_paths(root, &excludes);
        let normalized: Vec<_> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();

        assert!(normalized.contains(&"track.mp3".to_string()));
        assert!(normalized.contains(&"song.flac".to_string()));
        assert!(!normalized.contains(&"exclude_me.mp3".to_string()));
    }

    #[test]
    fn benchmark_scan_like_workflow() {
        let windows_root = Path::new("/mnt/c/Users/OU/Music/Music");
        let (root, expected_count) = if windows_root.exists() {
            println!("Using real Windows path for benchmark test: {}", windows_root.display());
            (windows_root.to_path_buf(), None)
        } else {
            let tmp = tempdir().unwrap();
            let root = tmp.path().join("tracks");
            FileScanner::generate_dummy_track_files(&root, 10000);
            (root, Some(10000))
        };

        let scanner = FileScanner::new();
        let paths = scanner.enumerate_paths(&root, &[]);
        assert!(!paths.is_empty());
        if let Some(expected) = expected_count {
            assert_eq!(paths.len(), expected);
        }
    }
}
