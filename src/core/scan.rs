//! Core::scan — ファイルシステムスキャンとメタデータ抽出
//!
//! このモジュールは音楽ファイル群の走査（スキャン）を担当します。
//! ディレクトリを再帰的に探索し、サポートされる音声ファイルを検出、
//! メタデータ（タグやファイル情報）を抽出して `TrackInput` を生成します。
//!
//! 並列解析を用いてタグ抽出のパフォーマンスを高めつつ、探索自体は逐次で行い
//! 作業負荷の暴発を抑えます。長時間実行される操作はワーカーとして起動できます。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use lofty::file::AudioFile;
use lofty::prelude::{Accessor, ItemKey, TaggedFileExt};
use lofty::probe::Probe;

use crate::core::library::{Library, TrackInput};
use crate::core::strings::StringInterner;
use rayon::prelude::*;

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

#[derive(Debug, Clone)]
pub struct ScanOutcome {
    pub result: ScanState,
    pub progress: ScanProgress,
    pub tracks: Vec<TrackInput>,
    pub targeted: bool,
}

#[derive(Debug, Clone)]
pub enum ScanWorkerMessage {
    Progress(ScanProgress),
    Finished(ScanOutcome),
}

impl ScanWorkerMessage {
    /// Debug / test helper to stringify the message variant if needed.
    #[allow(dead_code)]
    pub fn is_finished(&self) -> bool {
        matches!(self, ScanWorkerMessage::Finished(_))
    }
}

pub struct ScanWorkerHandle {
    cancel_requested: Arc<AtomicBool>,
    receiver: Receiver<ScanWorkerMessage>,
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
    StartTargetedScan(Vec<PathBuf>),
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
                self.config
                    .include_dirs
                    .retain(|existing| existing != &path);
            }
            ScanCommand::AddExcludeDir(path) => {
                if !self.config.exclude_dirs.contains(&path) {
                    self.config.exclude_dirs.push(path);
                }
            }
            ScanCommand::RemoveExcludeDir(path) => {
                self.config
                    .exclude_dirs
                    .retain(|existing| existing != &path);
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
        let outcome = run_full_scan_collect(self.config.clone(), None, None);
        let summary = outcome.summary();
        self.progress = outcome.progress.clone();
        self.finish_with(outcome.result);
        (
            build_library_from_track_inputs(strings, outcome.tracks),
            summary,
        )
    }

    pub fn run_targeted_scan(&mut self, roots: Vec<PathBuf>, strings: &mut StringInterner) -> (Library, ScanSummary) {
        let outcome = run_targeted_scan_collect(roots, self.config.exclude_dirs.clone(), None, None);
        let summary = outcome.summary();
        self.progress = outcome.progress.clone();
        self.finish_with(outcome.result);
        (
            build_library_from_track_inputs(strings, outcome.tracks),
            summary,
        )
    }
}

impl ScanOutcome {
    pub fn summary(&self) -> ScanSummary {
        ScanSummary {
            result: self.result,
            discovered_files: self.progress.discovered_files,
            parsed_files: self.progress.parsed_files,
            skipped_files: self.progress.skipped_files,
        }
    }
}

impl ScanWorkerHandle {
    pub fn spawn(config: ScanConfig) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel_requested);

        thread::spawn(move || {
            let outcome =
                run_full_scan_collect(config, Some(worker_cancel.as_ref()), Some(&sender));
            let _ = sender.send(ScanWorkerMessage::Finished(outcome));
        });

        Self {
            cancel_requested,
            receiver,
        }
    }

    pub fn spawn_targeted(roots: Vec<PathBuf>, exclude_dirs: Vec<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel_requested);

        thread::spawn(move || {
            let outcome = run_targeted_scan_collect(roots, exclude_dirs, Some(worker_cancel.as_ref()), Some(&sender));
            let _ = sender.send(ScanWorkerMessage::Finished(outcome));
        });

        Self {
            cancel_requested,
            receiver,
        }
    }

    pub fn cancel(&self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
    }

    pub fn drain_messages(&self) -> Vec<ScanWorkerMessage> {
        let mut messages = Vec::new();
        while let Ok(message) = self.receiver.try_recv() {
            messages.push(message);
        }
        messages
    }
}

pub fn build_library_from_track_inputs<I>(strings: &mut StringInterner, tracks: I) -> Library
where
    I: IntoIterator<Item = TrackInput>,
{
    let mut library = Library::default();
    for track in tracks {
        library.upsert_track(strings, track);
    }
    library
}

