use std::fs;
use std::path::{Path, PathBuf};

use crate::core::library::{Library, TrackInput};
use crate::core::strings::StringInterner;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanState {
    #[default]
    Idle,
    Scanning,
    Cancelling,
    Completed,
    Warning,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanProgress {
    pub discovered_files: u32,
    pub parsed_files: u32,
    pub skipped_files: u32,
    pub current_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanConfig {
    pub include_dirs: Vec<PathBuf>,
    pub exclude_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanModel {
    pub state: ScanState,
    pub last_visible_result: Option<ScanState>,
    pub progress: ScanProgress,
    pub config: ScanConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanSummary {
    pub result: ScanState,
    pub discovered_files: u32,
    pub parsed_files: u32,
    pub skipped_files: u32,
}

impl Default for ScanModel {
    fn default() -> Self {
        Self {
            state: ScanState::Idle,
            last_visible_result: None,
            progress: ScanProgress::default(),
            config: ScanConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanCommand {
    StartFullScan,
    Stop,
    AddIncludeDir(PathBuf),
    RemoveIncludeDir(PathBuf),
    AddExcludeDir(PathBuf),
    RemoveExcludeDir(PathBuf),
}

impl ScanModel {
    pub fn apply(&mut self, command: ScanCommand) {
        match command {
            ScanCommand::StartFullScan => {
                self.state = ScanState::Scanning;
                self.progress = ScanProgress::default();
            }
            ScanCommand::Stop if self.state == ScanState::Scanning => {
                self.state = ScanState::Cancelling;
                self.last_visible_result = Some(ScanState::Cancelling);
            }
            ScanCommand::AddIncludeDir(path) => {
                if !self.config.include_dirs.contains(&path) {
                    self.config.include_dirs.push(path);
                }
            }
            ScanCommand::RemoveIncludeDir(path) => {
                self.config.include_dirs.retain(|existing| existing != &path);
            }
            ScanCommand::AddExcludeDir(path) => {
                if !self.config.exclude_dirs.contains(&path) {
                    self.config.exclude_dirs.push(path);
                }
            }
            ScanCommand::RemoveExcludeDir(path) => {
                self.config.exclude_dirs.retain(|existing| existing != &path);
            }
            _ => {}
        }
    }

    pub fn finish_with(&mut self, result: ScanState) {
        self.state = result;
        self.last_visible_result = Some(result);
    }

    pub fn visible_status(&self) -> ScanState {
        match self.state {
            ScanState::Idle => self.last_visible_result.unwrap_or(ScanState::Idle),
            other => other,
        }
    }

    pub fn reset_if_terminal(&mut self) {
        if matches!(
            self.state,
            ScanState::Cancelling | ScanState::Completed | ScanState::Warning | ScanState::Failed
        ) {
            self.state = ScanState::Idle;
        }
    }

    pub fn run_full_scan(&mut self, strings: &mut StringInterner) -> (Library, ScanSummary) {
        self.state = ScanState::Scanning;
        self.progress = ScanProgress::default();

        let include_dirs = normalize_paths(&self.config.include_dirs);
        let exclude_dirs = normalize_paths(&self.config.exclude_dirs);
        let mut library = Library::default();
        let mut had_warning = false;

        for include_dir in include_dirs {
            if !include_dir.exists() {
                had_warning = true;
                self.progress.skipped_files += 1;
                continue;
            }

            if self.scan_directory(&include_dir, &exclude_dirs, strings, &mut library).is_err() {
                had_warning = true;
            }
        }

        let result = if self.progress.discovered_files == 0 && self.progress.skipped_files > 0 {
            ScanState::Failed
        } else if had_warning || self.progress.skipped_files > 0 {
            ScanState::Warning
        } else {
            ScanState::Completed
        };

        self.finish_with(result);

        (
            library,
            ScanSummary {
                result,
                discovered_files: self.progress.discovered_files,
                parsed_files: self.progress.parsed_files,
                skipped_files: self.progress.skipped_files,
            },
        )
    }

    fn scan_directory(
        &mut self,
        root: &Path,
        exclude_dirs: &[PathBuf],
        strings: &mut StringInterner,
        library: &mut Library,
    ) -> std::io::Result<()> {
        let mut stack = vec![root.to_path_buf()];

        while let Some(directory) = stack.pop() {
            if should_skip_path(&directory, exclude_dirs) {
                continue;
            }

            let entries = match fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(_) => {
                    self.progress.skipped_files += 1;
                    continue;
                }
            };

            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => {
                        self.progress.skipped_files += 1;
                        continue;
                    }
                };
                let path = entry.path();
                self.progress.current_path = Some(path.clone());

                if should_skip_path(&path, exclude_dirs) {
                    continue;
                }

                let metadata = match fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(_) => {
                        self.progress.skipped_files += 1;
                        continue;
                    }
                };

                if metadata.file_type().is_symlink() {
                    continue;
                }

                if metadata.is_dir() {
                    stack.push(path);
                    continue;
                }

                if !metadata.is_file() || !is_supported_music_file(&path) {
                    continue;
                }

                self.progress.discovered_files += 1;
                match build_track_input(&path, &metadata) {
                    Some(track_input) => {
                        library.upsert_track(strings, track_input);
                        self.progress.parsed_files += 1;
                    }
                    None => {
                        self.progress.skipped_files += 1;
                    }
                }
            }
        }

        Ok(())
    }
}

fn normalize_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths.iter()
        .map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
        .collect()
}

fn should_skip_path(path: &Path, exclude_dirs: &[PathBuf]) -> bool {
    is_hidden_path(path) || exclude_dirs.iter().any(|excluded| path.starts_with(excluded))
}

fn is_hidden_path(path: &Path) -> bool {
    path.file_name()
        .map(|name| {
            let name = name.to_string_lossy();
            name.starts_with('.') && name.len() > 1
        })
        .unwrap_or(false)
}

fn is_supported_music_file(path: &Path) -> bool {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) => matches!(extension.to_ascii_lowercase().as_str(), "mp3" | "wav" | "flac"),
        None => false,
    }
}

