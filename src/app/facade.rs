//! AppFacade: アプリケーションの中核オーケストレータ
//!
//! - `AppFacade` はライブラリ、キュー、プレイリスト、再生モデル、スキャンワーカー、
//!   デコーダ/デコードワーカー、オーディオ出力などアプリの主要サブシステムを所有します。
//! - UI からの `AppCommand` を受け取り、状態変更やバックグラウンドワーカー起動、永続化のトリガを行い、
//!   `AppEvent` 列を返して UI に通知します。
//! - 大きな責務を持つため、非同期ワーカー（スキャン/デコード/ブラウズローダ等）とは明確に境界を設け、
//!   メッセージ駆動で処理を行います。

use std::io;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::commands::{AppCommand, TrackListOrigin};
use crate::app::events::{AppEvent, PlaybackEvent};
use crate::app::view_models::{
    AlbumEntryVm, AlbumTabVm, AppViewModel, ArtistEntryVm, ArtistTabVm, DeferredListStateVm,
    DirectoryEntryVm, FavoriteStateVm, LibraryTabVm, PlayTabVm, PlaylistEntryVm, PlaylistTabVm,
    QueueTabVm, ScanTabVm, SettingItemVm, SettingsTabVm, TrackRowVm,
};
use crate::audio::device_watcher::DeviceWatcherHandle;
use crate::audio::output::{AudioDeviceOutput, AudioOutputConfig, BufferedAudioOutput};
use crate::core::ids::{AlbumId, ArtistId, StrId, TrackId};
use crate::core::library::{ArtistKind, DirectoryNodeId, Library};
use crate::core::persistence::{
    AppConfigFile,
    AppStateFile,
    JsonStore,
    PlaylistsFile,
    TabId,
    AlbumIndexEntry,
    ArtistIndexFile,
    ArtistIndexEntry,
};
use crate::core::artwork::ArtworkCache;
use crate::core::playback::PlaybackModel;
use crate::core::playback::PlaybackState;
use crate::core::playlists::{PlaylistKind, PlaylistStore};
use crate::core::queue::Queue;
use crate::core::scan::{
    ScanCommand, ScanModel, ScanOutcome, ScanState, ScanWorkerHandle, ScanWorkerMessage,
    build_library_from_track_inputs,
};
use crate::core::strings::StringInterner;
use crate::decode::decoder::{DecodeWorkerHandle, DecodeWorkerMessage, Decoder};
use crate::decode::source::DecodedSource;
use cpal::traits::{DeviceTrait, HostTrait};

const AUDIO_BLOCK_CAPACITY: usize = 8_192;
const BROWSE_HYDRATE_BATCH_SIZE: usize = 64;
const INITIAL_DECODE_PRIME_BLOCKS: usize = 64;

// ブラウズ用のログスナップショットエントリ（永続化インデックスからの読み出し等で使用）
#[derive(Debug, Clone, PartialEq, Eq)]
struct AlbumLogEntry {
    album_id: AlbumId,
    title: String,
    album_artist: String,
    track_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArtistLogEntry {
    artist_id: ArtistId,
    name: String,
    track_count: usize,
}

// ブラウズ UI のための遅延ハイドレーション状態
#[derive(Debug, Clone, Default)]
struct DeferredAlbumListState {
    log_entries: Vec<AlbumLogEntry>,
    hydrated_entries: Vec<AlbumEntryVm>,
}

#[derive(Debug, Clone, Default)]
struct DeferredArtistListState {
    log_entries: Vec<ArtistLogEntry>,
    hydrated_entries: Vec<ArtistEntryVm>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BrowseCacheState {
    library_revision: u64,
    albums: DeferredAlbumListState,
    track_artists: DeferredArtistListState,
    album_artists: DeferredArtistListState,
    composers: DeferredArtistListState,
}

// 背景ローダからのメッセージ（単一トラック、バッチ、完了マーカー）
#[derive(Debug)]
pub(crate) enum BrowseLoaderMessage {
    Track(crate::core::library::TrackInput),
    TrackBatch(Vec<crate::core::library::TrackInput>),
    Finished { missing_paths: Vec<String> },
}

// ブラウズ用のバックグラウンドローダハンドル
pub(crate) struct BrowseLoaderHandle {
    cancel_requested: Arc<AtomicBool>,
    receiver: Receiver<BrowseLoaderMessage>,
}

impl BrowseLoaderHandle {
    /// スレッドを生成して与えられたパス群を検証・TrackInput に変換し、
    /// バッチ単位でメインスレッドへ送信するローダを起動します。
    ///
    /// - `paths` は永続化インデックスなどから得たファイルパスの列。
    /// - 戻り値はキャンセルフラグと受信チャネルを保持するハンドルです。
    pub fn spawn(paths: Vec<String>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel_requested);

        // ワーカスレッド: ファイル存在チェック -> トラック入力生成 -> バッチ送信
        thread::spawn(move || {
            let mut missing_paths: Vec<String> = Vec::new();
            let mut batch: Vec<crate::core::library::TrackInput> = Vec::new();
            for p in paths {
                if worker_cancel.load(Ordering::Relaxed) {
                    break;
                }
                let path = Path::new(&p);
                if !path.exists() {
                    missing_paths.push(p.clone());
                    continue;
                }
                if let Some(track_input) = crate::core::scan::build_track_input_from_path(path) {
                    batch.push(track_input);
                    if batch.len() >= BROWSE_HYDRATE_BATCH_SIZE {
                        let send_batch = std::mem::take(&mut batch);
                        let _ = sender.send(BrowseLoaderMessage::TrackBatch(send_batch));
                    }
                } else {
                    missing_paths.push(p.clone());
                }
            }
            if !batch.is_empty() {
                let _ = sender.send(BrowseLoaderMessage::TrackBatch(batch));
            }
            let _ = sender.send(BrowseLoaderMessage::Finished { missing_paths });
        });

        Self {
            cancel_requested,
            receiver,
        }
    }

    pub fn cancel(&self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
    }
    pub fn drain_messages(&self) -> Vec<BrowseLoaderMessage> {
        let mut messages = Vec::new();
        // UI フレーム内での処理量を制限するためバッチ上限を設ける
        for _ in 0..BROWSE_HYDRATE_BATCH_SIZE {
            if let Ok(msg) = self.receiver.try_recv() {
                messages.push(msg);
            } else {
                break;
            }
        }
        messages
    }
}

// Background builder for the full browse cache. Builds a BrowseCacheState
// from cloned `Library` + `StringInterner` on a worker thread and sends
// the completed cache back to the main thread for an atomic swap.
pub(crate) struct BrowseCacheBuilderHandle {
    cancel_requested: Arc<AtomicBool>,
    receiver: Receiver<BrowseCacheState>,
    target_revision: u64,
}

impl BrowseCacheBuilderHandle {
    /// ブラウズ用キャッシュ（アルバム/アーティストの一覧等）をバックグラウンドで構築するビルダを起動する。
    ///
    /// ライブラリと文字列インターンをクローンして重い処理をワーカスレッドで実行し、
    /// 完成した `BrowseCacheState` をメインスレッドに送信する。
    pub fn spawn(library: Library, strings: StringInterner) -> Self {
        let target_revision = library.revision;
        let (sender, receiver) = mpsc::channel();
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel_requested);

        thread::spawn(move || {
            if worker_cancel.load(Ordering::Relaxed) {
                return;
            }

            let mut state = BrowseCacheState::default();
            state.library_revision = target_revision;

            // Build albums
            let album_entries: Vec<AlbumLogEntry> = library
                .albums
                .titles
                .iter()
                .enumerate()
                .map(|(index, title_id)| AlbumLogEntry {
                    album_id: AlbumId(index as u32),
                    title: strings
                        .resolve(*title_id)
                        .unwrap_or_else(|| "Unknown".to_string()),
                    album_artist: library
                        .albums
                        .album_artist_ids
                        .get(index)
                        .and_then(|artist_id| {
                            artist_id.and_then(|id| library.artist_name(id, &strings))
                        })
                        .unwrap_or_else(|| "Unknown".to_string()),
                    track_count: library.indexes.tracks_by_album.get(index).map_or(0, Vec::len),
                })
                .collect();

            if worker_cancel.load(Ordering::Relaxed) {
                return;
            }

            state.albums.reset(album_entries);

            // Build artist lists (track artists / album artists / composers)
            let build_artist_entries = |source: &Vec<Vec<TrackId>>| -> Vec<ArtistLogEntry> {
                let mut out: Vec<ArtistLogEntry> = Vec::new();
                for (index, name_id) in library.artists.names.iter().enumerate() {
                    if worker_cancel.load(Ordering::Relaxed) {
                        return Vec::new();
                    }
                    let track_count = source.get(index).map_or(0, Vec::len);
                    if track_count == 0 {
                        continue;
                    }
                    let name = strings
                        .resolve(*name_id)
                        .unwrap_or_else(|| "Unknown".to_string());
                    out.push(ArtistLogEntry { artist_id: ArtistId(index as u32), name, track_count });
                }
                out
            };

            if worker_cancel.load(Ordering::Relaxed) {
                return;
            }

            let track_artist_entries = build_artist_entries(&library.indexes.tracks_by_artist);
            state.track_artists.reset(track_artist_entries);

            if worker_cancel.load(Ordering::Relaxed) {
                return;
            }

            let album_artist_entries = build_artist_entries(&library.indexes.tracks_by_album_artist);
            state.album_artists.reset(album_artist_entries);

            if worker_cancel.load(Ordering::Relaxed) {
                return;
            }

            let composer_entries = build_artist_entries(&library.indexes.tracks_by_composer);
            state.composers.reset(composer_entries);

            if worker_cancel.load(Ordering::Relaxed) {
                return;
            }

            let _ = sender.send(state);
        });

        Self { cancel_requested, receiver, target_revision }
    }

    pub fn cancel(&self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
    }

    /// ワーカが生成したキャッシュ状態をすべて受け取って返す（通常は 1 件）
    pub fn drain_messages(&self) -> Vec<BrowseCacheState> {
        let mut messages = Vec::new();
        // 完成したキャッシュは通常単一だが、複数ある場合はすべて受け取る
        while let Ok(msg) = self.receiver.try_recv() {
            messages.push(msg);
        }
        messages
    }

    /// ビルダが目標としているライブラリ revision（この revision に対してビルド中）を返す
    pub fn target_revision(&self) -> u64 {
        self.target_revision
    }
}

impl DeferredAlbumListState {
    fn reset(&mut self, log_entries: Vec<AlbumLogEntry>) {
        self.log_entries = log_entries;
        self.hydrated_entries.clear();
    }

    fn hydrate_next_batch(&mut self) {
        let start = self.hydrated_entries.len();
        let end = (start + BROWSE_HYDRATE_BATCH_SIZE).min(self.log_entries.len());
        if start >= end {
            return;
        }

        self.hydrated_entries
            .extend(
                self.log_entries[start..end]
                    .iter()
                    .map(|entry| AlbumEntryVm {
                        album_id: entry.album_id,
                        title: entry.title.clone(),
                        album_artist: entry.album_artist.clone(),
                        track_count: entry.track_count,
                        is_provisional: false,
                    }),
            );
    }

    fn visible_entries(&self) -> Vec<AlbumEntryVm> {
        let mut entries = self.hydrated_entries.clone();
        entries.extend(
            self.log_entries[self.hydrated_entries.len()..]
                .iter()
                .map(|entry| AlbumEntryVm {
                    album_id: entry.album_id,
                    title: entry.title.clone(),
                    album_artist: entry.album_artist.clone(),
                    track_count: entry.track_count,
                    is_provisional: true,
                }),
        );
        entries
    }

    fn loading_state(&self) -> DeferredListStateVm {
        DeferredListStateVm {
            total_entries: self.log_entries.len(),
            hydrated_entries: self.hydrated_entries.len(),
            is_loading: self.hydrated_entries.len() < self.log_entries.len(),
        }
    }
}

impl DeferredArtistListState {
    fn reset(&mut self, log_entries: Vec<ArtistLogEntry>) {
        self.log_entries = log_entries;
        self.hydrated_entries.clear();
    }

    fn hydrate_next_batch(&mut self) {
        let start = self.hydrated_entries.len();
        let end = (start + BROWSE_HYDRATE_BATCH_SIZE).min(self.log_entries.len());
        if start >= end {
            return;
        }

        self.hydrated_entries
            .extend(
                self.log_entries[start..end]
                    .iter()
                    .map(|entry| ArtistEntryVm {
                        artist_id: entry.artist_id,
                        name: entry.name.clone(),
                        track_count: entry.track_count,
                        is_provisional: false,
                    }),
            );
    }

    fn visible_entries(&self) -> Vec<ArtistEntryVm> {
        let mut entries = self.hydrated_entries.clone();
        entries.extend(
            self.log_entries[self.hydrated_entries.len()..]
                .iter()
                .map(|entry| ArtistEntryVm {
                    artist_id: entry.artist_id,
                    name: entry.name.clone(),
                    track_count: entry.track_count,
                    is_provisional: true,
                }),
        );
        entries
    }

    fn loading_state(&self) -> DeferredListStateVm {
        DeferredListStateVm {
            total_entries: self.log_entries.len(),
            hydrated_entries: self.hydrated_entries.len(),
            is_loading: self.hydrated_entries.len() < self.log_entries.len(),
        }
    }
}