fn run_full_scan_collect(
    config: ScanConfig,
    cancel_requested: Option<&AtomicBool>,
    sender: Option<&Sender<ScanWorkerMessage>>,
) -> ScanOutcome {
    let include_dirs = normalize_paths(&config.include_dirs);
    let exclude_dirs = normalize_paths(&config.exclude_dirs);
    let mut progress = ScanProgress::default();
    let mut tracks = Vec::new();
    let mut had_warning = false;
    let mut cancelled = false;

    for include_dir in include_dirs {
        if is_cancel_requested(cancel_requested) {
            cancelled = true;
            break;
        }

        if !include_dir.exists() {
            had_warning = true;
            progress.skipped_files += 1;
            send_progress(sender, &progress);
            continue;
        }

        match scan_directory_collect(
            &include_dir,
            &exclude_dirs,
            &mut progress,
            &mut tracks,
            cancel_requested,
            sender,
        ) {
            Ok(true) => {
                cancelled = true;
                break;
            }
            Ok(false) => {}
            Err(_) => {
                had_warning = true;
            }
        }
    }

    let result = if cancelled {
        ScanState::Cancelling
    } else if progress.discovered_files == 0 && progress.skipped_files > 0 {
        ScanState::Failed
    } else if had_warning || progress.skipped_files > 0 {
        ScanState::Warning
    } else {
        ScanState::Completed
    };

    ScanOutcome {
        result,
        progress,
        tracks,
        targeted: false,
    }
}

fn run_targeted_scan_collect(
    roots: Vec<PathBuf>,
    exclude_dirs: Vec<PathBuf>,
    cancel_requested: Option<&AtomicBool>,
    sender: Option<&Sender<ScanWorkerMessage>>,
) -> ScanOutcome {
    let include_dirs = normalize_paths(&roots);
    let exclude_dirs = normalize_paths(&exclude_dirs);
    let mut progress = ScanProgress::default();
    let mut tracks = Vec::new();
    let mut had_warning = false;
    let mut cancelled = false;

    for include_dir in include_dirs {
        if is_cancel_requested(cancel_requested) {
            cancelled = true;
            break;
        }

        if !include_dir.exists() {
            had_warning = true;
            progress.skipped_files += 1;
            send_progress(sender, &progress);
            continue;
        }

        match scan_directory_collect(
            &include_dir,
            &exclude_dirs,
            &mut progress,
            &mut tracks,
            cancel_requested,
            sender,
        ) {
            Ok(true) => {
                cancelled = true;
                break;
            }
            Ok(false) => {}
            Err(_) => {
                had_warning = true;
            }
        }
    }

    let result = if cancelled {
        ScanState::Cancelling
    } else if progress.discovered_files == 0 && progress.skipped_files > 0 {
        ScanState::Failed
    } else if had_warning || progress.skipped_files > 0 {
        ScanState::Warning
    } else {
        ScanState::Completed
    };

    ScanOutcome {
        result,
        progress,
        tracks,
        targeted: true,
    }
}

fn scan_directory_collect(
    root: &Path,
    exclude_dirs: &[PathBuf],
    progress: &mut ScanProgress,
    tracks: &mut Vec<TrackInput>,
    cancel_requested: Option<&AtomicBool>,
    sender: Option<&Sender<ScanWorkerMessage>>,
) -> io::Result<bool> {
    let mut stack = vec![root.to_path_buf()];

    while let Some(directory) = stack.pop() {
        if is_cancel_requested(cancel_requested) {
            return Ok(true);
        }

        if should_skip_path(&directory, exclude_dirs) {
            continue;
        }

        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                progress.skipped_files += 1;
                send_progress(sender, progress);
                continue;
            }
        };
        // Collect files in this directory and parse them in parallel to speed up
        // metadata/tag extraction (lofty/symphonia work). We still traverse
        // subdirectories serially to avoid explosion of work-stealing overhead.
        let mut files_to_parse: Vec<PathBuf> = Vec::new();

        for entry in entries {
            if is_cancel_requested(cancel_requested) {
                return Ok(true);
            }

            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    progress.skipped_files += 1;
                    send_progress(sender, progress);
                    continue;
                }
            };
            let path = entry.path();

            if should_skip_path(&path, exclude_dirs) {
                continue;
            }

            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => {
                    progress.skipped_files += 1;
                    send_progress(sender, progress);
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

            progress.discovered_files += 1;
            files_to_parse.push(path);
        }

        if !files_to_parse.is_empty() {
            // indicate work in progress for this directory
            progress.current_path = Some(directory.clone());
            send_progress(sender, progress);

            // Parse files in parallel; collect results and update progress.
            let results: Vec<Option<TrackInput>> = files_to_parse
                .par_iter()
                .map(|p| build_track_input_from_path(p))
                .collect();

            for res in results {
                if is_cancel_requested(cancel_requested) {
                    return Ok(true);
                }
                match res {
                    Some(track_input) => {
                        tracks.push(track_input);
                        progress.parsed_files += 1;
                    }
                    None => {
                        progress.skipped_files += 1;
                    }
                }
                send_progress(sender, progress);
            }
        }
    }

    Ok(false)
}

