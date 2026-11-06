//! アプリケーション状態の永続化と基本操作を担当
//!
//! - State persistence goes through a debounced background [`StateSaver`]: hot paths only set
//!   a dirty flag ([`GuiApp::save_state`]); the frame loop hands a snapshot to the saver at
//!   most once per second, and `on_exit` flushes synchronously.
//! - Delegation to the action modules.
//! - Construction of the app (`new`): fonts, theme, audio engine, shared DB handle.

use std::collections::HashSet;
use std::time::Instant;

use eframe::{egui, CreationContext};
use lrc_player_application::config::{Config, Theme};
use lrc_player_application::dto::{legacy_playback_mode, LibraryDto, PlaylistDto, StateDto};
use lrc_player_application::infrastructure::audio::engine::AudioEngine;
use lrc_player_application::infrastructure::library_snapshot::LibrarySnapshotCache;
use lrc_player_application::infrastructure::persistence::{MetaCache, PlaylistsFile, StateFile, StateSaver};
use lrc_player_application::usecases::queue_navigation::QueueNavigator;
use lrc_player_application::view_model::QueueVm;
use lrc_player_domain::model::PlaybackState;

use crate::presentation::actions::{library, play, playlist, queue, scan};
use crate::presentation::app::playback::{create_engine, eq_from_dto, eq_to_dto};
use crate::presentation::app::{
    scan_status, track_info, ArtistViewMode, ArtworkLoader, GuiApp, Tab, TrackMetaCache,
};
use crate::presentation::components::fft_overlay::FftAnalyzer;
use crate::presentation::components::lrc_overlay::LrcLayoutCache;

/// Applies the theme setting (System follows the OS).
pub(crate) fn apply_theme(ctx: &egui::Context, theme: Theme) {
    let pref = match theme {
        Theme::System => egui::ThemePreference::System,
        Theme::Light => egui::ThemePreference::Light,
        Theme::Dark => egui::ThemePreference::Dark,
    };
    ctx.set_theme(pref);
}

impl GuiApp {
    /// 保存されたアプリケーション状態をファイルから読み込みます（無ければ既定値）。
    pub(crate) fn load_state() -> StateDto {
        StateFile::open()
            .and_then(|store| store.load())
            .unwrap_or_default()
    }

    /// Builds the persisted state from the in-memory app state.
    pub(crate) fn build_state_dto(&self) -> StateDto {
        let shuffle = self.navigator.shuffle();
        let repeat = self.navigator.repeat();
        StateDto {
            last_tab: Some(self.active_tab.label().to_string()),
            last_play_path: if self.play_path.is_empty() {
                None
            } else {
                Some(self.play_path.clone())
            },
            last_play_position_ms: Some(self.play_position_ms),
            last_scan_status: self.last_scan_status.clone(),
            scan_scanned: None,
            scan_parsed: None,
            scan_registered: None,
            scan_targets: self.scan_targets.clone(),
            scan_lrc_targets: self.scan_lrc_targets.clone(),
            scan_excludes: self.scan_excludes.clone(),
            queue_tracks: self.queue.tracks.clone(),
            selected_queue_index: self.current_queue_index.or(self.selected_queue_index),
            queue_playback_mode: legacy_playback_mode(shuffle, repeat),
            artist_view_mode: self.artist_view_mode,
            lrc_overlay_enabled: self.lrc_overlay_enabled,
            fft_overlay_enabled: self.fft_overlay_enabled,
            side_panel_expanded: self.side_panel_expanded,
            favorite_directories: self.favorite_directories.clone(),
            volume: self.volume,
            muted: self.muted,
            eq: eq_to_dto(&self.eq),
            shuffle: Some(shuffle),
            repeat: Some(repeat),
            lyrics_offsets: self.lyrics_offsets.clone(),
        }
    }

    /// Marks the state as changed. Cheap: the frame loop hands it to the background saver
    /// (at most once per second); nothing is serialized or written here.
    pub(crate) fn save_state(&mut self) {
        self.state_dirty = true;
    }

