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
use std::cell::RefCell;
use std::thread;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::commands::{AppCommand, TrackListOrigin};
use crate::app::events::{AppEvent, PlaybackEvent};
use crate::app::view_models::{
    AlbumEntryVm, AlbumTabVm, AppViewModel, ArtistEntryVm, ArtistTabVm, DeferredListStateVm,
    DirectoryEntryVm, FavoriteStateVm, LibraryTabVm, PlayTabVm, PlaylistEntryVm, PlaylistTabVm,
    QueueTabVm, ScanTabVm, SettingItemVm, SettingsTabVm, TrackRowVm, TrackRowStaticVm,
};
use crate::app::vm_cache::VmCache;
mod view_model_assembler;
mod album_vm_builder;
mod artist_vm_builder;
mod playlist_vm_builder;
mod library_vm_builder;
mod queue_vm_builder;
mod play_vm_builder;
mod scan_vm_builder;
mod settings_vm_builder;
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
pub struct BrowseCacheState {
    library_revision: u64,
    albums: DeferredAlbumListState,
    track_artists: DeferredArtistListState,
    album_artists: DeferredArtistListState,
    composers: DeferredArtistListState,
}

// Cached VMs were previously stored in `CachedVm`. That responsibility has
// moved into `VmCache` to centralize presentation caches.

// 背景ローダからのメッセージ（単一トラック、バッチ、完了マーカー）
#[derive(Debug)]
pub enum BrowseLoaderMessage {
    Track(crate::core::library::TrackInput),
    TrackBatch(Vec<crate::core::library::TrackInput>),
    Finished { missing_paths: Vec<String> },
}

// ブラウズ用のバックグラウンドローダハンドル
pub struct BrowseLoaderHandle {
    pub cancel_requested: Arc<AtomicBool>,
    pub receiver: Receiver<BrowseLoaderMessage>,
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