fn send_progress(sender: Option<&Sender<ScanWorkerMessage>>, progress: &ScanProgress) {
    if let Some(sender) = sender {
        let _ = sender.send(ScanWorkerMessage::Progress(progress.clone()));
    }
}

fn is_cancel_requested(cancel_requested: Option<&AtomicBool>) -> bool {
    cancel_requested
        .map(|flag| flag.load(Ordering::Relaxed))
        .unwrap_or(false)
}

fn normalize_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
        .collect()
}

fn should_skip_path(path: &Path, exclude_dirs: &[PathBuf]) -> bool {
    is_hidden_path(path)
        || exclude_dirs
            .iter()
            .any(|excluded| path.starts_with(excluded))
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
        Some(extension) => matches!(
            extension.to_ascii_lowercase().as_str(),
            "mp3" | "wav" | "flac"
        ),
        None => false,
    }
}

fn build_track_input(path: &Path, metadata: &fs::Metadata) -> Option<TrackInput> {
    let canonical_path = fs::canonicalize(path).ok()?;
    let normalized_path = canonical_path.to_string_lossy().to_string();
    let file_name = path.file_name()?.to_string_lossy().to_string();
    let fallback_title = path.file_stem()?.to_string_lossy().to_string();
    let fallback_album = path
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().to_string());
    let fallback_artist = path
        .parent()
        .and_then(|parent| parent.parent())
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().to_string());
    let fallback_track_number = fallback_title
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
    let tag_metadata = read_tag_metadata(path);
    let title = tag_metadata
        .as_ref()
        .and_then(|metadata| metadata.title.clone())
        .unwrap_or(fallback_title);
    let album = tag_metadata
        .as_ref()
        .and_then(|metadata| metadata.album.clone())
        .or(fallback_album.clone());
    let artist = tag_metadata
        .as_ref()
        .and_then(|metadata| metadata.artist.clone())
        .or(fallback_artist.clone());
    let album_artist = tag_metadata
        .as_ref()
        .and_then(|metadata| metadata.album_artist.clone())
        .or_else(|| artist.clone());
    let composer = tag_metadata
        .as_ref()
        .and_then(|metadata| metadata.composer.clone());
    let track_number = tag_metadata
        .as_ref()
        .and_then(|metadata| metadata.track_number)
        .or(fallback_track_number);
    let duration_ms = tag_metadata
        .as_ref()
        .map(|metadata| metadata.duration_ms)
        .unwrap_or(0);

    Some(TrackInput {
        title,
        album,
        artist,
        album_artist,
        composer,
        normalized_path,
        file_size: metadata.len(),
        modified_unix_secs,
        file_name,
        track_number,
        duration_ms,
        artwork_key: None,
    })
}

/// Public helper to build a `TrackInput` from a file path by collecting
/// filesystem metadata and tag metadata. Returns `None` if the file cannot
/// be read or parsed.
pub fn build_track_input_from_path(path: &Path) -> Option<TrackInput> {
    let metadata = fs::metadata(path).ok()?;
    build_track_input(path, &metadata)
}

#[derive(Debug, Clone, Default)]
struct TagMetadata {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    composer: Option<String>,
    track_number: Option<std::num::NonZeroU16>,
    duration_ms: u32,
}