    /// Serializes the state now and queues it on the background saver (no disk I/O here).
    pub(crate) fn submit_state_now(&mut self) {
        self.state_dirty = false;
        self.state_last_submit = Some(Instant::now());
        let state = self.build_state_dto();
        match &self.state_saver {
            Some(saver) => {
                if let Err(err) = saver.save(&state) {
                    tracing::warn!("state save failed: {err:#}");
                }
            }
            None => {
                if let Ok(store) = StateFile::open() {
                    let _ = store.save(&state);
                }
            }
        }
    }

    /// Submits the current state and blocks until it is on disk (exit path).
    pub(crate) fn flush_state(&mut self) {
        self.submit_state_now();
        if let Some(saver) = &self.state_saver {
            if let Err(err) = saver.flush() {
                tracing::warn!("state flush failed: {err:#}");
            }
        }
    }

    /// Applies scan status updates published in-process by the scan server (no file polling).
    pub(crate) fn poll_scan_status(&mut self) {
        let mut completed = false;
        while let Some(update) = scan_status::try_recv() {
            if update.status == "Scan completed" {
                completed = true;
            }
            self.last_scan_status = Some(update.status.clone());
            self.app_vm.scan.last_scan_status = Some(update.status);
            self.scan_counts = (update.scanned, update.parsed, update.registered);
            self.app_vm.scan.scan_scanned = update.scanned;
            self.app_vm.scan.scan_parsed = update.parsed;
            self.app_vm.scan.scan_registered = update.registered;
            self.save_state();
        }
        if completed {
            self.on_library_changed();
            self.request_track_info();
        }
    }

    /// Drops library-derived caches after a scan or a library clear.
    pub(crate) fn on_library_changed(&mut self) {
        self.library_snapshot.invalidate();
        self.track_meta.clear();
        self.artwork.clear();
        self.update_app_vm();
    }

    /// 保存されたプレイリストをファイルから読み込み、正規化します。
    /// 
    /// プレイリストファイルが存在しない場合は空のリストを返します。
    /// 読み込み後、`normalize_playlists`を呼び出して：
    /// - 複数の"favorite"プレイリストを1つに統合
    /// - favoriteプレイリスト内の重複トラックを除去
    /// - favoriteプレイリストをリストの先頭に配置
    /// 
    /// # 戻り値
    /// 正規化されたプレイリストのリスト
    pub(crate) fn load_playlists() -> Vec<PlaylistDto> {
        if let Ok(file) = PlaylistsFile::open() {
            let playlists = file.load().unwrap_or_default();
            Self::normalize_playlists(playlists)
        } else {
            Self::normalize_playlists(Vec::new())
        }
    }

    /// プレイリスト一覧を正規化します。
    /// 
    /// `playlist`モジュールの同名関数に処理を委譲します。
    pub(crate) fn normalize_playlists(playlists: Vec<PlaylistDto>) -> Vec<PlaylistDto> {
        playlist::normalize_playlists(playlists)
    }

    /// プレイリストをファイルに保存します。
    /// 
    /// `playlist`モジュールの同名関数に処理を委譲します。
    pub(crate) fn save_playlists(&mut self) {
        playlist::save_playlists(self);
        self.rebuild_favorite_set();
    }

    /// Rebuilds the O(1) favorite lookup set from the favorite playlist (playlists[0]).
    pub(crate) fn rebuild_favorite_set(&mut self) {
        self.favorite_set = self
            .playlists
            .first()
            .map(|p| p.tracks.iter().cloned().collect())
            .unwrap_or_default();
    }

    /// 新しいプレイリストを追加します。
    pub(crate) fn add_playlist(&mut self) {
        playlist::add_playlist(self);
    }

    /// 指定されたインデックスのプレイリストを削除します。
    pub(crate) fn remove_playlist(&mut self, index: usize) {
        playlist::remove_playlist(self, index);
    }