/// アプリケーションの中核を担うファサード構造体。
///
/// 主な責務:
/// - 内部モデル（`Library` / `Queue` / `PlaybackModel` 等）の保持
/// - バックグラウンドワーカー（スキャン/デコード/ブラウズローダ）の管理
/// - オーディオデバイスの検出・再設定とバッファのプリフェッチ
/// - 永続化層との入出力（`save_to_store` / `bootstrap_from_store`）
///
/// 設計上、重い処理はワーカスレッドに委譲し、メインスレッドではメッセージをポーリングして
/// 状態遷移を適用することで UI ブロックを回避しています。
pub struct AppFacade {
    pub strings: StringInterner,
    pub library: Library,
    pub queue: Queue,
    pub playlists: PlaylistStore,
    pub playback: PlaybackModel,
    pub scan: ScanModel,
    pub settings: AppConfigFile,
    pub audio_output: BufferedAudioOutput,
    _audio_device: Option<AudioDeviceOutput>,
    device_watcher: Option<DeviceWatcherHandle>,
    current_audio_device_name: Option<String>,
    audio_disconnected: bool,
    #[cfg(test)]
    test_audio_device_present: bool,
    pub artwork_cache: ArtworkCache,
    decoder: Decoder,
    decode_worker: Option<DecodeWorkerHandle>,
    decode_completion_pending: Option<TrackId>,
    scan_worker: Option<ScanWorkerHandle>,
    pub current_tab: TabId,
    selected_album: Option<AlbumId>,
    selected_artist_kind: ArtistKind,
    selected_artist: Option<ArtistId>,
    selected_directory: Option<DirectoryNodeId>,
    browse_cache: BrowseCacheState,
    // persisted indices loaded at startup (may be empty)
    album_index: Vec<crate::core::persistence::AlbumIndexEntry>,
    artist_index: crate::core::persistence::ArtistIndexFile,
    // mapping of artist name -> ArtistId for quick lookup when lazy-loading
    artist_name_to_id: std::collections::HashMap<String, ArtistId>,
    pub browse_loader: Option<BrowseLoaderHandle>,
    pub browse_cache_builder: Option<BrowseCacheBuilderHandle>,
}

impl Default for AppFacade {
    fn default() -> Self {
        // Default 構築: テスト時はデバイス周りをモック的に扱う。
        // - オーディオ出力は `BufferedAudioOutput` を作成
        // - デバイスウォッチャは通常起動（テスト時は無効）
        let mut strings = StringInterner::default();
        let favorite_name = strings.intern("favorite");
        let audio_config = if cfg!(test) {
            AudioOutputConfig::default()
        } else {
            AudioDeviceOutput::preferred_config().unwrap_or_else(AudioOutputConfig::default)
        };
        let audio_output = BufferedAudioOutput::new(audio_config, AUDIO_BLOCK_CAPACITY);
        // capture the current default device name (if any) before attempting to start
        let default_device_name = if cfg!(test) {
            None
        } else {
            cpal::default_host()
                .default_output_device()
                .and_then(|d| d.name().ok())
        };

        let audio_device = if cfg!(test) {
            None
        } else {
            AudioDeviceOutput::try_start(&audio_output).ok()
        };

        let current_audio_device_name_val = audio_device
            .as_ref()
            .and_then(|_| default_device_name.clone());
        audio_output.set_playback_active(false);

        let device_watcher = if cfg!(test) {
            None
        } else {
            Some(DeviceWatcherHandle::spawn(500))
        };

        Self {
            strings,
            library: Library::default(),
            queue: Queue::default(),
            playlists: PlaylistStore::new(favorite_name),
            playback: PlaybackModel::default(),
            scan: ScanModel::default(),
            settings: AppConfigFile::default(),
            audio_output,
            _audio_device: audio_device,
            device_watcher,
            current_audio_device_name: current_audio_device_name_val,
            audio_disconnected: false,
            #[cfg(test)]
            test_audio_device_present: false,
            artwork_cache: ArtworkCache::new(64),
            decoder: Decoder::default(),
            decode_worker: None,
            decode_completion_pending: None,
            scan_worker: None,
            current_tab: TabId::Queue,
            selected_album: None,
            selected_artist_kind: ArtistKind::TrackArtist,
            selected_artist: None,
            selected_directory: None,
            browse_cache: BrowseCacheState::default(),
            album_index: Vec::new(),
            artist_index: crate::core::persistence::ArtistIndexFile::default(),
            artist_name_to_id: std::collections::HashMap::new(),
            browse_loader: None,
            browse_cache_builder: None,
        }
    }
}

impl AppFacade {
    pub fn bootstrap_summary(&self) -> String {
        format!(
            "lrc_player initialized: tab={:?}, queue_entries={}, playlists={}, scan_state={:?}",
            self.current_tab,
            self.queue.entries.len(),
            self.playlists.playlists.len(),
            self.scan.state
        )
    }

    /// 簡易なブートサマリ文字列を返す。デバッグ/起動ログ用。

    pub fn load_demo_state(&mut self) {
        if self.playlists.selected.is_none() {
            self.playlists.selected = Some(self.playlists.favorite_playlist);
        }
    }

    /// デモ状態を読み込む（テストや初回起動用の簡易ヘルパー）