fn build_track_input(path: &Path, metadata: &fs::Metadata) -> Option<TrackInput> {
    let canonical_path = fs::canonicalize(path).ok()?;
    let normalized_path = canonical_path.to_string_lossy().to_string();
    let file_name = path.file_name()?.to_string_lossy().to_string();
    let title = path.file_stem()?.to_string_lossy().to_string();
    let album = path
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().to_string());
    let artist = path
        .parent()
        .and_then(|parent| parent.parent())
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().to_string());
    let track_number = title
        .split(|character: char| !character.is_ascii_digit())
        .find(|segment| !segment.is_empty())
        .and_then(|segment| segment.parse::<u16>().ok())
        .and_then(std::num::NonZeroU16::new);
    let modified_unix_secs = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();

    Some(TrackInput {
        title,
        album: album.clone(),
        artist: artist.clone(),
        album_artist: artist,
        composer: None,
        normalized_path,
        file_size: metadata.len(),
        modified_unix_secs,
        file_name,
        track_number,
        duration_ms: 0,
        artwork_key: None,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{ScanCommand, ScanModel, ScanState};
    use crate::core::strings::StringInterner;

    #[test]
    fn terminal_result_stays_visible_until_tab_change() {
        let mut scan = ScanModel::default();
        scan.apply(ScanCommand::StartFullScan);
        scan.finish_with(ScanState::Warning);

        assert_eq!(scan.visible_status(), ScanState::Warning);

        scan.reset_if_terminal();
        assert_eq!(scan.state, ScanState::Idle);
        assert_eq!(scan.visible_status(), ScanState::Warning);
    }

    #[test]
    fn full_scan_registers_supported_files_and_skips_hidden_and_excluded_paths() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let include_dir = temp_dir.path().join("music");
        let artist_dir = include_dir.join("Artist").join("Album");
        let excluded_dir = include_dir.join("skip_me");
        let hidden_dir = include_dir.join(".hidden");

        fs::create_dir_all(&artist_dir).expect("artist dir should be created");
        fs::create_dir_all(&excluded_dir).expect("excluded dir should be created");
        fs::create_dir_all(&hidden_dir).expect("hidden dir should be created");

        fs::write(artist_dir.join("01-first.wav"), b"RIFFdemo").expect("audio file should be created");
        fs::write(artist_dir.join("02-second.flac"), b"fLaCdemo").expect("audio file should be created");
        fs::write(artist_dir.join("notes.txt"), b"lyrics").expect("non-music file should be created");
        fs::write(excluded_dir.join("03-excluded.mp3"), b"ID3demo").expect("excluded file should be created");
        fs::write(hidden_dir.join("04-hidden.mp3"), b"ID3demo").expect("hidden file should be created");

        let mut scan = ScanModel::default();
        scan.config.include_dirs.push(include_dir.clone());
        scan.config.exclude_dirs.push(excluded_dir.clone());
        let mut strings = StringInterner::default();

        let (library, summary) = scan.run_full_scan(&mut strings);

        assert_eq!(summary.discovered_files, 2);
        assert_eq!(summary.parsed_files, 2);
        assert_eq!(summary.result, ScanState::Completed);
        assert_eq!(library.tracks.titles.len(), 2);
        assert_eq!(library.visible_tracks_for_album(crate::core::ids::AlbumId(0)).len(), 2);
    }
}