fn read_tag_metadata(path: &Path) -> Option<TagMetadata> {
    let tagged_file = Probe::open(path).ok()?.read().ok()?;
    let properties = tagged_file.properties();
    let duration_ms = properties.duration().as_millis().min(u32::MAX as u128) as u32;
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());

    let Some(tag) = tag else {
        return Some(TagMetadata {
            duration_ms,
            ..TagMetadata::default()
        });
    };

    let composer = tag
        .get_string(&ItemKey::Composer)
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty());
    let album_artist = tag
        .get_string(&ItemKey::AlbumArtist)
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty());

    Some(TagMetadata {
        title: tag
            .title()
            .map(|value| value.to_string())
            .filter(|value| !value.trim().is_empty()),
        artist: tag
            .artist()
            .map(|value| value.to_string())
            .filter(|value| !value.trim().is_empty()),
        album: tag
            .album()
            .map(|value| value.to_string())
            .filter(|value| !value.trim().is_empty()),
        album_artist,
        composer,
        track_number: tag
            .track()
            .and_then(|value| u16::try_from(value).ok())
            .and_then(std::num::NonZeroU16::new),
        duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::thread;

    use hound::{SampleFormat, WavSpec, WavWriter};
    use tempfile::TempDir;

    use super::{ScanCommand, ScanModel, ScanState, ScanWorkerHandle, ScanWorkerMessage};
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

        fs::write(artist_dir.join("01-first.wav"), b"RIFFdemo")
            .expect("audio file should be created");
        fs::write(artist_dir.join("02-second.flac"), b"fLaCdemo")
            .expect("audio file should be created");
        fs::write(artist_dir.join("notes.txt"), b"lyrics")
            .expect("non-music file should be created");
        fs::write(excluded_dir.join("03-excluded.mp3"), b"ID3demo")
            .expect("excluded file should be created");
        fs::write(hidden_dir.join("04-hidden.mp3"), b"ID3demo")
            .expect("hidden file should be created");

        let mut scan = ScanModel::default();
        scan.config.include_dirs.push(include_dir.clone());
        scan.config.exclude_dirs.push(excluded_dir.clone());
        let mut strings = StringInterner::default();

        let (library, summary) = scan.run_full_scan(&mut strings);

        assert_eq!(summary.discovered_files, 2);
        assert_eq!(summary.parsed_files, 2);
        assert_eq!(summary.result, ScanState::Completed);
        assert_eq!(library.tracks.titles.len(), 2);
        assert_eq!(
            library
                .visible_tracks_for_album(crate::core::ids::AlbumId(0))
                .len(),
            2
        );
    }

    #[test]
    fn background_scan_worker_finishes_and_reports_tracks() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let include_dir = temp_dir.path().join("music");
        let artist_dir = include_dir.join("Artist").join("Album");

        fs::create_dir_all(&artist_dir).expect("artist dir should be created");
        fs::write(artist_dir.join("01-first.wav"), b"RIFFdemo")
            .expect("audio file should be created");

        let mut scan = ScanModel::default();
        scan.config.include_dirs.push(include_dir);
        let worker = ScanWorkerHandle::spawn(scan.config.clone());

        let mut saw_progress = false;
        let mut finished = None;
        for _ in 0..10_000 {
            for message in worker.drain_messages() {
                match message {
                    ScanWorkerMessage::Progress(progress) => {
                        saw_progress = saw_progress || progress.current_path.is_some();
                    }
                    ScanWorkerMessage::Finished(outcome) => {
                        finished = Some(outcome);
                    }
                }
            }
            if finished.is_some() {
                break;
            }
            thread::yield_now();
        }

        let outcome = finished.expect("scan should finish");
        assert!(saw_progress);
        assert_eq!(outcome.result, ScanState::Completed);
        assert_eq!(outcome.tracks.len(), 1);
        assert_eq!(outcome.progress.parsed_files, 1);
    }

    #[test]
    fn full_scan_extracts_duration_from_audio_properties() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let include_dir = temp_dir.path().join("music");
        let artist_dir = include_dir.join("Artist").join("Album");

        fs::create_dir_all(&artist_dir).expect("artist dir should be created");
        let file_path = artist_dir.join("01-first.wav");
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&file_path, spec).expect("wav file should be created");
        for _ in 0..48_000u32 {
            writer
                .write_sample(0i16)
                .expect("left sample should be written");
            writer
                .write_sample(0i16)
                .expect("right sample should be written");
        }
        writer.finalize().expect("wav file should be finalized");

        let mut scan = ScanModel::default();
        scan.config.include_dirs.push(include_dir);
        let mut strings = StringInterner::default();

        let (library, summary) = scan.run_full_scan(&mut strings);

        assert_eq!(summary.parsed_files, 1);
        assert_eq!(library.tracks.durations_ms.len(), 1);
        assert!(library.tracks.durations_ms[0] >= 900);
        assert_eq!(
            strings.resolve(library.tracks.titles[0]).as_deref(),
            Some("01-first")
        );
    }
}