    pub fn save_to_store(&self, store: &JsonStore) -> io::Result<()> {
        // 永続化: Config / State / Playlists / Artwork / Indices を順に保存する。
        // 各保存はベストエフォートで行い、失敗した場合は Err を返す。
        let mut config = self.settings.clone();
        config.include_dirs = self.scan.config.include_dirs.clone();
        config.exclude_dirs = self.scan.config.exclude_dirs.clone();
        store.save_config(&config)?;
        store.save_state(&AppStateFile::capture(
            self.current_tab,
            &self.queue,
            &self.playback,
            &self.library,
        ))?;
        store.save_playlists(&PlaylistsFile::capture(
            &self.playlists,
            &self.library,
            &self.strings,
        ))?;
        // save artwork cache if present
        let _ = store.save_artwork_cache(&self.artwork_cache);

        // Build and save album index
        let album_entries: Vec<AlbumIndexEntry> = self
            .library
            .albums
            .titles
            .iter()
            .enumerate()
            .map(|(index, title_id)| {
                let title = self
                    .strings
                    .resolve(*title_id)
                    .unwrap_or_else(|| "Unknown".to_string());
                let album_artist = self
                    .library
                    .albums
                    .album_artist_ids
                    .get(index)
                    .and_then(|a| a.and_then(|id| self.library.artist_name(id, &self.strings)))
                    .unwrap_or_else(|| "Unknown".to_string());
                let tracks: Vec<String> = self
                    .library
                    .indexes
                    .tracks_by_album
                    .get(index)
                    .map(|v| {
                        v.iter()
                            .filter_map(|track_id| {
                                self.library
                                    .track_key(*track_id)
                                    .map(|k| k.normalized_path.clone())
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                AlbumIndexEntry {
                    title,
                    album_artist,
                    track_count: tracks.len(),
                    tracks,
                }
            })
            .collect();

        let _ = store.save_album_index(&album_entries);

        // Build and save artist index file (track/artists, album_artists, composers)
        let mut track_artists: Vec<ArtistIndexEntry> = Vec::new();
        let mut album_artists: Vec<ArtistIndexEntry> = Vec::new();
        let mut composers: Vec<ArtistIndexEntry> = Vec::new();

        for (index, name_id) in self.library.artists.names.iter().enumerate() {
            let name = self
                .strings
                .resolve(*name_id)
                .unwrap_or_else(|| "Unknown".to_string());

            let track_list: Vec<String> = self
                .library
                .indexes
                .tracks_by_artist
                .get(index)
                .map(|v| {
                    v.iter()
                        .filter_map(|tid| {
                            self.library
                                .track_key(*tid)
                                .map(|k| k.normalized_path.clone())
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !track_list.is_empty() {
                track_artists.push(ArtistIndexEntry {
                    name: name.clone(),
                    track_count: track_list.len(),
                    tracks: track_list.clone(),
                });
            }

            let album_list: Vec<String> = self
                .library
                .indexes
                .tracks_by_album_artist
                .get(index)
                .map(|v| {
                    v.iter()
                        .filter_map(|tid| {
                            self.library
                                .track_key(*tid)
                                .map(|k| k.normalized_path.clone())
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !album_list.is_empty() {
                album_artists.push(ArtistIndexEntry {
                    name: name.clone(),
                    track_count: album_list.len(),
                    tracks: album_list.clone(),
                });
            }

            let composer_list: Vec<String> = self
                .library
                .indexes
                .tracks_by_composer
                .get(index)
                .map(|v| {
                    v.iter()
                        .filter_map(|tid| {
                            self.library
                                .track_key(*tid)
                                .map(|k| k.normalized_path.clone())
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !composer_list.is_empty() {
                composers.push(ArtistIndexEntry {
                    name: name.clone(),
                    track_count: composer_list.len(),
                    tracks: composer_list.clone(),
                });
            }
        }

        let artist_file = ArtistIndexFile {
            track_artists,
            album_artists,
            composers,
        };

        let _ = store.save_artist_index(&artist_file);

        Ok(())
    }

    pub fn bootstrap_from_store(&mut self, store: &JsonStore) -> io::Result<()> {
        // ブートストラップ: 設定を読み込み、設定された include_dirs があれば
        // フルスキャンを走らせてライブラリを構築した後に状態を復元する。
        let config = store.load_config()?;
        self.settings = config.clone();
        self.scan.config.include_dirs = config.include_dirs;
        self.scan.config.exclude_dirs = config.exclude_dirs;

        if !self.scan.config.include_dirs.is_empty() {
            let (library, _) = self.scan.run_full_scan(&mut self.strings);
            self.library = library;
        }

        self.restore_loaded_state(store)
    }

    pub fn restore_from_store(&mut self, store: &JsonStore) -> io::Result<()> {
        // 単純に保存済みの状態を復元する（ライブラリは事前にセットされている想定）
        self.restore_loaded_state(store)
    }

    fn restore_loaded_state(&mut self, store: &JsonStore) -> io::Result<()> {
        // 永続化レイヤからの復元処理:
        // - 設定の反映
        // - キュー/再生状態の復元
        // - プレイリストの復元
        // - アートワークキャッシュやインデックスの読み込み（ベストエフォート）
        let config = store.load_config()?;
        self.settings = config.clone();
        self.scan.config.include_dirs = config.include_dirs;
        self.scan.config.exclude_dirs = config.exclude_dirs;

        let state = store.load_state()?;
        self.current_tab = state.last_tab;
        let restored_queue = state.resolve_queue_track_ids(&self.library);
        self.queue.replace_with_tracks(&restored_queue);
        self.queue.current_index = state
            .current_index
            .filter(|index| (*index as usize) < self.queue.entries.len());
        self.queue.repeat_mode = state.repeat_mode;

        if let Some(track_id) = self.queue.current_track() {
            self.playback
                .restore_track(track_id, state.playback_position_ms);
            let _ = self.prepare_audio(track_id);
        } else {
            self.playback.stop();
        }
        self.sync_audio_transport();

        let playlist_file = store.load_playlists()?;
        let (playlists, favorite_playlist) =
            playlist_file.restore_playlists(&self.library, &mut self.strings);
        self.playlists
            .replace_all(playlists, Some(favorite_playlist), favorite_playlist);

        // attempt to load artwork cache (best-effort)
        if let Ok(cache) = store.load_artwork_cache(64) {
            self.artwork_cache = cache;
        }

        // Attempt to load persisted album/artist indices (best-effort).
        if let Ok(album_index) = store.load_album_index() {
            let artist_index = store.load_artist_index().unwrap_or_default();

            // keep copies of the persisted indices for lazy loading on demand
            self.album_index = album_index.clone();
            self.artist_index = artist_index.clone();

            // Build a mapping of artist name -> ArtistId and pre-populate artist table
            let mut artist_map: std::collections::HashMap<String, crate::core::ids::ArtistId> =
                std::collections::HashMap::new();

            for entry in artist_index
                .track_artists
                .iter()
                .chain(artist_index.album_artists.iter())
                .chain(artist_index.composers.iter())
            {
                if !artist_map.contains_key(&entry.name) {
                    let id = crate::core::ids::ArtistId(self.library.artists.names.len() as u32);
                    let sid = self.strings.intern(entry.name.clone());
                    self.library.artists.names.push(sid);
                    artist_map.insert(entry.name.clone(), id);
                }
            }

            for album in &album_index {
                if !album.album_artist.is_empty() && !artist_map.contains_key(&album.album_artist) {
                    let id = crate::core::ids::ArtistId(self.library.artists.names.len() as u32);
                    let sid = self.strings.intern(album.album_artist.clone());
                    self.library.artists.names.push(sid);
                    artist_map.insert(album.album_artist.clone(), id);
                }
            }

            // Pre-populate albums in the same order as saved index so AlbumId values are stable
            for album in &album_index {
                let title_id = self.strings.intern(album.title.clone());
                let artist_id_opt = artist_map.get(&album.album_artist).copied();
                self.library.albums.titles.push(title_id);
                self.library.albums.album_artist_ids.push(artist_id_opt);
                self.library.albums.representative_artwork.push(None);
            }

            // Initialize empty index vectors sized to the prepopulated counts
            self.library.indexes.tracks_by_album = vec![Vec::new(); self.library.albums.titles.len()];
            self.library.indexes.tracks_by_artist = vec![Vec::new(); self.library.artists.names.len()];
            self.library.indexes.tracks_by_album_artist = vec![Vec::new(); self.library.artists.names.len()];
            self.library.indexes.tracks_by_composer = vec![Vec::new(); self.library.artists.names.len()];

            // Populate browse_cache from the index entries so UI can show lists immediately
            self.browse_cache.albums.reset(
                album_index
                    .iter()
                    .enumerate()
                    .map(|(i, a)| AlbumLogEntry {
                        album_id: crate::core::ids::AlbumId(i as u32),
                        title: a.title.clone(),
                        album_artist: a.album_artist.clone(),
                        track_count: a.track_count,
                    })
                    .collect(),
            );

            self.browse_cache.track_artists.reset(
                artist_index
                    .track_artists
                    .iter()
                    .filter_map(|entry| {
                        artist_map
                            .get(&entry.name)
                            .map(|&id| ArtistLogEntry {
                                artist_id: id,
                                name: entry.name.clone(),
                                track_count: entry.track_count,
                            })
                    })
                    .collect(),
            );

            self.browse_cache.album_artists.reset(
                artist_index
                    .album_artists
                    .iter()
                    .filter_map(|entry| {
                        artist_map
                            .get(&entry.name)
                            .map(|&id| ArtistLogEntry {
                                artist_id: id,
                                name: entry.name.clone(),
                                track_count: entry.track_count,
                            })
                    })
                    .collect(),
            );

            self.browse_cache.composers.reset(
                artist_index
                    .composers
                    .iter()
                    .filter_map(|entry| {
                        artist_map
                            .get(&entry.name)
                            .map(|&id| ArtistLogEntry {
                                artist_id: id,
                                name: entry.name.clone(),
                                track_count: entry.track_count,
                            })
                    })
                    .collect(),
            );

            // keep a copy of the artist name -> id mapping for lazy-load lookups
            self.artist_name_to_id = artist_map;

            // Keep the browse cache revision in sync with library revision so we don't
            // rebuild it prematurely; the actual tracks will be filled in when lazy-loaded.
            self.browse_cache.library_revision = self.library.revision;
        }

        Ok(())
    }

    /// UI からのコマンドを受け取り処理するメインエントリ。
    ///
    /// - 与えられた `AppCommand` を解釈して内部状態を更新する。
    /// - 必要に応じてワーカー起動やデバイス再初期化、永続化トリガを行う。
    /// - 発生した副作用を `AppEvent` の列として返すので、呼び出し元（UI）はそれを使って表示更新を行う。
    pub fn handle_command(&mut self, command: AppCommand) -> Vec<AppEvent> {
        // Entrypoint for command processing: dispatch and side-effect orchestration.
        // Invariants:
        // - Handlers must not perform long-blocking work; heavy tasks should be spawned to worker threads.
        // - Apply model updates before starting workers or emitting events so the UI observes a
        //   consistent state snapshot.
        // - The returned `Vec<AppEvent>` should reflect causal ordering of side effects and be
        //   safe to consume immediately by the UI layer.
        log::info!("handle_command start: {:?}", command);
        let events = match command {
            AppCommand::ClickTrack {
                visible_tracks,
                clicked_index,
                origin,
            } => self.handle_click_track(visible_tracks, clicked_index, origin),
            AppCommand::PlayNext { track_id } => {
                self.queue.insert_after_current(track_id);
                vec![AppEvent::QueueChanged]
            }
            AppCommand::AppendToQueue { track_id } => {
                self.queue.push_track(track_id);
                vec![AppEvent::QueueChanged]
            }
            AppCommand::AddToPlaylist {
                playlist_id,
                track_id,
            } => {
                self.playlists.add_track(playlist_id, track_id);
                vec![AppEvent::PlaylistChanged]
            }
            AppCommand::ToggleFavorite { track_id } => {
                self.playlists.toggle_favorite(track_id);
                vec![AppEvent::PlaylistChanged]
            }
            AppCommand::ShowTrackDetails { track_id } => {
                vec![AppEvent::TrackDetailsRequested(track_id)]
            }
            AppCommand::QueueRemoveByContext { entry_id } => {
                self.queue.remove_entry(entry_id);
                self.sync_playback_from_queue()
            }
            AppCommand::TogglePlayPause => {
                self.playback.toggle_pause();
                self.sync_audio_transport();
                // If playback is starting but we don't have an active audio device,
                // attempt a best-effort restart of the output (preferred or default).
                if matches!(
                    self.playback.state,
                    PlaybackState::Playing | PlaybackState::Buffering
                ) && !self.audio_device_present()
                {
                    if let Some(pref) = self.settings.preferred_output_device.clone() {
                        if let Ok(device) =
                            AudioDeviceOutput::try_start_by_name(&self.audio_output, &pref)
                        {
                            self.set_audio_device(Some(device), Some(pref));
                            self.audio_disconnected = false;
                            self.sync_audio_transport();
                        }
                    } else if let Some(default_name) = cpal::default_host()
                        .default_output_device()
                        .and_then(|d| d.name().ok())
                    {
                        let old_cfg = self.audio_output.config();
                        let target_cfg =
                            AudioDeviceOutput::config_for_name(&default_name).unwrap_or(old_cfg);
                        if target_cfg.sample_rate == old_cfg.sample_rate {
                            if let Ok(device) = AudioDeviceOutput::try_start_by_name(
                                &self.audio_output,
                                &default_name,
                            ) {
                                self.set_audio_device(Some(device), Some(default_name));
                                self.audio_disconnected = false;
                                self.sync_audio_transport();
                            }
                        } else {
                            // recreate buffered output and re-prime
                            self.cancel_decode_worker();
                            let old_block_capacity = self.audio_output.block_capacity();
                            let new_output =
                                BufferedAudioOutput::new(target_cfg, old_block_capacity);
                            if let Ok(device) =
                                AudioDeviceOutput::try_start_by_name(&new_output, &default_name)
                            {
                                self.audio_output = new_output;
                                self.set_audio_device(Some(device), Some(default_name.clone()));
                                if let Some(track_id) = self.playback.current_track {
                                    let _ = self.prepare_audio(track_id);
                                    // after priming, resume audio output
                                    self.sync_audio_transport();
                                }
                            }
                        }
                    }
                }

                vec![AppEvent::Playback(PlaybackEvent::StateChanged(
                    self.playback.state,
                ))]
            }
            AppCommand::SeekBy { delta_ms } => {
                self.playback.seek_by(delta_ms);
                if let Some(track_id) = self.playback.current_track {
                    let _ = self.prepare_audio(track_id);
                }
                self.sync_audio_transport();
                vec![AppEvent::Playback(PlaybackEvent::PositionUpdated {
                    position_ms: self.playback.position_ms,
                    buffered_ms: self.playback.buffered_ms,
                })]
            }
            AppCommand::SeekTo { position_ms } => {
                self.playback.seek_to(position_ms);
                if let Some(track_id) = self.playback.current_track {
                    let _ = self.prepare_audio(track_id);
                }
                self.sync_audio_transport();
                vec![AppEvent::Playback(PlaybackEvent::PositionUpdated {
                    position_ms: self.playback.position_ms,
                    buffered_ms: self.playback.buffered_ms,
                })]
            }
            AppCommand::NextTrack => {
                self.queue.advance();
                self.sync_playback_from_queue()
            }
            AppCommand::PreviousTrack => {
                self.queue.previous();
                self.sync_playback_from_queue()
            }
            AppCommand::SelectTab(tab) => {
                if self.current_tab == TabId::Scan && tab != TabId::Scan {
                    self.scan.reset_if_terminal();
                }
                self.current_tab = tab;
                vec![AppEvent::TabChanged(tab)]
            }
            AppCommand::SelectAlbum(album_id) => {
                let prev = self.selected_album;
                self.selected_album = album_id;
                log::info!("[facade] SelectAlbum: prev={:?} new={:?}", prev, album_id);
                if let Some(aid) = album_id {
                    self.lazy_load_album_tracks(aid);
                }
                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::SelectArtistKind(kind) => {
                self.selected_artist_kind = kind;
                self.selected_artist = None;
                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::SelectArtist(artist_id) => {
                let prev = self.selected_artist;
                self.selected_artist = artist_id;
                log::info!("[facade] SelectArtist: prev={:?} new={:?}", prev, artist_id);
                if let Some(aid) = artist_id {
                    self.lazy_load_artist_tracks(aid, self.selected_artist_kind);
                }
                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::SelectPlaylist(playlist_id) => {
                self.playlists.selected = playlist_id;
                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::SelectDirectory(node_id) => {
                self.selected_directory = node_id;
                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::SetPreferredOutputDevice { device_name } => {
                self.settings.preferred_output_device = device_name.clone();

                // Stop any ongoing decoder so we can re-prime for the new device config.
                self.cancel_decode_worker();

                let old_block_capacity = self.audio_output.block_capacity();
                let old_config = self.audio_output.config();

                // Determine target config from the selected device (if available).
                let target_config = match device_name.as_ref() {
                    Some(name) => AudioDeviceOutput::config_for_name(name).unwrap_or(old_config),
                    None => AudioDeviceOutput::preferred_config().unwrap_or(old_config),
                };

                let new_output = BufferedAudioOutput::new(target_config, old_block_capacity);

                // Try to start the device using the new buffered output. If successful,
                // replace the current audio_output and re-prime playback. If not,
                // keep the existing output and attempt a best-effort restart.
                let started_device = match device_name.as_ref() {
                    Some(name) => AudioDeviceOutput::try_start_by_name(&new_output, name).ok(),
                    None => AudioDeviceOutput::try_start(&new_output).ok(),
                };

                if let Some(device) = started_device {
                    self.audio_output = new_output;
                    self.set_audio_device(
                        Some(device),
                        device_name.clone().or_else(|| {
                            cpal::default_host()
                                .default_output_device()
                                .and_then(|d| d.name().ok())
                        }),
                    );

                    // Re-prepare audio for the currently selected track so decoder will
                    // resample to the device's sample rate, then resume audio output
                    if let Some(track_id) = self.playback.current_track {
                        let _ = self.prepare_audio(track_id);
                    }
                    if matches!(
                        self.playback.state,
                        PlaybackState::Playing | PlaybackState::Buffering
                    ) {
                        self.sync_audio_transport();
                    }
                } else {
                    // Could not start the new device. Try to restart using the old output
                    // (best-effort) and keep current audio_output unchanged.
                    let fallback = match device_name.as_ref() {
                        Some(name) => {
                            AudioDeviceOutput::try_start_by_name(&self.audio_output, name).ok()
                        }
                        None => AudioDeviceOutput::try_start(&self.audio_output).ok(),
                    };
                    self.set_audio_device(
                        fallback,
                        device_name.clone().or_else(|| {
                            cpal::default_host()
                                .default_output_device()
                                .and_then(|d| d.name().ok())
                        }),
                    );
                    // update current device name if fallback succeeded (test helper will set name)
                    if self.audio_device_present() {
                        // if playback was active, prime and resume audio
                        if matches!(
                            self.playback.state,
                            PlaybackState::Playing | PlaybackState::Buffering
                        ) {
                            if let Some(track_id) = self.playback.current_track {
                                let _ = self.prepare_audio(track_id);
                            }
                            self.sync_audio_transport();
                        }
                    }
                }

                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::SetDebugMode { enabled } => {
                self.settings.debug_mode = enabled;
                if enabled {
                    log::set_max_level(log::LevelFilter::Debug);
                    log::info!("[facade] debug mode enabled");
                } else {
                    log::set_max_level(log::LevelFilter::Info);
                    log::info!("[facade] debug mode disabled");
                }
                vec![AppEvent::SettingsChanged]
            }
            AppCommand::SetLogFilters { filters } => {
                self.settings.log_filters = filters.clone();
                // update runtime logger filters
                crate::logging::set_log_filters(filters.clone());
                log::info!("[facade] log filters set: {:?}", self.settings.log_filters);
                vec![AppEvent::SettingsChanged]
            }
            AppCommand::Scan(scan_command) => self.handle_scan_command(scan_command),
            AppCommand::CreatePlaylist { name } => {
                let name_id = self.strings.intern(name);
                let _ = self.playlists.create_user_playlist(name_id);
                vec![AppEvent::PlaylistChanged]
            }
        };
        log::info!("handle_command end: produced {} events", events.len());
        // Preserve causal ordering: model mutations applied above, events reflect those changes.
        events
    }

    /// 現在の内部状態から UI 描画用の `AppViewModel` を組み立てて返す。
    ///
    /// - 軽量な DTO を返すために必要なデータのみを抽出する。
    /// - 呼び出し中に `ensure_browse_cache_current()` をトリガして、ブラウズキャッシュのバックグラウンド構築を開始する。
    /// - UI フレーム中に頻繁に呼ばれることを想定しているため、コストを抑えた実装である。
    pub fn view_model(&mut self) -> AppViewModel {
        // assemble and return a lightweight view model for the UI
        // Capture selection snapshot to detect unexpected overwrites
        let prev_selected_album = self.selected_album;
        let prev_selected_artist = self.selected_artist;

        self.ensure_browse_cache_current();

        if prev_selected_album != self.selected_album || prev_selected_artist != self.selected_artist {
            log::warn!(
                "[facade] selection_overwrite: prev_album={:?} prev_artist={:?} new_album={:?} new_artist={:?}",
                prev_selected_album,
                prev_selected_artist,
                self.selected_album,
                self.selected_artist
            );
            eprintln!(
                "selection_overwrite: prev_album={:?} prev_artist={:?} new_album={:?} new_artist={:?}",
                prev_selected_album,
                prev_selected_artist,
                self.selected_album,
                self.selected_artist
            );
        }

        AppViewModel {
            current_tab: self.current_tab,
            queue: self.queue_tab_vm(),
            play: self.play_tab_vm(),
            album: self.album_tab_vm(),
            artist: self.artist_tab_vm(),
            playlist: self.playlist_tab_vm(),
            library: self.library_tab_vm(),
            scan: self.scan_tab_vm(),
            settings: self.settings_tab_vm(),
        }
    }

    fn audio_device_present(&self) -> bool {
        #[cfg(test)]
        {
            self._audio_device.is_some() || self.test_audio_device_present
        }
        #[cfg(not(test))]
        {
            self._audio_device.is_some()
        }
    }

    fn set_audio_device(&mut self, device: Option<AudioDeviceOutput>, name: Option<String>) {
        #[cfg(test)]
        {
            // in tests we don't construct real AudioDeviceOutput streams; track presence with flag
            self._audio_device = device;
            self.test_audio_device_present = self._audio_device.is_some();
            self.current_audio_device_name = name;
        }
        #[cfg(not(test))]
        {
            self._audio_device = device;
            self.current_audio_device_name = name;
        }
    }

    #[cfg(test)]
    pub fn set_test_audio_device_present(&mut self, present: bool, name: Option<String>) {
        self.test_audio_device_present = present;
        if present {
            self.current_audio_device_name = name;
        } else {
            self.current_audio_device_name = None;
        }
    }

    /// バックグラウンドワーカーをポーリングしてメッセージを処理する。
    ///
    /// - デバイスのホットプラグ/デフォルトデバイス変化の検出と処理
    /// - スキャン/デコード/ブラウズローダーからのメッセージハンドリング
    /// - ライブラリへのバッチ upsert を制限付きで実行し、UI に更新イベントを返す
    ///
    /// この関数はメインスレッドから頻繁に呼ばれることを想定しており、長時間ブロックしないよう
    /// 1 回の呼び出しで処理するアップサート数やバッチサイズに上限を設けています。
    pub fn poll_background_tasks(&mut self) -> Vec<AppEvent> {
        let mut events = Vec::new();
        let mut finished = None;
        let poll_start = Instant::now();
        let mut total_upserts = 0usize;

        // Hotplug detection: prefer background watcher messages; fall back to inline polling if not available.
        let mut handled_hotplug = false;
        if let Some(watcher) = self.device_watcher.as_ref() {
            handled_hotplug = true;
            for message in watcher.drain_messages() {
                match message {
                    crate::audio::device_watcher::DeviceWatcherMessage::DevicesChanged {
                        devices,
                        default,
                    } => {
                                if let Some(pref) = self.settings.preferred_output_device.clone() {
                                    let present = devices.iter().any(|d| d == &pref);
                                    if !present && self.audio_device_present() {
                                        // device disappeared: don't change playback state, just stop audio output
                                        self.set_audio_device(None, None);
                                self.audio_output.set_playback_active(false);
                                self.audio_disconnected = true;
                                events.push(AppEvent::Playback(PlaybackEvent::DeviceUnderrun));
                            } else if present && !self.audio_device_present() {
                                // device reappeared, try to restart output (do not auto-resume playback)
                                if let Ok(device) =
                                    AudioDeviceOutput::try_start_by_name(&self.audio_output, &pref)
                                {
                                    self.set_audio_device(Some(device), Some(pref.clone()));
                                    self.audio_disconnected = false;
                                    // If playback was active, re-prime playback and resume audio output
                                    if matches!(
                                        self.playback.state,
                                        PlaybackState::Playing | PlaybackState::Buffering
                                    ) {
                                        if let Some(track_id) = self.playback.current_track {
                                            let _ = self.prepare_audio(track_id);
                                        }
                                        self.sync_audio_transport();
                                    }
                                    events.push(AppEvent::Playback(PlaybackEvent::StateChanged(
                                        self.playback.state,
                                    )));
                                }
                            }
                        } else {
                            // No preferred device configured: handle changes in the system default device.
                            let default_name = default;

                            // If default changed and we previously had a device, attempt to switch.
                            if default_name != self.current_audio_device_name {
                                // If no default now, clear existing device.
                                if default_name.is_none() {
                                    if self.audio_device_present() {
                                        self.set_audio_device(None, None);
                                        self.audio_output.set_playback_active(false);
                                        self.audio_disconnected = true;
                                        self.current_audio_device_name = None;
                                        events.push(AppEvent::Playback(
                                            PlaybackEvent::DeviceUnderrun,
                                        ));
                                    }
                                } else if let Some(new_name) = default_name.clone() {
                                    // Attempt to negotiate config and start the new default device.
                                    let old_config = self.audio_output.config();
                                    let target_cfg = AudioDeviceOutput::config_for_name(&new_name)
                                        .unwrap_or(old_config);

                                    if target_cfg.sample_rate == old_config.sample_rate {
                                        // can reuse existing BufferedAudioOutput
                                        if let Ok(device) = AudioDeviceOutput::try_start_by_name(
                                            &self.audio_output,
                                            &new_name,
                                        ) {
                                            self.set_audio_device(Some(device), Some(new_name.clone()));
                                            self.audio_disconnected = false;
                                            // resume audio output if playback is active
                                            if matches!(
                                                self.playback.state,
                                                PlaybackState::Playing | PlaybackState::Buffering
                                            ) {
                                                self.sync_audio_transport();
                                            }
                                            events.push(AppEvent::Playback(
                                                PlaybackEvent::StateChanged(self.playback.state),
                                            ));
                                        }
                                    } else {
                                        // sample-rate changed: recreate buffered output and re-prime from current position
                                        self.cancel_decode_worker();
                                        let old_block_capacity = self.audio_output.block_capacity();
                                        let new_output = BufferedAudioOutput::new(
                                            target_cfg,
                                            old_block_capacity,
                                        );
                                        if let Ok(device) = AudioDeviceOutput::try_start_by_name(
                                            &new_output,
                                            &new_name,
                                        ) {
                                            self.audio_output = new_output;
                                            self.set_audio_device(Some(device), Some(new_name.clone()));
                                            if let Some(track_id) = self.playback.current_track {
                                                let _ = self.prepare_audio(track_id);
                                            }
                                            // after priming, resume audio if playback was active
                                            if matches!(
                                                self.playback.state,
                                                PlaybackState::Playing | PlaybackState::Buffering
                                            ) {
                                                self.sync_audio_transport();
                                            }
                                        } else {
                                            // fallback: try start using existing audio_output
                                            let fallback = AudioDeviceOutput::try_start_by_name(
                                                &self.audio_output,
                                                &new_name,
                                            )
                                            .ok();
                                            if fallback.is_some() {
                                                self.set_audio_device(fallback, Some(new_name.clone()));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if !handled_hotplug {
            // Legacy/poll fallback (keeps previous behavior when watcher isn't running)
            if let Some(pref) = self.settings.preferred_output_device.clone() {
                let available = AudioDeviceOutput::list_output_devices();
                let present = available.iter().any(|d| d == &pref);
                if !present && self.audio_device_present() {
                    // device disappeared
                    self.set_audio_device(None, None);
                    if matches!(
                        self.playback.state,
                        PlaybackState::Playing | PlaybackState::Buffering
                    ) {
                        self.playback.state = PlaybackState::Paused;
                    }
                    self.audio_output.set_playback_active(false);
                    self.audio_disconnected = true;
                    events.push(AppEvent::Playback(PlaybackEvent::DeviceUnderrun));
                } else if present && !self.audio_device_present() {
                    // device reappeared, try to restart output (do not auto-resume playback)
                    if let Ok(device) =
                        AudioDeviceOutput::try_start_by_name(&self.audio_output, &pref)
                    {
                        self.set_audio_device(Some(device), Some(pref.clone()));
                        self.audio_disconnected = false;
                        // resume audio output if playback is active
                        if matches!(
                            self.playback.state,
                            PlaybackState::Playing | PlaybackState::Buffering
                        ) {
                            if let Some(track_id) = self.playback.current_track {
                                let _ = self.prepare_audio(track_id);
                            }
                            self.sync_audio_transport();
                        }
                        events.push(AppEvent::Playback(PlaybackEvent::StateChanged(
                            self.playback.state,
                        )));
                    }
                }
            }
        }

        if let Some(worker) = self.scan_worker.as_ref() {
            for message in worker.drain_messages() {
                match message {
                    ScanWorkerMessage::Progress(progress) => {
                        self.scan.progress = progress;
                    }
                    ScanWorkerMessage::Finished(outcome) => {
                        finished = Some(outcome);
                    }
                }
            }
        }

        if let Some(outcome) = finished {
            self.complete_scan(outcome);
            self.scan_worker = None;
            events.push(AppEvent::ScanStateChanged(self.scan.state));
        }

        let mut decode_failed_for = None;
        let mut decode_worker_finished = false;
        if let Some(worker) = self.decode_worker.as_ref() {
            for message in worker.drain_messages() {
                match message {
                    DecodeWorkerMessage::SourceExhausted { track_id } => {
                        self.decode_completion_pending = Some(track_id);
                        decode_worker_finished = true;
                    }
                    DecodeWorkerMessage::Failed { track_id, .. } => {
                        decode_failed_for = Some(track_id);
                        decode_worker_finished = true;
                    }
                }
            }
        }
        if decode_worker_finished {
            self.decode_worker = None;
        }

        if let Some(track_id) = decode_failed_for {
            if self.playback.current_track == Some(track_id)
                && self.audio_output.pending_samples() == 0
            {
                events.extend(self.advance_after_track_end());
            }
        }

        if let Some(track_id) = self.decode_completion_pending {
            if self.playback.current_track == Some(track_id)
                && self.audio_output.pending_samples() == 0
            {
                self.decode_completion_pending = None;
                events.extend(self.advance_after_track_end());
            }
        }

        // Drain messages from the browse loader (background lazy-load of persisted index paths)
        let mut browse_finished = false;
        let mut browse_missing_paths: Vec<String> = Vec::new();
        let mut library_changed = false;
        // Limit total upserts per poll to avoid long blocking periods.
        // Increased for investigation to amortize directory/index rebuilds.
        const MAX_UPSERTS_PER_POLL: usize = 1024;
        if let Some(loader) = self.browse_loader.as_ref() {
            // Drain into a Vec first so we can report how many messages were available
            let drained_msgs = loader.drain_messages();
            let drained_count = drained_msgs.len();
            let mut upserts_done = 0usize;
            let mut batch: Vec<crate::core::library::TrackInput> = Vec::new();

            'drain_loop: for message in drained_msgs.into_iter() {
                match message {
                    BrowseLoaderMessage::Track(track_input) => {
                        batch.push(track_input);
                        upserts_done += 1;
                        if upserts_done >= MAX_UPSERTS_PER_POLL {
                            break 'drain_loop;
                        }
                    }
                    BrowseLoaderMessage::TrackBatch(vec) => {
                        for track_input in vec {
                            batch.push(track_input);
                            upserts_done += 1;
                            if upserts_done >= MAX_UPSERTS_PER_POLL {
                                break 'drain_loop;
                            }
                        }
                    }
                    BrowseLoaderMessage::Finished { missing_paths } => {
                        browse_finished = true;
                        browse_missing_paths = missing_paths;
                        // continue draining to collect other finished messages
                    }
                }
            }

            if !batch.is_empty() {
                let batch_len = batch.len();
                let upsert_start = Instant::now();
                let _ids = self.library.upsert_tracks_batch(&mut self.strings, batch);
                let upsert_dur = upsert_start.elapsed();
                total_upserts = upserts_done;
                library_changed = true;
                log::info!(
                    "poll_background_upsert: drained_msgs={} collected_inputs={} upserts={} upsert_ms={}",
                    drained_count,
                    batch_len,
                    upserts_done,
                    upsert_dur.as_millis()
                );
            } else {
                log::info!("poll_background: drained_msgs={} no_upserts", drained_count);
            }
        }

        if library_changed {
            // Defer actual rebuild — signal UI to refresh; `view_model()` will
            // call `ensure_browse_cache_current()` once during the next frame.
            events.push(AppEvent::TabChanged(self.current_tab));
        }

        if browse_finished {
            // keep the loader cleared and refresh UI
            self.browse_loader = None;
            events.push(AppEvent::TabChanged(self.current_tab));


        let poll_dur = poll_start.elapsed();
        log::info!(
            "poll_background_tasks: duration_ms={} upserts={}",
            poll_dur.as_millis(),
            total_upserts
        );
            // If there were missing paths, attempt a targeted re-scan of likely roots
            if !browse_missing_paths.is_empty() {
                // compute candidate roots: prefer configured include dirs that contain the missing path,
                // otherwise use the nearest existing ancestor directory
                let mut roots: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
                for missing in browse_missing_paths.iter() {
                    let p = Path::new(missing);
                    if let Some(mut ancestor) = p.parent() {
                        // climb until we find an existing directory or run out
                        while !ancestor.exists() {
                            if let Some(parent) = ancestor.parent() {
                                ancestor = parent;
                            } else {
                                break;
                            }
                        }

                        // pick the include dir that is an ancestor of this path if any
                        let mut added = false;
                        for include in &self.scan.config.include_dirs {
                            if ancestor.starts_with(include) {
                                roots.insert(include.clone());
                                added = true;
                                break;
                            }
                        }
                        if !added && ancestor.exists() {
                            roots.insert(ancestor.to_path_buf());
                        }
                    }
                }

                if roots.is_empty() {
                    // fallback to full scan if we couldn't locate candidate roots
                    if self.scan_worker.is_none() {
                        events.extend(self.handle_scan_command(ScanCommand::StartFullScan));
                    }
                } else {
                    // spawn targeted scan for the computed roots
                    if self.scan_worker.is_none() {
                        self.scan.apply(ScanCommand::StartFullScan);
                        let roots_vec: Vec<PathBuf> = roots.into_iter().collect();
                        self.scan_worker = Some(ScanWorkerHandle::spawn_targeted(roots_vec, self.scan.config.exclude_dirs.clone()));
                    }
                }
            }
        }

        self.hydrate_browse_lists();

        events
    }

    fn handle_click_track(
        &mut self,
        visible_tracks: Vec<TrackId>,
        clicked_index: usize,
        origin: TrackListOrigin,
    ) -> Vec<AppEvent> {
        if clicked_index >= visible_tracks.len() {
            return Vec::new();
        }

        match origin {
            TrackListOrigin::Queue => {
                self.queue.set_current_index(clicked_index);
            }
            _ => {
                self.queue.replace_with_tracks(&visible_tracks);
                self.queue.set_current_index(clicked_index);
            }
        }

        self.sync_playback_from_queue()
    }

    fn handle_scan_command(&mut self, command: ScanCommand) -> Vec<AppEvent> {
        match command {
            ScanCommand::StartFullScan => {
                if self.scan_worker.is_none() {
                    self.scan.apply(ScanCommand::StartFullScan);
                    self.scan_worker = Some(ScanWorkerHandle::spawn(self.scan.config.clone()));
                }
                vec![AppEvent::ScanStateChanged(self.scan.state)]
            }
            ScanCommand::Stop => {
                if let Some(worker) = self.scan_worker.as_ref() {
                    worker.cancel();
                }
                self.scan.apply(ScanCommand::Stop);
                vec![AppEvent::ScanStateChanged(self.scan.state)]
            }
            other => {
                self.scan.apply(other);
                vec![AppEvent::ScanStateChanged(self.scan.state)]
            }
        }
    }

    fn sync_playback_from_queue(&mut self) -> Vec<AppEvent> {
        self.cancel_decode_worker();

        let queue_len = self.queue.entries.len();
        if queue_len == 0 {
            self.playback.stop();
            self.sync_audio_transport();
            return vec![
                AppEvent::QueueChanged,
                AppEvent::Playback(PlaybackEvent::TrackChanged(None)),
                AppEvent::Playback(PlaybackEvent::StateChanged(self.playback.state)),
            ];
        }

        let original_index = self.queue.current_index;
        for attempt in 0..queue_len {
            let Some(track_id) = self.queue.current_track() else {
                break;
            };

            self.playback.play_track(track_id);
            if self.prepare_audio(track_id).is_ok() && self.audio_output.buffered_blocks() > 0 {
                self.sync_audio_transport();
                return vec![
                    AppEvent::QueueChanged,
                    AppEvent::Playback(PlaybackEvent::TrackChanged(Some(track_id))),
                    AppEvent::Playback(PlaybackEvent::StateChanged(self.playback.state)),
                    AppEvent::Playback(PlaybackEvent::PositionUpdated {
                        position_ms: self.playback.position_ms,
                        buffered_ms: self.playback.buffered_ms,
                    }),
                ];
            }

            if attempt + 1 < queue_len {
                self.queue.advance();
            }
        }

        self.queue.current_index = original_index;
        self.audio_output.clear();
        self.playback.stop();
        self.sync_audio_transport();
        vec![
            AppEvent::QueueChanged,
            AppEvent::Playback(PlaybackEvent::TrackChanged(None)),
            AppEvent::Playback(PlaybackEvent::StateChanged(self.playback.state)),
        ]
    }

    fn prepare_audio(&mut self, track_id: TrackId) -> io::Result<()> {
        // Prepare audio pipeline for the specified `track_id`.
        //
        // - 停止中のデコードワーカーをキャンセルし、出力バッファをクリア
        // - ライブラリから `DecodedSource` を取得し、デコーダで初期ブロックを生成してプリフェッチ
        // - 必要に応じてシーク位置に応じたスキップブロック数を計算してワーカーを起動
        self.cancel_decode_worker();
        self.audio_output.clear();
        let Some(source) = DecodedSource::from_library(track_id, &self.library) else {
            self.playback.buffered_ms = 0;
            return Ok(());
        };
        let cfg = self.audio_output.config();
        let frames_per_block = cfg.frames_per_buffer as u64;
        let sample_rate = cfg.sample_rate as u64;
        let desired_position_ms = self.playback.position_ms;

        if desired_position_ms > 0 {
            let skip_frames = desired_position_ms.saturating_mul(sample_rate) / 1000u64;
            let skip_blocks = (skip_frames / frames_per_block) as usize;

            // Decode entire file to obtain blocks starting at the desired position,
            // then prime the output with a small number of blocks from that position.
            let all_blocks = self.decoder.decode_all_blocks(&source, cfg)?;
            let start = skip_blocks.min(all_blocks.len());
            let prime_blocks: Vec<_> = all_blocks[start..]
                .iter()
                .take(INITIAL_DECODE_PRIME_BLOCKS.min(self.audio_output.block_capacity()))
                .cloned()
                .collect();

            let primed = self.audio_output.try_enqueue_many(prime_blocks);
            let worker_skip = skip_blocks.saturating_add(primed);
            self.decode_worker = Some(DecodeWorkerHandle::spawn(
                source,
                cfg,
                self.audio_output.clone(),
                worker_skip,
            ));
        } else {
            let initial_blocks = self.decoder.decode_blocks(
                &source,
                cfg,
                INITIAL_DECODE_PRIME_BLOCKS.min(self.audio_output.block_capacity()),
            )?;
            let initial_count = self.audio_output.try_enqueue_many(initial_blocks);
            self.decode_worker = Some(DecodeWorkerHandle::spawn(
                source,
                cfg,
                self.audio_output.clone(),
                initial_count,
            ));
        }
        self.decode_completion_pending = None;
        self.playback.buffered_ms = self.audio_output.buffered_ms();
        Ok(())
    }

    fn cancel_decode_worker(&mut self) {
        if let Some(worker) = self.decode_worker.as_ref() {
            worker.cancel();
        }
        self.decode_worker = None;
        self.decode_completion_pending = None;
    }

    fn advance_after_track_end(&mut self) -> Vec<AppEvent> {
        self.queue.advance();
        self.sync_playback_from_queue()
    }

    fn sync_audio_transport(&self) {
        let active = matches!(
            self.playback.state,
            PlaybackState::Playing | PlaybackState::Buffering
        );
        self.audio_output.set_playback_active(active);
    }

    fn complete_scan(&mut self, outcome: ScanOutcome) {
        self.scan.progress = outcome.progress.clone();
        self.scan.finish_with(outcome.result);
        if outcome.targeted {
            // Batch upserts for targeted scans to amortize rebuilds.
            let inputs = outcome.tracks;
            let _ids = self.library.upsert_tracks_batch(&mut self.strings, inputs.into_iter());
        } else {
            // Preserve current selection names so we can re-resolve after rebuilding
            let preserved_album = self
                .selected_album
                .and_then(|aid| self.library.album_title(aid, &self.strings));
            let preserved_artist = self
                .selected_artist
                .and_then(|aid| self.library.artist_name(aid, &self.strings));

            let new_library = build_library_from_track_inputs(&mut self.strings, outcome.tracks);

            // Attempt to remap preserved selections into the new library by name
            let mut new_selected_album: Option<AlbumId> = None;
            if let Some(album_name) = preserved_album {
                for (i, title_id) in new_library.albums.titles.iter().enumerate() {
                    if let Some(title) = self.strings.resolve(*title_id) {
                        if title == album_name {
                            new_selected_album = Some(AlbumId(i as u32));
                            break;
                        }
                    }
                }
            }

            let mut new_selected_artist: Option<ArtistId> = None;
            if let Some(artist_name) = preserved_artist {
                for (i, name_id) in new_library.artists.names.iter().enumerate() {
                    if let Some(name) = self.strings.resolve(*name_id) {
                        if name == artist_name {
                            new_selected_artist = Some(ArtistId(i as u32));
                            break;
                        }
                    }
                }
            }

            self.library = new_library;
            log::info!(
                "[facade] restore bootstrap selected_album: prev={:?} new={:?}, selected_artist: prev={:?} new={:?}",
                self.selected_album,
                new_selected_album,
                self.selected_artist,
                new_selected_artist
            );
            self.selected_album = new_selected_album;
            self.selected_artist = new_selected_artist;
        }
        self.ensure_browse_cache_current();
    }

    fn ensure_browse_cache_current(&mut self) {
        // If already up-to-date, nothing to do.
        if self.browse_cache.library_revision == self.library.revision {
            return;
        }

        // If a builder is already running, try to drain its result. If it completed
        // with a cache matching the current library revision, apply an atomic swap.
        if let Some(builder) = self.browse_cache_builder.as_ref() {
            let msgs = builder.drain_messages();
            if !msgs.is_empty() {
                // take first completed state that matches current revision
                for state in msgs {
                    if state.library_revision == self.library.revision {
                        self.browse_cache = state;
                        log::info!(
                            "ensure_browse_cache_current: swapped browse cache rev={}",
                            self.browse_cache.library_revision
                        );
                        break;
                    }
                }
                // builder finished (or delivered a result) — clear it
                self.browse_cache_builder = None;
                return;
            }

            // still building; if builder target differs from current revision, cancel and restart
            if builder.target_revision() != self.library.revision {
                builder.cancel();
                self.browse_cache_builder = None;
            } else {
                // builder is producing the right revision; defer swap until it completes
                return;
            }
        }

        // No builder running: spawn one using cloned library + strings so the heavy
        // work occurs off the UI thread and we can atomically swap when ready.
        let lib_clone = self.library.clone();
        let strings_clone = self.strings.clone();
        self.browse_cache_builder = Some(BrowseCacheBuilderHandle::spawn(lib_clone, strings_clone));
        log::info!(
            "ensure_browse_cache_current: spawned background builder for rev={}",
            self.library.revision
        );
    }

    fn hydrate_browse_lists(&mut self) {
        self.browse_cache.albums.hydrate_next_batch();
        self.browse_cache.track_artists.hydrate_next_batch();
        self.browse_cache.album_artists.hydrate_next_batch();
        self.browse_cache.composers.hydrate_next_batch();
    }

    fn lazy_load_album_tracks(&mut self, album_id: AlbumId) {
        let idx = album_id.0 as usize;
        if idx >= self.album_index.len() {
            return;
        }

        // If already have tracks in library for this album, skip.
        if self
            .library
            .indexes
            .tracks_by_album
            .get(idx)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            return;
        }

        // cancel any existing loader and spawn a new background loader for this album
        if let Some(loader) = self.browse_loader.as_ref() {
            loader.cancel();
        }
        let paths: Vec<String> = self.album_index[idx].tracks.clone();
        if paths.is_empty() {
            return;
        }
        self.browse_loader = Some(BrowseLoaderHandle::spawn(paths));
    }

    fn lazy_load_artist_tracks(&mut self, artist_id: ArtistId, kind: ArtistKind) {
        let entries = match kind {
            ArtistKind::TrackArtist => &self.artist_index.track_artists,
            ArtistKind::AlbumArtist => &self.artist_index.album_artists,
            ArtistKind::Composer => &self.artist_index.composers,
        };

        // find the persisted artist entry and spawn background loader for its paths
        for entry in entries.iter() {
            if let Some(&id) = self.artist_name_to_id.get(&entry.name) {
                if id == artist_id {
                    if let Some(loader) = self.browse_loader.as_ref() {
                        loader.cancel();
                    }
                    let paths: Vec<String> = entry.tracks.clone();
                    if !paths.is_empty() {
                        self.browse_loader = Some(BrowseLoaderHandle::spawn(paths));
                    }
                    break;
                }
            }
        }
    }

    fn artist_log_entries_for(&self, kind: ArtistKind) -> Vec<ArtistLogEntry> {
        let source: &Vec<Vec<TrackId>> = match kind {
            ArtistKind::TrackArtist => &self.library.indexes.tracks_by_artist,
            ArtistKind::AlbumArtist => &self.library.indexes.tracks_by_album_artist,
            ArtistKind::Composer => &self.library.indexes.tracks_by_composer,
        };

        self.library
            .artists
            .names
            .iter()
            .enumerate()
            .filter_map(|(index, name_id)| {
                let track_count = source.get(index).map_or(0, Vec::len);
                if track_count == 0 {
                    return None;
                }
                Some(ArtistLogEntry {
                    artist_id: ArtistId(index as u32),
                    name: self
                        .strings
                        .resolve(*name_id)
                        .unwrap_or_else(|| "Unknown".to_string()),
                    track_count,
                })
            })
            .collect()
    }

    fn queue_tab_vm(&self) -> QueueTabVm {
        QueueTabVm {
            rows: self
                .queue
                .entries
                .iter()
                .enumerate()
                .map(|(index, entry)| self.track_row_vm(entry.track_id, Some(index)))
                .collect(),
            repeat_mode: self.queue.repeat_mode,
        }
    }

    fn play_tab_vm(&self) -> PlayTabVm {
        PlayTabVm {
            current_track: self
                .playback
                .current_track
                .map(|track_id| self.track_row_vm(track_id, None)),
            playback_state: self.playback.state,
            position_ms: self.playback.position_ms,
        }
    }

    fn album_tab_vm(&self) -> AlbumTabVm {
        let visible_tracks = self
            .selected_album
            .map(|album_id| {
                self.track_rows_for_tracks(self.library.visible_tracks_for_album(album_id))
            })
            .unwrap_or_default();

        // Compute per-album track hydration progress
        let (tracks_loading, tracks_loaded, tracks_total) = if let Some(album_id) = self.selected_album {
            let idx = album_id.0 as usize;
            let total = self
                .album_index
                .get(idx)
                .map(|a| a.tracks.len())
                .or_else(|| self.library.indexes.tracks_by_album.get(idx).map(|v| v.len()))
                .unwrap_or(0);
            let loaded = self
                .library
                .indexes
                .tracks_by_album
                .get(idx)
                .map(|v| v.len())
                .unwrap_or(0);
            (self.browse_loader.is_some() && loaded < total, loaded, total)
        } else {
            (false, 0, 0)
        };

        AlbumTabVm {
            albums: self.browse_cache.albums.visible_entries(),
            expanded_album: self.selected_album,
            visible_tracks,
            loading: self.browse_cache.albums.loading_state(),
            tracks_loading,
            tracks_loaded,
            tracks_total,
        }
    }

    fn artist_tab_vm(&self) -> ArtistTabVm {
        let visible_tracks = self
            .selected_artist
            .map(|artist_id| {
                self.track_rows_for_tracks(
                    self.library
                        .visible_tracks_for_artist(self.selected_artist_kind, artist_id),
                )
            })
            .unwrap_or_default();

        let (artists, loading) = match self.selected_artist_kind {
            ArtistKind::TrackArtist => (
                self.browse_cache.track_artists.visible_entries(),
                self.browse_cache.track_artists.loading_state(),
            ),
            ArtistKind::AlbumArtist => (
                self.browse_cache.album_artists.visible_entries(),
                self.browse_cache.album_artists.loading_state(),
            ),
            ArtistKind::Composer => (
                self.browse_cache.composers.visible_entries(),
                self.browse_cache.composers.loading_state(),
            ),
        };

        // Compute per-artist track hydration progress
        let (tracks_loading, tracks_loaded, tracks_total) = if let Some(artist_id) = self.selected_artist {
            let idx = artist_id.0 as usize;
            let loaded = self
                .library
                .indexes
                .tracks_by_artist
                .get(idx)
                .map(|v| v.len())
                .unwrap_or(0);
            // try to resolve total from persisted index entries; fallback to library index
            let total = match self.selected_artist_kind {
                ArtistKind::TrackArtist => self
                    .artist_index
                    .track_artists
                    .iter()
                    .find(|e| self.library.artist_name(artist_id, &self.strings) == Some(e.name.clone()))
                    .map(|e| e.tracks.len())
                    .or_else(|| self.library.indexes.tracks_by_artist.get(idx).map(|v| v.len()))
                    .unwrap_or(0),
                ArtistKind::AlbumArtist => self
                    .artist_index
                    .album_artists
                    .iter()
                    .find(|e| self.library.artist_name(artist_id, &self.strings) == Some(e.name.clone()))
                    .map(|e| e.tracks.len())
                    .or_else(|| self.library.indexes.tracks_by_album_artist.get(idx).map(|v| v.len()))
                    .unwrap_or(0),
                ArtistKind::Composer => self
                    .artist_index
                    .composers
                    .iter()
                    .find(|e| self.library.artist_name(artist_id, &self.strings) == Some(e.name.clone()))
                    .map(|e| e.tracks.len())
                    .or_else(|| self.library.indexes.tracks_by_composer.get(idx).map(|v| v.len()))
                    .unwrap_or(0),
            };

            (self.browse_loader.is_some() && loaded < total, loaded, total)
        } else {
            (false, 0, 0)
        };

        ArtistTabVm {
            kind: self.selected_artist_kind,
            artists,
            selected_artist: self.selected_artist,
            visible_tracks,
            loading,
            tracks_loading,
            tracks_loaded,
            tracks_total,
        }
    }

    fn playlist_tab_vm(&self) -> PlaylistTabVm {
        let playlists = self
            .playlists
            .playlists
            .iter()
            .map(|playlist| PlaylistEntryVm {
                playlist_id: playlist.id,
                name: self
                    .strings
                    .resolve(playlist.name)
                    .unwrap_or_else(|| format!("playlist-{}", playlist.id.0)),
                track_count: playlist.tracks.len(),
                is_system: playlist.kind == PlaylistKind::SystemFavorite,
            })
            .collect();

        let visible_tracks = self
            .playlists
            .selected
            .and_then(|selected| {
                self.playlists
                    .playlists
                    .iter()
                    .find(|playlist| playlist.id == selected)
                    .map(|playlist| self.track_rows_for_tracks(&playlist.tracks))
            })
            .unwrap_or_default();

        PlaylistTabVm {
            playlists,
            selected_playlist: self.playlists.selected,
            visible_tracks,
        }
    }

    fn library_tab_vm(&self) -> LibraryTabVm {
        let nodes = self
            .library
            .directories
            .nodes
            .get(self.library.directories.root.0 as usize)
            .map(|root| {
                root.children
                    .iter()
                    .filter_map(|node_id| {
                        self.library
                            .directories
                            .nodes
                            .get(node_id.0 as usize)
                            .map(|node| DirectoryEntryVm {
                                node_id: *node_id,
                                name: self
                                    .strings
                                    .resolve(node.name)
                                    .unwrap_or_else(|| "Unknown".to_string()),
                                child_count: node.children.len(),
                                track_count: node.tracks.len(),
                            })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let visible_tracks = self
            .selected_directory
            .and_then(|node_id| self.library.directories.nodes.get(node_id.0 as usize))
            .map(|node| self.track_rows_for_tracks(&node.tracks))
            .unwrap_or_default();

        LibraryTabVm {
            nodes,
            selected_directory: self.selected_directory,
            visible_tracks,
        }
    }

    fn scan_tab_vm(&self) -> ScanTabVm {
        ScanTabVm {
            include_dirs: self
                .scan
                .config
                .include_dirs
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            exclude_dirs: self
                .scan
                .config
                .exclude_dirs
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            visible_status: self.scan.visible_status(),
            cancel_visible: matches!(self.scan.state, ScanState::Scanning | ScanState::Cancelling),
        }
    }

    fn settings_tab_vm(&self) -> SettingsTabVm {
        SettingsTabVm {
            items: vec![
                SettingItemVm {
                    key: "accent_color".to_string(),
                    value: self.settings.accent_color.clone(),
                },
                SettingItemVm {
                    key: "theme".to_string(),
                    value: self.settings.theme.clone(),
                },
                SettingItemVm {
                    key: "seek_step_ms".to_string(),
                    value: self.settings.seek_step_ms.to_string(),
                },
                SettingItemVm {
                    key: "restore_position_policy".to_string(),
                    value: self.settings.restore_position_policy.clone(),
                },
                SettingItemVm {
                    key: "full_auto_scan".to_string(),
                    value: self.settings.full_auto_scan.to_string(),
                },
                SettingItemVm {
                    key: "fallback_artwork_path".to_string(),
                    value: self
                        .settings
                        .fallback_artwork_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "none".to_string()),
                },
                SettingItemVm {
                    key: "play_empty_state".to_string(),
                    value: self.settings.play_empty_state.clone(),
                },
            ],
            available_output_devices: AudioDeviceOutput::list_output_devices(),
            preferred_output_device: self.settings.preferred_output_device.clone(),
            debug_mode: self.settings.debug_mode,
            available_log_categories: crate::logging::available_log_categories(),
            selected_log_filters: self.settings.log_filters.clone(),
        }
    }

    fn track_rows_for_tracks(&self, tracks: &[TrackId]) -> Vec<TrackRowVm> {
        tracks
            .iter()
            .copied()
            .map(|track_id| self.track_row_vm(track_id, None))
            .collect()
    }

    fn track_row_vm(&self, track_id: TrackId, queue_index: Option<usize>) -> TrackRowVm {
        let track_index = track_id.0 as usize;
        let title = self
            .library
            .tracks
            .titles
            .get(track_index)
            .and_then(|id| self.strings.resolve(*id))
            .unwrap_or_else(|| format!("Track {}", track_id.0));
        let artist = self
            .library
            .tracks
            .artist_ids
            .get(track_index)
            .and_then(|artist_id| {
                artist_id.and_then(|id| self.library.artist_name(id, &self.strings))
            })
            .unwrap_or_else(|| "Unknown".to_string());
        let album = self
            .library
            .tracks
            .album_ids
            .get(track_index)
            .and_then(|album_id| {
                album_id.and_then(|id| self.library.album_title(id, &self.strings))
            })
            .unwrap_or_else(|| "Unknown".to_string());

        TrackRowVm {
            track_id,
            title,
            artist,
            album,
            artwork: None,
            favorite_state: if self.playlists.contains_in_favorite(track_id) {
                FavoriteStateVm::Included
            } else {
                FavoriteStateVm::Excluded
            },
            is_current: self.playback.current_track == Some(track_id),
            is_selected: queue_index
                .and_then(|index| self.queue.entries.get(index))
                .map(|entry| self.queue.selected.contains(&entry.id))
                .unwrap_or(false),
        }
    }
}

impl From<StrId> for String {
    fn from(value: StrId) -> Self {
        value.0.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use hound::{SampleFormat, WavSpec, WavWriter};
    use tempfile::TempDir;

    use super::AppFacade;
    use crate::app::commands::{AppCommand, TrackListOrigin};
    use crate::app::events::{AppEvent, PlaybackEvent};
    use crate::core::ids::TrackId;
    use crate::core::library::TrackInput;
    use crate::core::persistence::{JsonStore, PersistencePaths, TabId};
    use crate::core::playback::PlaybackState;
    use crate::core::scan::{ScanCommand, ScanState};

    fn temp_paths() -> PersistencePaths {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("lrc-player-facade-{unique}"));
        PersistencePaths {
            config_json: root.join("config.json"),
            state_json: root.join("state.json"),
            playlists_json: root.join("playlists.json"),
        }
    }

    fn poll_scan_until_finished(facade: &mut AppFacade) {
        for _ in 0..10_000 {
            facade.poll_background_tasks();
            if !matches!(
                facade.scan.state,
                ScanState::Scanning | ScanState::Cancelling
            ) {
                return;
            }
            thread::yield_now();
        }

        panic!("scan worker did not finish in time");
    }

    fn write_wav(path: &std::path::Path, frames: u32) {
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec).expect("wav file should be created");
        for index in 0..frames {
            let phase = (index % 256) as i16;
            let sample = (phase * 128) - 16_384;
            writer
                .write_sample(sample)
                .expect("left sample should be written");
            writer
                .write_sample(sample)
                .expect("right sample should be written");
        }
        writer.finalize().expect("wav file should finalize");
    }

    #[test]
    fn save_and_restore_round_trip() {
        let paths = temp_paths();
        let store = JsonStore::new(paths.clone());

        let mut facade = AppFacade::default();
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "Track One".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: "/music/album/01-track-one.flac".to_string(),
                file_size: 1,
                modified_unix_secs: 10,
                file_name: "01-track-one.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "Track Two".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: "/music/album/02-track-two.flac".to_string(),
                file_size: 2,
                modified_unix_secs: 10,
                file_name: "02-track-two.flac".to_string(),
                track_number: None,
                duration_ms: 2_000,
                artwork_key: None,
            },
        );

        facade.queue.replace_with_tracks(&[TrackId(0), TrackId(1)]);
        facade.queue.set_current_index(1);
        facade.playback.restore_track(TrackId(1), 3_210);
        facade.current_tab = TabId::Play;
        let _ = facade.handle_command(AppCommand::Scan(ScanCommand::AddIncludeDir(
            "/music".into(),
        )));
        let _ = facade.handle_command(AppCommand::CreatePlaylist {
            name: "Roadtrip".to_string(),
        });
        if let Some(playlist_id) = facade.playlists.selected {
            let _ = facade.handle_command(AppCommand::AddToPlaylist {
                playlist_id,
                track_id: TrackId(1),
            });
        }
        let _ = facade.handle_command(AppCommand::ToggleFavorite {
            track_id: TrackId(0),
        });

        facade.save_to_store(&store).expect("save should succeed");

        let mut restored = AppFacade::default();
        restored.library = facade.library.clone();
        restored.strings = facade.strings.clone();
        restored
            .restore_from_store(&store)
            .expect("restore should succeed");

        assert_eq!(restored.current_tab, TabId::Play);
        assert_eq!(restored.queue.current_track(), Some(TrackId(1)));
        assert_eq!(restored.playback.position_ms, 3_210);
        assert_eq!(
            restored.scan.config.include_dirs,
            vec![std::path::PathBuf::from("/music")]
        );
        assert!(restored.playlists.contains_in_favorite(TrackId(0)));

        let root = paths
            .config_json
            .parent()
            .expect("config parent should exist")
            .to_path_buf();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bootstrap_from_store_rescans_and_restores_queue() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let include_root = temp_dir.path().join("library_root");
        let music_dir = include_root.join("Artist").join("Album");
        fs::create_dir_all(&music_dir).expect("music dir should be created");
        fs::write(music_dir.join("01-track-one.flac"), b"fLaCdemo")
            .expect("track one should be created");
        fs::write(music_dir.join("02-track-two.flac"), b"fLaCdemo")
            .expect("track two should be created");

        let paths = temp_paths();
        let store = JsonStore::new(paths.clone());

        let mut facade = AppFacade::default();
        let _ = facade.handle_command(AppCommand::Scan(ScanCommand::AddIncludeDir(include_root)));
        let _ = facade.handle_command(AppCommand::Scan(ScanCommand::StartFullScan));
        poll_scan_until_finished(&mut facade);
        facade.queue.replace_with_tracks(&[TrackId(0), TrackId(1)]);
        facade.queue.set_current_index(1);
        facade.playback.restore_track(TrackId(1), 999);
        facade.save_to_store(&store).expect("save should succeed");

        let mut restored = AppFacade::default();
        restored
            .bootstrap_from_store(&store)
            .expect("bootstrap should succeed");

        assert_eq!(restored.library.tracks.titles.len(), 2);
        assert_eq!(restored.queue.current_track(), Some(TrackId(1)));
        assert_eq!(restored.playback.position_ms, 999);

        let root = paths
            .config_json
            .parent()
            .expect("config parent should exist")
            .to_path_buf();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn view_model_exposes_album_artist_playlist_library_and_settings_tabs() {
        let mut facade = AppFacade::default();
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "01-track-one".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: "/music/Artist/Album/01-track-one.flac".to_string(),
                file_size: 1,
                modified_unix_secs: 10,
                file_name: "01-track-one.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );
        let playlist_id = facade
            .playlists
            .create_user_playlist(facade.strings.intern("Roadtrip"))
            .expect("playlist should be created");
        facade.playlists.add_track(playlist_id, TrackId(0));
        facade.playlists.selected = Some(playlist_id);
        facade.selected_album = Some(crate::core::ids::AlbumId(0));
        facade.selected_artist = Some(crate::core::ids::ArtistId(0));
        facade.selected_directory = facade.library.directories.nodes[0]
            .children
            .first()
            .copied();

        let view_model = facade.view_model();

        assert_eq!(view_model.album.albums.len(), 1);
        assert_eq!(view_model.album.visible_tracks.len(), 1);
        assert_eq!(view_model.artist.artists.len(), 1);
        assert_eq!(view_model.artist.visible_tracks.len(), 1);
        assert_eq!(view_model.playlist.playlists.len(), 2);
        assert_eq!(view_model.playlist.visible_tracks.len(), 1);
        assert_eq!(view_model.library.nodes.len(), 1);
        assert!(!view_model.settings.items.is_empty());
    }

    #[test]
    fn select_album_command_updates_visible_tracks() {
        let mut facade = AppFacade::default();
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "track-a".to_string(),
                album: Some("Album A".to_string()),
                artist: Some("Artist A".to_string()),
                album_artist: Some("Artist A".to_string()),
                composer: None,
                normalized_path: "/music/a/01.flac".to_string(),
                file_size: 1,
                modified_unix_secs: 10,
                file_name: "01.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "track-b".to_string(),
                album: Some("Album B".to_string()),
                artist: Some("Artist B".to_string()),
                album_artist: Some("Artist B".to_string()),
                composer: None,
                normalized_path: "/music/b/01.flac".to_string(),
                file_size: 2,
                modified_unix_secs: 10,
                file_name: "01.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );

        let _ = facade.view_model();
        let _ = facade.handle_command(AppCommand::SelectAlbum(Some(crate::core::ids::AlbumId(1))));
        let view_model = facade.view_model();

        assert_eq!(
            view_model.album.expanded_album,
            Some(crate::core::ids::AlbumId(1))
        );
        assert_eq!(view_model.album.visible_tracks.len(), 1);
        assert_eq!(view_model.album.visible_tracks[0].title, "track-b");
    }

    #[test]
    fn select_artist_command_updates_visible_tracks() {
        let mut facade = AppFacade::default();
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "track-a".to_string(),
                album: Some("Album A".to_string()),
                artist: Some("Artist A".to_string()),
                album_artist: Some("Artist A".to_string()),
                composer: None,
                normalized_path: "/music/a/01.flac".to_string(),
                file_size: 1,
                modified_unix_secs: 10,
                file_name: "01.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "track-b".to_string(),
                album: Some("Album B".to_string()),
                artist: Some("Artist B".to_string()),
                album_artist: Some("Artist B".to_string()),
                composer: None,
                normalized_path: "/music/b/01.flac".to_string(),
                file_size: 2,
                modified_unix_secs: 10,
                file_name: "01.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );

        let _ = facade.view_model();
        let _ = facade.handle_command(AppCommand::SelectArtist(Some(crate::core::ids::ArtistId(
            1,
        ))));
        let view_model = facade.view_model();

        assert_eq!(
            view_model.artist.selected_artist,
            Some(crate::core::ids::ArtistId(1))
        );
        assert_eq!(view_model.artist.visible_tracks.len(), 1);
        assert_eq!(view_model.artist.visible_tracks[0].title, "track-b");
    }

    #[test]
    fn album_and_artist_lists_start_from_log_snapshot_then_hydrate_in_background() {
        let mut facade = AppFacade::default();
        for index in 0..80u32 {
            facade.library.upsert_track(
                &mut facade.strings,
                TrackInput {
                    title: format!("Track {index}"),
                    album: Some(format!("Album {index}")),
                    artist: Some(format!("Artist {index}")),
                    album_artist: Some(format!("Artist {index}")),
                    composer: None,
                    normalized_path: format!("/music/Artist{index}/Album{index}/{index:02}.flac"),
                    file_size: u64::from(index) + 1,
                    modified_unix_secs: 10,
                    file_name: format!("{index:02}.flac"),
                    track_number: None,
                    duration_ms: 1_000,
                    artwork_key: None,
                },
            );
        }

        let initial = facade.view_model();

        assert_eq!(initial.album.loading.hydrated_entries, 0);
        assert!(initial.album.loading.is_loading);
        assert!(
            initial
                .album
                .albums
                .iter()
                .all(|entry| entry.is_provisional)
        );
        assert_eq!(initial.artist.loading.hydrated_entries, 0);
        assert!(initial.artist.loading.is_loading);
        assert!(
            initial
                .artist
                .artists
                .iter()
                .all(|entry| entry.is_provisional)
        );

        facade.poll_background_tasks();
        let hydrated = facade.view_model();

        assert!(hydrated.album.loading.hydrated_entries > 0);
        assert!(hydrated.album.loading.hydrated_entries < hydrated.album.loading.total_entries);
        assert!(
            hydrated
                .album
                .albums
                .iter()
                .take(hydrated.album.loading.hydrated_entries)
                .all(|entry| !entry.is_provisional)
        );
        assert!(hydrated.artist.loading.hydrated_entries > 0);
        assert!(hydrated.artist.loading.hydrated_entries < hydrated.artist.loading.total_entries);
        assert!(
            hydrated
                .artist
                .artists
                .iter()
                .take(hydrated.artist.loading.hydrated_entries)
                .all(|entry| !entry.is_provisional)
        );
    }

    #[test]
    fn playback_prefills_audio_buffer_for_existing_file() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let file_path = temp_dir.path().join("sample.wav");
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&file_path, spec).expect("wav file should be created");
        for index in 0..16i16 {
            let sample = (index * 256) - 2048;
            writer
                .write_sample(sample)
                .expect("left sample should be written");
            writer
                .write_sample(sample)
                .expect("right sample should be written");
        }
        writer.finalize().expect("wav file should finalize");

        let mut facade = AppFacade::default();
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "sample".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: file_path.to_string_lossy().to_string(),
                file_size: fs::metadata(&file_path)
                    .expect("metadata should exist")
                    .len(),
                modified_unix_secs: 10,
                file_name: "sample.wav".to_string(),
                track_number: None,
                duration_ms: 0,
                artwork_key: None,
            },
        );
        facade.queue.replace_with_tracks(&[TrackId(0)]);

        let events = facade.handle_command(AppCommand::NextTrack);

        assert!(!events.is_empty());
        assert!(facade.audio_output.buffered_blocks() > 0);
        assert!(facade.playback.buffered_ms > 0);
    }

    #[test]
    fn start_scan_runs_in_background_until_polled() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let include_root = temp_dir.path().join("library_root");
        let music_dir = include_root.join("Artist").join("Album");
        fs::create_dir_all(&music_dir).expect("music dir should be created");
        fs::write(music_dir.join("01-track-one.flac"), b"fLaCdemo").expect("track should exist");

        let mut facade = AppFacade::default();
        let _ = facade.handle_command(AppCommand::Scan(ScanCommand::AddIncludeDir(include_root)));

        let events = facade.handle_command(AppCommand::Scan(ScanCommand::StartFullScan));

        assert_eq!(
            events,
            vec![AppEvent::ScanStateChanged(ScanState::Scanning)]
        );
        assert_eq!(facade.library.tracks.titles.len(), 0);

        poll_scan_until_finished(&mut facade);

        assert_eq!(facade.library.tracks.titles.len(), 1);
        assert_eq!(facade.scan.visible_status(), ScanState::Completed);
    }

    #[test]
    fn exhausted_track_advances_to_next_track_after_buffer_drains() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let first = temp_dir.path().join("first.wav");
        let second = temp_dir.path().join("second.wav");
        write_wav(&first, 2_048);
        write_wav(&second, 2_048);

        let mut facade = AppFacade::default();
        for (index, path, name) in [(0, &first, "first.wav"), (1, &second, "second.wav")] {
            facade.library.upsert_track(
                &mut facade.strings,
                TrackInput {
                    title: format!("track-{index}"),
                    album: Some("Album".to_string()),
                    artist: Some("Artist".to_string()),
                    album_artist: Some("Artist".to_string()),
                    composer: None,
                    normalized_path: path.to_string_lossy().to_string(),
                    file_size: fs::metadata(path).expect("metadata should exist").len(),
                    modified_unix_secs: 10,
                    file_name: name.to_string(),
                    track_number: None,
                    duration_ms: 0,
                    artwork_key: None,
                },
            );
        }
        facade.queue.replace_with_tracks(&[TrackId(0), TrackId(1)]);

        let _ = facade.handle_command(AppCommand::ClickTrack {
            visible_tracks: vec![TrackId(0), TrackId(1)],
            clicked_index: 0,
            origin: TrackListOrigin::Queue,
        });

        for _ in 0..10_000 {
            while facade.audio_output.buffered_blocks() > 0 {
                let _ = facade.audio_output.pop_or_silence();
            }
            facade.poll_background_tasks();
            if facade.queue.current_track() == Some(TrackId(1)) {
                break;
            }
            thread::yield_now();
        }

        assert_eq!(facade.queue.current_track(), Some(TrackId(1)));
        assert_eq!(facade.playback.current_track, Some(TrackId(1)));
        assert_eq!(facade.playback.state, PlaybackState::Playing);
    }

    #[test]
    fn undecodable_track_skips_to_next_track() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let broken = temp_dir.path().join("broken.mp3");
        let valid = temp_dir.path().join("valid.wav");
        fs::write(&broken, b"not-audio").expect("broken file should be written");

        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer =
            hound::WavWriter::create(&valid, spec).expect("wav file should be created");
        for _ in 0..16 {
            writer
                .write_sample(0i16)
                .expect("left sample should be written");
            writer
                .write_sample(0i16)
                .expect("right sample should be written");
        }
        writer.finalize().expect("wav file should finalize");

        let mut facade = AppFacade::default();
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "broken".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: broken.to_string_lossy().to_string(),
                file_size: 9,
                modified_unix_secs: 10,
                file_name: "broken.mp3".to_string(),
                track_number: None,
                duration_ms: 0,
                artwork_key: None,
            },
        );
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "valid".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: valid.to_string_lossy().to_string(),
                file_size: 64,
                modified_unix_secs: 10,
                file_name: "valid.wav".to_string(),
                track_number: None,
                duration_ms: 0,
                artwork_key: None,
            },
        );
        facade.queue.replace_with_tracks(&[TrackId(0), TrackId(1)]);
        facade.queue.set_current_index(0);

        let events = facade.handle_command(AppCommand::ClickTrack {
            visible_tracks: vec![TrackId(0), TrackId(1)],
            clicked_index: 0,
            origin: TrackListOrigin::Queue,
        });

        assert!(!events.is_empty());
        assert_eq!(facade.queue.current_track(), Some(TrackId(1)));
        assert!(facade.audio_output.buffered_blocks() > 0);
    }

    #[test]
    fn device_watcher_preferred_device_disappears_emits_underrun() {
        use std::sync::mpsc;
        use crate::audio::device_watcher::DeviceWatcherMessage;

        let (sender, receiver) = mpsc::channel();

        let mut facade = AppFacade::default();
        facade.settings.preferred_output_device = Some("test-dev".to_string());

        // simulate that a device is currently present (test-only flag)
        facade.set_test_audio_device_present(true, Some("test-dev".to_string()));

        // install a test watcher that uses our receiver
        let handle = crate::audio::device_watcher::DeviceWatcherHandle::from_receiver_for_test(receiver);
        facade.device_watcher = Some(handle);

        // send a DevicesChanged message with no devices -> preferred device disappeared
        let _ = sender.send(DeviceWatcherMessage::DevicesChanged {
            devices: vec![],
            default: None,
        });

        let events = facade.poll_background_tasks();

        assert!(events.iter().any(|e| matches!(e, AppEvent::Playback(PlaybackEvent::DeviceUnderrun))));
        assert!(facade.audio_disconnected);
        assert!(!facade.audio_device_present());
    }

    #[test]
    fn background_lazy_loader_loads_album_tracks_from_persisted_index() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let music_dir = temp_dir.path().join("Artist").join("Album");
        fs::create_dir_all(&music_dir).expect("music dir should be created");

        let file1 = music_dir.join("01.wav");
        let file2 = music_dir.join("02.wav");
        write_wav(&file1, 16);
        write_wav(&file2, 16);

        // create a JsonStore that uses a `data/` subdirectory inside our temp dir
        let base = temp_dir.path().to_path_buf();
        let store_paths = PersistencePaths {
            config_json: base.join("data").join("config").join("config.json"),
            state_json: base.join("data").join("state").join("state.json"),
            playlists_json: base.join("data").join("playlists").join("playlists.json"),
        };
        let store = JsonStore::new(store_paths.clone());

        let album_entry = crate::core::persistence::AlbumIndexEntry {
            title: "Album".to_string(),
            album_artist: "Artist".to_string(),
            track_count: 2,
            tracks: vec![file1.to_string_lossy().to_string(), file2.to_string_lossy().to_string()],
        };

        let artist_entry = crate::core::persistence::ArtistIndexEntry {
            name: "Artist".to_string(),
            track_count: 2,
            tracks: vec![file1.to_string_lossy().to_string(), file2.to_string_lossy().to_string()],
        };

        let artist_file = crate::core::persistence::ArtistIndexFile {
            track_artists: vec![artist_entry],
            album_artists: Vec::new(),
            composers: Vec::new(),
        };

        store
            .save_album_index(&vec![album_entry])
            .expect("save album index should succeed");
        store
            .save_artist_index(&artist_file)
            .expect("save artist index should succeed");

        let mut facade = AppFacade::default();
        facade
            .restore_from_store(&store)
            .expect("restore should succeed");

        let _ = facade.handle_command(AppCommand::SelectAlbum(Some(crate::core::ids::AlbumId(0))));

        for _ in 0..10_000 {
            facade.poll_background_tasks();
            if facade
                .library
                .indexes
                .tracks_by_album
                .get(0)
                .map(|v| v.len())
                .unwrap_or(0)
                == 2
            {
                break;
            }
            thread::yield_now();
        }

        assert_eq!(
            facade
                .library
                .indexes
                .tracks_by_album
                .get(0)
                .map(|v| v.len())
                .unwrap_or(0),
            2
        );

        let root = store_paths
            .config_json
            .parent()
            .expect("config parent should exist")
            .to_path_buf();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn measure_poll_and_browse_cache_timings() {
        use std::sync::{mpsc, Arc, atomic::{AtomicBool, Ordering}};
        use std::time::Instant;

        let mut facade = AppFacade::default();

        // Prepare a batch of synthetic TrackInput entries and send them via a
        // fake BrowseLoaderHandle so `poll_background_tasks()` will process
        // them and call `upsert_tracks_batch`.
        let n = 200usize;
        let mut inputs: Vec<crate::core::library::TrackInput> = Vec::with_capacity(n);
        for i in 0..n {
            inputs.push(crate::core::library::TrackInput {
                title: format!("Track {i}"),
                album: Some(format!("Album {}", i % 10)),
                artist: Some(format!("Artist {}", i % 20)),
                album_artist: Some(format!("Artist {}", i % 20)),
                composer: None,
                normalized_path: format!("/music/Artist{}/Album{}/{:02}.flac", i, i, i),
                file_size: i as u64 + 1,
                modified_unix_secs: 10,
                file_name: format!("{:02}.flac", i),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            });
        }

        let (sender, receiver) = mpsc::channel::<super::BrowseLoaderMessage>();
        let cancel = Arc::new(AtomicBool::new(false));

        // send a single TrackBatch message containing all inputs
        let _ = sender.send(super::BrowseLoaderMessage::TrackBatch(inputs));

        let loader = super::BrowseLoaderHandle { cancel_requested: Arc::clone(&cancel), receiver };
        facade.browse_loader = Some(loader);

        // Measure poll duration (will process up to MAX_UPSERTS_PER_POLL)
        let start = Instant::now();
        let events = facade.poll_background_tasks();
        let poll_dur = start.elapsed();
        eprintln!(
            "measure: poll_background_tasks duration_ms={} events={} tracks_after_poll={}",
            poll_dur.as_millis(),
            events.len(),
            facade.library.tracks.titles.len()
        );

        // Measure view_model() which triggers ensure_browse_cache_current() when
        // the library revision changed as a result of the poll upserts.
        let start2 = Instant::now();
        let _vm = facade.view_model();
        let view_dur = start2.elapsed();
        eprintln!(
            "measure: view_model (ensure_browse_cache_current) duration_ms={} albums={} artists={}",
            view_dur.as_millis(),
            facade.library.albums.titles.len(),
            facade.library.artists.names.len()
        );

        assert!(facade.library.tracks.titles.len() > 0);
    }

    #[test]
    fn measure_poll_with_many_tracks() {
        use std::sync::{mpsc, Arc, atomic::{AtomicBool, Ordering}};
        use std::time::Instant;

        let mut facade = AppFacade::default();

        // Large synthetic workload (reduced to 1k for timely measurement)
        let total = 1_024usize;
        let chunk_size = 64usize;

        // Build batches and send as separate TrackBatch messages so loader
        // simulates streaming many small batches like a real background loader.
        let (sender, receiver) = mpsc::channel::<super::BrowseLoaderMessage>();
        let cancel = Arc::new(AtomicBool::new(false));

        let mut i = 0usize;
        while i < total {
            let mut batch: Vec<crate::core::library::TrackInput> = Vec::new();
            for j in i..(i + chunk_size).min(total) {
                batch.push(crate::core::library::TrackInput {
                    title: format!("Track {j}"),
                    album: Some(format!("Album {}", j % 100)),
                    artist: Some(format!("Artist {}", j % 200)),
                    album_artist: Some(format!("Artist {}", j % 200)),
                    composer: None,
                    normalized_path: format!("/music/Artist{}/Album{}/{:05}.flac", j % 200, j % 100, j),
                    file_size: j as u64 + 1,
                    modified_unix_secs: 10,
                    file_name: format!("{:05}.flac", j),
                    track_number: None,
                    duration_ms: 1_000,
                    artwork_key: None,
                });
            }
            let _ = sender.send(super::BrowseLoaderMessage::TrackBatch(batch));
            i += chunk_size;
        }

        // send finished marker
        let _ = sender.send(super::BrowseLoaderMessage::Finished { missing_paths: Vec::new() });

        let loader = super::BrowseLoaderHandle { cancel_requested: Arc::clone(&cancel), receiver };
        facade.browse_loader = Some(loader);

        // Poll until loader completes; measure total poll time and number of polls.
        let start = Instant::now();
        let mut polls = 0usize;
        while facade.browse_loader.is_some() {
            let _ = facade.poll_background_tasks();
            polls += 1;
        }
        let total_dur = start.elapsed();
        eprintln!(
            "measure_large: total_polls={} total_duration_ms={} tracks_after={} albums={} artists={}",
            polls,
            total_dur.as_millis(),
            facade.library.tracks.titles.len(),
            facade.library.albums.titles.len(),
            facade.library.artists.names.len()
        );

        // Measure browse-cache rebuild via view_model()
        let start2 = Instant::now();
        let _vm = facade.view_model();
        let vm_dur = start2.elapsed();
        eprintln!(
            "measure_large: ensure_browse_cache_current_ms={} albums={} artists={}",
            vm_dur.as_millis(),
            facade.library.albums.titles.len(),
            facade.library.artists.names.len()
        );

        assert_eq!(facade.library.tracks.titles.len(), total);
    }

    #[test]
    fn measure_upsert_batch_and_browse_cache() {
        use std::time::Instant;

        // Measure a single batched upsert and the subsequent browse-cache rebuild.
        let total = 1_024usize;
        let mut inputs: Vec<crate::core::library::TrackInput> = Vec::with_capacity(total);
        for i in 0..total {
            inputs.push(crate::core::library::TrackInput {
                title: format!("Track {i}"),
                album: Some(format!("Album {}", i % 100)),
                artist: Some(format!("Artist {}", i % 200)),
                album_artist: Some(format!("Artist {}", i % 200)),
                composer: None,
                normalized_path: format!("/music/Artist{}/Album{}/{:05}.flac", i % 200, i % 100, i),
                file_size: i as u64 + 1,
                modified_unix_secs: 10,
                file_name: format!("{:05}.flac", i),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            });
        }

        let mut lib = crate::core::library::Library::default();
        let mut strings = crate::core::strings::StringInterner::default();

        let start = Instant::now();
        let ids = lib.upsert_tracks_batch(&mut strings, inputs.into_iter());
        let upsert_dur = start.elapsed();
        eprintln!(
            "measure_upsert: total={} upsert_ms={} ids={}",
            total,
            upsert_dur.as_millis(),
            ids.len()
        );

        // Move into facade and measure browse-cache rebuild via view_model()
        let mut facade = AppFacade::default();
        facade.library = lib;
        facade.strings = strings;

        let start2 = Instant::now();
        let _vm = facade.view_model();
        let vm_dur = start2.elapsed();
        eprintln!(
            "measure_upsert: ensure_browse_cache_current_ms={} albums={} artists={}",
            vm_dur.as_millis(),
            facade.library.albums.titles.len(),
            facade.library.artists.names.len()
        );

        assert_eq!(facade.library.tracks.titles.len(), total);
    }

    #[test]
    fn burst_upserts_repro() {
        use std::time::Instant;

        // Initialize logger to a temp data root so SimpleFileLogger is active
        let tmp = std::env::temp_dir().join("lrc_player_lock_test");
        let _ = std::fs::create_dir_all(tmp.join("config"));
        let paths = crate::core::persistence::PersistencePaths {
            config_json: tmp.join("config").join("config.json"),
            state_json: tmp.join("state.json"),
            playlists_json: tmp.join("playlists.json"),
        };
        match crate::logging::init(&paths) {
            Ok(_) => eprintln!("[test] logger initialized"),
            Err(e) => eprintln!("[test] logger init error: {}", e),
        }

        // Spawn background threads that log frequently to create contention
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
        let stop = Arc::new(AtomicBool::new(false));
        let mut bg_handles = Vec::new();
        for _ in 0..4 {
            let stopc = Arc::clone(&stop);
            bg_handles.push(std::thread::spawn(move || {
                while !stopc.load(Ordering::Relaxed) {
                    log::info!("bg_logger: heartbeat");
                    std::thread::sleep(std::time::Duration::from_micros(50));
                }
            }));
        }

        // Reproduce burst of small upserts via targeted scan outcome
        let count = 1024usize;
        let mut tracks: Vec<crate::core::library::TrackInput> = Vec::with_capacity(count);
        for i in 0..count {
            tracks.push(crate::core::library::TrackInput {
                title: format!("Track {i}"),
                album: Some(format!("Album {}", i % 100)),
                artist: Some(format!("Artist {}", i % 200)),
                album_artist: Some(format!("Artist {}", i % 200)),
                composer: None,
                normalized_path: format!("/music/Artist{}/Album{}/{:05}.flac", i % 200, i % 100, i),
                file_size: i as u64 + 1,
                modified_unix_secs: 10,
                file_name: format!("{:05}.flac", i),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            });
        }

        let mut facade = AppFacade::default();

        let outcome = crate::core::scan::ScanOutcome {
            result: crate::core::scan::ScanState::Completed,
            progress: crate::core::scan::ScanProgress::default(),
            tracks,
            targeted: true,
        };

        let start = Instant::now();
        facade.complete_scan(outcome);
        let dur = start.elapsed();
        eprintln!(
            "burst_upserts: count={} total_ms={} albums={} artists={}",
            count,
            dur.as_millis(),
            facade.library.albums.titles.len(),
            facade.library.artists.names.len()
        );

        assert_eq!(facade.library.tracks.titles.len(), count);
    }
}