    /// プレイリストを一つ上に移動します。
    pub(crate) fn move_playlist_up(&mut self, index: usize) {
        playlist::move_playlist_up(self, index);
    }

    /// プレイリストを一つ下に移動します。
    pub(crate) fn move_playlist_down(&mut self, index: usize) {
        playlist::move_playlist_down(self, index);
    }

    /// キューにアイテムを追加します。
    pub(crate) fn add_queue_item(&mut self) {
        queue::add_queue_item(self);
    }

    /// キュー内の選択状態をクリアします。
    pub(crate) fn clear_queue_selection(&mut self) {
        queue::clear_queue_selection(self);
    }

    /// ライブラリをクリアします。
    pub(crate) fn clear_library(&mut self) {
        library::clear_library(self);
    }

    /// キューアイテムを単一選択します。
    pub(crate) fn select_queue_item_single(&mut self, index: usize) {
        queue::select_queue_item_single(self, index);
    }

    /// キューアイテムの選択状態をトグルします。
    pub(crate) fn toggle_queue_item_selection(&mut self, index: usize) {
        queue::toggle_queue_item_selection(self, index);
    }

    /// アンカーから指定インデックスまでの範囲を選択します。
    pub(crate) fn select_queue_item_range_to(&mut self, index: usize) {
        queue::select_queue_item_range_to(self, index);
    }

    /// 選択中のキューアイテムを削除します。
    pub(crate) fn remove_selected_queue_items(&mut self) {
        queue::remove_selected_queue_items(self);
    }

    /// キューハンドルのホールド状態をクリアします。
    pub(crate) fn clear_queue_handle_hold(&mut self) {
        queue::clear_queue_handle_hold(self);
    }

    /// キュー内のドラッグ操作をキャンセルします。
    pub(crate) fn cancel_queue_drag(&mut self) {
        queue::cancel_queue_drag(self);
    }

    /// キューアイテムを指定位置に移動します。
    pub(crate) fn move_queue_item_to(&mut self, from: usize, to: usize) {
        queue::move_queue_item_to(self, from, to);
    }


    /// キューアイテムを一つ上に移動します。
    pub(crate) fn move_queue_item_up(&mut self, index: usize) {
        queue::move_queue_item_up(self, index);
    }

    /// キューアイテムを一つ下に移動します。
    pub(crate) fn move_queue_item_down(&mut self, index: usize) {
        queue::move_queue_item_down(self, index);
    }

    /// キューをクリアします。
    pub(crate) fn clear_queue(&mut self) {
        queue::clear_queue(self);
    }

    /// アルバムを選択します。
    pub(crate) fn select_album(&mut self, album: Option<String>) {
        library::select_album(self, album);
    }

    /// アーティストを選択します。
    pub(crate) fn select_artist(&mut self, artist: Option<String>) {
        library::select_artist(self, artist);
    }

    /// アーティストビューの表示モードを設定します。
    pub(crate) fn set_artist_view_mode(&mut self, mode: ArtistViewMode) {
        library::set_artist_view_mode(self, mode);
    }

    /// ディレクトリを選択します。
    pub(crate) fn select_directory(&mut self, directory: Option<String>) {
        library::select_directory(self, directory);
    }

    /// ライブラリディレクトリの展開状態をトグルします。
    pub(crate) fn toggle_library_directory_expanded(&mut self, path: &str) {
        library::toggle_library_directory_expanded(self, path);
    }

    /// ライブラリディレクトリが展開状態かどうかを返します。
    pub(crate) fn is_library_directory_expanded(&self, path: &str) -> bool {
        library::is_library_directory_expanded(self, path)
    }

    /// お気に入りディレクトリを追加します。
    pub(crate) fn add_favorite_directory(&mut self) {
        library::add_favorite_directory(self);
    }

    /// お気に入りディレクトリを削除します。
    pub(crate) fn remove_favorite_directory(&mut self, index: usize) {
        library::remove_favorite_directory(self, index);
    }