    /// Test helper: construct a `BrowseLoaderHandle` from an existing receiver so
    /// integration tests can inject fake loader messages deterministically.
    pub fn from_receiver_for_test(receiver: Receiver<BrowseLoaderMessage>) -> Self {
        Self { cancel_requested: Arc::new(AtomicBool::new(false)), receiver }
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
pub struct BrowseCacheBuilderHandle {
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
                        title: Arc::from(entry.title.clone().into_boxed_str()),
                        album_artist: Arc::from(entry.album_artist.clone().into_boxed_str()),
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
                    title: Arc::from(entry.title.clone().into_boxed_str()),
                    album_artist: Arc::from(entry.album_artist.clone().into_boxed_str()),
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
                        name: Arc::from(entry.name.clone().into_boxed_str()),
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
                    name: Arc::from(entry.name.clone().into_boxed_str()),
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
    pub cache: VmCache,
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
    app_view_model: Option<crate::app::view_models::AppViewModel>,
    // coarse-grained flag retained for compatibility; fine-grained flags below drive rebuilds
    view_model_dirty: bool,
    // fine-grained dirty flags per tab to avoid full rebuilds
    dirty_queue: bool,
    dirty_play: bool,
    dirty_playlist: bool,
    dirty_library: bool,
    dirty_scan: bool,
    dirty_settings: bool,
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

        let mut facade = Self {
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
            cache: VmCache::new(),
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
            app_view_model: None,
            view_model_dirty: true,
            dirty_queue: false,
            dirty_play: false,
            dirty_playlist: false,
            dirty_library: false,
            dirty_scan: false,
            dirty_settings: false,
            album_index: Vec::new(),
            artist_index: crate::core::persistence::ArtistIndexFile::default(),
            artist_name_to_id: std::collections::HashMap::new(),
            browse_loader: None,
            browse_cache_builder: None,
        };

        // Initialize VmCache entries from current state so first `view_model()` can clone cheaply.
        facade.cache.album_vm = Some(facade.album_tab_vm());
        facade.cache.artist_vm = Some(facade.artist_tab_vm());

        // build initial AppViewModel snapshot so `view_model()` can return a reference
        let initial_vm = crate::app::view_models::AppViewModel {
            current_tab: facade.current_tab,
            queue: facade.queue_tab_vm(),
            play: facade.play_tab_vm(),
            album: facade.cache.album_vm.clone().unwrap(),
            artist: facade.cache.artist_vm.clone().unwrap(),
            playlist: facade.playlist_tab_vm(),
            library: facade.library_tab_vm(),
            scan: facade.scan_tab_vm(),
            settings: facade.settings_tab_vm(),
        };
        facade.app_view_model = Some(initial_vm);
        facade.view_model_dirty = false;

        facade
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
            // Full scan replaced library during bootstrap — invalidate cached VMs
            self.cache.invalidate_album();
            self.cache.invalidate_artist();
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
                if let Some(aid) = album_id {
                    self.lazy_load_album_tracks(aid);
                }
                // selection changed -> album VM may need update
                self.cache.invalidate_album();
                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::SelectArtistKind(kind) => {
                self.selected_artist_kind = kind;
                self.selected_artist = None;
                // switching artist kind changes artist list
                self.cache.invalidate_artist();
                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::SelectArtist(artist_id) => {
                let prev = self.selected_artist;
                self.selected_artist = artist_id;
                if let Some(aid) = artist_id {
                    self.lazy_load_artist_tracks(aid, self.selected_artist_kind);
                }
                // selection changed -> artist VM may need update
                self.cache.invalidate_artist();
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
                vec![AppEvent::SettingsChanged]
            }
            AppCommand::SetLogFilters { filters } => {
                self.settings.log_filters = filters.clone();
                // update runtime logger filters
                crate::logging::set_log_filters(filters.clone());
                vec![AppEvent::SettingsChanged]
            }
            AppCommand::Scan(scan_command) => self.handle_scan_command(scan_command),
            AppCommand::CreatePlaylist { name } => {
                let name_id = self.strings.intern(name);
                let _ = self.playlists.create_user_playlist(name_id);
                vec![AppEvent::PlaylistChanged]
            }
        };
        // Preserve causal ordering: model mutations applied above, events reflect those changes.
        // Mark fine-grained dirty flags according to produced events so UI rebuilds only affected tabs.
        self.mark_dirty_from_events(&events);
        events
    }

    /// 現在の内部状態から UI 描画用の `AppViewModel` を組み立てて返す。
    ///
    /// - 軽量な DTO を返すために必要なデータのみを抽出する。
    /// - 呼び出し中に `ensure_browse_cache_current()` をトリガして、ブラウズキャッシュのバックグラウンド構築を開始する。
    /// - UI フレーム中に頻繁に呼ばれることを想定しているため、コストを抑えた実装である。
    pub fn view_model(&mut self) -> &crate::app::view_models::AppViewModel {
        view_model_assembler::view_model(self)
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

    fn mark_dirty_from_events(&mut self, events: &[AppEvent]) {
        for event in events {
            match event {
                AppEvent::Playback(_) => self.dirty_play = true,
                AppEvent::QueueChanged => self.dirty_queue = true,
                AppEvent::PlaylistChanged => self.dirty_playlist = true,
                AppEvent::SettingsChanged => self.dirty_settings = true,
                AppEvent::ScanStateChanged(_) => self.dirty_scan = true,
                AppEvent::TabChanged(tab) => match tab {
                    crate::core::persistence::TabId::Queue => self.dirty_queue = true,
                    crate::core::persistence::TabId::Play => self.dirty_play = true,
                    crate::core::persistence::TabId::Album => self.cache.invalidate_album(),
                    crate::core::persistence::TabId::Artist => self.cache.invalidate_artist(),
                    crate::core::persistence::TabId::Playlist => self.dirty_playlist = true,
                    crate::core::persistence::TabId::Library => self.dirty_library = true,
                    crate::core::persistence::TabId::Scan => self.dirty_scan = true,
                    crate::core::persistence::TabId::Settings => self.dirty_settings = true,
                },
                AppEvent::TrackDetailsRequested(_) => {}
            }
        }

        if !events.is_empty() {
            self.view_model_dirty = true;
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
                // library mutated by background loader — invalidate album/artist cache
                self.cache.invalidate_album();
                self.cache.invalidate_artist();
                
            } else {
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

        // If any background events were produced, mark fine-grained dirty flags.
        self.mark_dirty_from_events(&events);
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
            // Targeted scan mutated the library — invalidate cached VMs
            self.cache.invalidate_album();
            self.cache.invalidate_artist();
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
            self.selected_album = new_selected_album;
            self.selected_artist = new_selected_artist;
            // Library replaced by scan result — invalidate album/artist cached VMs
            self.cache.invalidate_album();
            self.cache.invalidate_artist();
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
                        // New browse cache changes album/artist lists; invalidate cached VMs
                        self.cache.invalidate_album();
                        self.cache.invalidate_artist();
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
        queue_vm_builder::build_queue_tab(self)
    }

    fn play_tab_vm(&self) -> PlayTabVm {
        play_vm_builder::build_play_tab(self)
    }

    fn album_tab_vm(&self) -> AlbumTabVm {
        album_vm_builder::build_album_tab(self)
    }

    fn artist_tab_vm(&self) -> ArtistTabVm {
        artist_vm_builder::build_artist_tab(self)
    }

    fn playlist_tab_vm(&self) -> PlaylistTabVm {
        playlist_vm_builder::build_playlist_tab(self)
    }

    fn library_tab_vm(&self) -> LibraryTabVm {
        library_vm_builder::build_library_tab(self)
    }

    fn scan_tab_vm(&self) -> ScanTabVm {
        scan_vm_builder::build_scan_tab(self)
    }

    fn settings_tab_vm(&self) -> SettingsTabVm {
        settings_vm_builder::build_settings_tab(self)
    }

    fn track_rows_for_tracks(&self, tracks: &[TrackId]) -> Vec<TrackRowVm> {
        tracks
            .iter()
            .copied()
            .map(|track_id| self.track_row_vm(track_id, None))
            .collect()
    }

    fn track_row_vm(&self, track_id: TrackId, queue_index: Option<usize>) -> TrackRowVm {
        let factory = crate::app::track_row_factory::TrackRowFactory::new(
            &self.library,
            &self.playlists,
            &self.playback,
            &self.strings,
            &self.cache.track_rows,
        );

        factory.build(track_id, &self.queue, queue_index)
    }
}

impl From<StrId> for String {
    fn from(value: StrId) -> Self {
        value.0.to_string()
    }
}

