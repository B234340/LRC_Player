use std::fs;
use std::path::{Path, PathBuf};

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "flac", "aac"];

#[derive(Default)]
pub struct FileScanner;

impl FileScanner {
    pub fn new() -> Self {
        Self
    }

    pub fn enumerate_paths(&self, path: &Path) -> Vec<PathBuf> {
        let mut results = Vec::new();
        self.collect_paths(path, &mut results, true);
        results
    }

    fn collect_paths(&self, path: &Path, results: &mut Vec<PathBuf>, is_root: bool) {
        let metadata = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(_) => return,
        };

        if metadata.file_type().is_symlink() {
            return;
        }

        if metadata.is_file() {
            if self.is_supported_file(path) && (is_root || !self.is_hidden(path)) {
                results.push(path.to_path_buf());
            }
            return;
        }

        if metadata.is_dir() {
            if !is_root && self.is_hidden(path) {
                return;
            }

            let read_dir = match fs::read_dir(path) {
                Ok(rd) => rd,
                Err(_) => return,
            };

            for entry in read_dir.flatten() {
                let child_path = entry.path();
                self.collect_paths(&child_path, results, false);
            }
        }
    }

    fn is_hidden(&self, path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.starts_with('.'))
            .unwrap_or(false)
    }

    fn is_supported_file(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                AUDIO_EXTENSIONS
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(ext))
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
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
        let paths = scanner.enumerate_paths(root);
        let normalized: Vec<_> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();

        assert!(normalized.contains(&"track.mp3".to_string()));
        assert!(normalized.contains(&"song.flac".to_string()));
        assert!(!normalized.contains(&".secret.wav".to_string()));
    }
}