    /// 選択中のキューアイテムを再生します。
    pub(crate) fn play_selected_queue_item(&mut self) {
        play::play_selected_queue_item(self);
    }





    /// スキャン要求を送信します。
    pub(crate) fn request_scan(&mut self) {
        scan::request_scan(self);
    }

    /// スキャンキャンセル要求を送信します。
    pub(crate) fn request_cancel(&mut self) {
        scan::request_cancel(self);
    }

    /// スキャン対象を追加します。
    pub(crate) fn add_scan_target(&mut self) {
        scan::add_scan_target(self);
    }

    /// スキャン対象を削除します。
    pub(crate) fn remove_scan_target(&mut self, index: usize) {
        scan::remove_scan_target(self, index);
    }

    /// LRCスキャン対象を追加します。
    pub(crate) fn add_scan_lrc_target(&mut self) {
        scan::add_scan_lrc_target(self);
    }

    /// LRCスキャン対象を削除します。
    pub(crate) fn remove_scan_lrc_target(&mut self, index: usize) {
        scan::remove_scan_lrc_target(self, index);
    }

    /// スキャン除外パターンを追加します。
    pub(crate) fn add_scan_exclude(&mut self) {
        scan::add_scan_exclude(self);
    }

    /// スキャン除外パターンを削除します。
    pub(crate) fn remove_scan_exclude(&mut self, index: usize) {
        scan::remove_scan_exclude(self, index);
    }

