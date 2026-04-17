use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "flac", "aac"];

#[derive(Default)]
pub struct FileScanner;

impl FileScanner {
    pub fn new() -> Self {
        Self
    }

    pub fn enumerate_paths(&self, path: &Path, excludes: &[PathBuf]) -> Vec<PathBuf> {
        let mut results = Vec::new();
        self.collect_paths(path, &mut results, true, excludes);
        results
    }

    pub fn generate_dummy_track_files(root: &Path, count: usize) {
        fs::create_dir_all(root).unwrap();
        let dirs = 10.min(count);
        let files_per_dir = count.div_ceil(dirs);

        for dir_index in 0..dirs {
            let dir = root.join(format!("batch_{:02}", dir_index));
            fs::create_dir_all(&dir).unwrap();

            for file_index in 0..files_per_dir {
                let global_index = dir_index * files_per_dir + file_index;
                if global_index >= count {
                    break;
                }
                let file_name = format!("track_{:06}.mp3", global_index);
                let file_path = dir.join(file_name);
                let mut file = File::create(&file_path).unwrap();
                file.write_all(b"dummy audio content\n").unwrap();
            }
        }
    }

    fn collect_paths(&self, path: &Path, results: &mut Vec<PathBuf>, is_root: bool, excludes: &[PathBuf]) {
        if self.is_excluded(path, excludes) {
            return;
        }

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
                self.collect_paths(&child_path, results, false, excludes);
            }
        }
    }

    fn is_hidden(&self, path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.starts_with('.'))
            .unwrap_or(false)
    }

    fn is_excluded(&self, path: &Path, excludes: &[PathBuf]) -> bool {
        excludes.iter().any(|exclude| {
            if exclude.is_absolute() {
                path.starts_with(exclude)
            } else {
                path.to_string_lossy()
                    .contains(&exclude.to_string_lossy().to_string())
            }
        })
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