    /// `GuiApp`の新しいインスタンスを構築します。
    ///
    /// フォント・テーマ、オーディオエンジン（1 つだけ・常駐）、共有 DB ハンドル、
    /// 状態の復元を行います。重いファイル処理（メタデータ・歌詞・アートワーク）は
    /// ワーカースレッドで行うため、最初のフレームを遅らせません。
    pub fn new(cc: &CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        Self::setup_fonts(cc);

        let cfg = Config::load();
        apply_theme(&ctx, cfg.theme);
        let state = Self::load_state();

        let active_tab = state
            .last_tab
            .as_deref()
            .and_then(Tab::from_label)
            .unwrap_or(Tab::Library);

        let playlists = Self::load_playlists();
        let queue = QueueVm::new(state.queue_tracks.clone());
        let selected_queue_index = state.selected_queue_index.filter(|&i| i < queue.tracks.len());
        let (shuffle, repeat) = state.shuffle_repeat();
        let mut navigator = QueueNavigator::new(false, repeat);
        navigator.set_shuffle(shuffle, selected_queue_index, queue.tracks.len());

        let volume = state.volume.clamp(0.0, 1.0);
        let eq = eq_from_dto(&state.eq);
        let engine = create_engine(&ctx, &cfg, volume, state.muted, eq);
        let device_names = AudioEngine::output_device_names();

        let meta = match MetaCache::shared() {
            Ok(meta) => Some(meta),
            Err(err) => {
                tracing::warn!("opening the library database failed: {err:#}");
                None
            }
        };
        let state_saver = match StateSaver::open_default() {
            Ok(saver) => Some(saver),
            Err(err) => {
                tracing::warn!("state saver unavailable, saving synchronously: {err:#}");
                None
            }
        };
        scan_status::set_repaint_context(ctx.clone());

        let play_path = state.last_play_path.clone().unwrap_or_default();
        let lyrics_offset_ms = state.lyrics_offsets.get(&play_path).copied().unwrap_or(0);

        let mut gui = GuiApp {
            app_vm: Self::default_app_vm(
                active_tab,
                &cfg,
                state.last_scan_status.clone(),
                state.scan_targets.clone(),
                state.scan_lrc_targets.clone(),
                state.scan_excludes.clone(),
                LibraryDto {
                    album_count: 0,
                    track_count: 0,
                    recent_locations: Vec::new(),
                    recent_directories: Vec::new(),
                    directory_tree: Vec::new(),
                    favorite_directories: state.favorite_directories.clone(),
                    albums: Vec::new(),
                    selected_album: None,
                    selected_album_tracks: Vec::new(),
                    artists: Vec::new(),
                    selected_artist: None,
                    selected_artist_tracks: Vec::new(),
                    selected_directory: None,
                    selected_directory_tracks: Vec::new(),
                },
                &playlists,
                queue.clone(),
            ),
            active_tab,

            engine,
            engine_snapshot: None,
            device_names,
            preferred_output_device: cfg.preferred_output_device.clone(),
            device_filter: String::new(),
            active_output_device: None,
            resampler: cfg.resampler,
            taps: cfg.taps_for_device(cfg.preferred_output_device.as_ref()),

            play_path,
            playback_state: PlaybackState::Stopped,
            navigator,
            current_queue_index: selected_queue_index,
            pending_next: None,
            consecutive_errors: 0,
            pending_error_skip: false,
            volume,
            muted: state.muted,
            eq,

            pending_seek_ms: None,
            seek_last_change_at: None,
            seek_display_hold: None,
            seek_interacting: false,
            seek_key_hold_direction: 0,
            seek_key_hold_started_at: None,
            seek_key_last_step_at: None,
            seek_key_hold_steps: 0,

            queue_scroll_to_playing: active_tab == Tab::Queue,
            track_list_scroll_request: None,
            artist_view_mode: state.artist_view_mode,

            play_position_ms: state.last_play_position_ms.unwrap_or(0),
            play_duration_ms: None,
            play_title: None,
            play_artist: None,
            play_album: None,
            track_info: track_info::TrackInfoLoader::new(ctx.clone()),

            play_artwork_handle: None,
            play_artwork_blur_handle: None,
            play_artwork_requested_for: None,

            lrc_overlay_enabled: state.lrc_overlay_enabled,
            fft_overlay_enabled: state.fft_overlay_enabled,
            lyrics_files: Vec::new(),
            lyrics_candidates: Vec::new(),
            selected_lyrics_index: None,
            loaded_lyrics: None,
            loaded_timeline: None,
            lyrics_offset_ms,
            lyrics_offsets: state.lyrics_offsets.clone(),
            lrc_layout: LrcLayoutCache::default(),
            fft: FftAnalyzer::default(),

            play_status: None,

            scan_target_input: String::new(),
            scan_lrc_target_input: String::new(),
            scan_exclude_input: String::new(),
            scan_targets: state.scan_targets,
            scan_lrc_targets: state.scan_lrc_targets,
            scan_excludes: state.scan_excludes,

            selected_album: None,
            selected_artist: None,
            selected_directory: None,
            favorite_directories: state.favorite_directories,
            favorite_directory_input: String::new(),
            library_expanded_directories: HashSet::new(),

            playlists,
            favorite_set: HashSet::new(),
            new_playlist_name: String::new(),
            selected_playlist: None,
            playlist_track_input: String::new(),

            queue,
            queue_item_input: String::new(),
            selected_queue_index,
            selected_queue_indices: selected_queue_index.into_iter().collect(),
            queue_selection_anchor: selected_queue_index,
            queue_handle_hold_index: None,
            queue_handle_hold_started_at: None,
            queue_drag_source_index: None,

            side_panel_expanded: state.side_panel_expanded,
            theme: cfg.theme,
            log_level: cfg.log_level,
            language: cfg.language,

            last_scan_status: state.last_scan_status,
            scan_counts: (None, None, None),

            artwork: ArtworkLoader::new(ctx.clone()),
            meta,
            library_snapshot: LibrarySnapshotCache::new(),
            track_meta: TrackMetaCache::default(),

            state_saver,
            state_dirty: false,
            state_last_submit: None,

            track_details_dialog: None,
        };

        gui.rebuild_favorite_set();
        if !gui.play_path.is_empty() {
            // Duration / tags / lyrics for the restored track, off the UI thread.
            gui.request_track_info();
        }
        gui.update_app_vm();
        gui
    }
}
