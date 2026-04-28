//! アプリケーション状態の永続化と基本操作を担当
//! 
//! このモジュールは以下の責務を持ちます：
//! - アプリケーション状態の保存と読み込み
//! - プレイリスト操作の委譲
//! - キュー操作の委譲
//! - ライブラリ操作の委譲
//! - 再生制御操作の委譲
//! - スキャン操作の委譲
//! - アプリケーションの初期化（`new`）

use std::collections::{HashMap, HashSet, VecDeque};

use cpal::traits::{DeviceTrait, HostTrait};
use eframe::CreationContext;
use lrc_player_application::config::Config;
use lrc_player_application::dto::{LibraryDto, PlaylistDto, StateDto};
use lrc_player_application::infrastructure::audio;
use lrc_player_application::infrastructure::persistence::{MetaCache, PlaylistsFile, StateFile};
use lrc_player_application::view_model::QueueVm;
use lrc_player_domain::model::{PlaybackMode, PlaybackState};

use crate::presentation::actions::{library, play, playlist, queue, scan};
use crate::presentation::app::{ArtistViewMode, GuiApp, Tab};

impl GuiApp {
    /// 保存されたアプリケーション状態をファイルから読み込みます。
    /// 
    /// 状態ファイル（`StateFile`）から前回終了時の状態を復元します。
    /// ファイルが存在しないか読み込みに失敗した場合は、デフォルト値で
    /// 初期化された`StateDto`を返します。
    /// 
    /// 復元される情報：
    /// - 最後に開いていたタブ
    /// - 最後に再生していたファイルパスと再生位置
    /// - スキャン設定（対象パス、除外パターン）
    /// - キュー状態（トラックリスト、選択インデックス）
    /// - 再生モード
    /// - アーティスト表示モード
    /// - UI設定（LRCオーバーレイ、サイドパネル展開状態）
    /// - お気に入りディレクトリ
    /// 
    /// # 戻り値
    /// 読み込まれた状態、またはデフォルト状態
    pub(crate) fn load_state() -> StateDto {
        StateFile::open()
            .and_then(|store| store.load())
            .unwrap_or(StateDto {
                last_tab: None,
                last_play_path: None,
                last_scan_status: None,
                scan_scanned: None,
                scan_parsed: None,
                scan_registered: None,
                scan_targets: Vec::new(),
                scan_lrc_targets: Vec::new(),
                scan_excludes: Vec::new(),
                queue_tracks: Vec::new(),
                selected_queue_index: None,
                queue_playback_mode: PlaybackMode::Sequential,
                artist_view_mode: ArtistViewMode::Artist,
                lrc_overlay_enabled: false,
                side_panel_expanded: true,
                last_play_position_ms: None,
                favorite_directories: Vec::new(),
            })
    }

    /// 現在のアプリケーション状態をファイルに保存します。
    /// 
    /// この関数は、状態が変更されるたびに呼び出され、以下の情報を永続化します：
    /// - 現在アクティブなタブ
    /// - 現在再生中のファイルパスと再生位置
    /// - 最後のスキャンステータス
    /// - スキャン設定（対象パス、LRC対象パス、除外パターン）
    /// - キュー内のトラックリストと選択状態
    /// - 再生モード
    /// - アーティスト表示モード
    /// - UI設定
    /// - お気に入りディレクトリ
    /// 
    /// 保存に失敗した場合は、エラーは無視されます（サイレント失敗）。
    pub(crate) fn save_state(&self) {
        // 現在の状態をStateDtoに変換
        let state = StateDto {
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
            selected_queue_index: self.selected_queue_index,
            queue_playback_mode: self.playback_mode,
            artist_view_mode: self.artist_view_mode,
            lrc_overlay_enabled: self.lrc_overlay_enabled,
            side_panel_expanded: self.side_panel_expanded,
            favorite_directories: self.favorite_directories.clone(),
        };

        // 状態ファイルに保存
        if let Ok(store) = StateFile::open() {
            let _ = store.save(&state);
        }
    }

    /// スキャンステータスをファイルから再読み込みして更新します。
    /// 
    /// バックエンドのスキャンプロセスは、進行状況を状態ファイルに書き込みます。
    /// この関数はそのファイルを読み込み、UIに表示されるスキャンステータスを
    /// 最新の状態に更新します。
    /// 
    /// 主にスキャンタブ表示中に定期的に呼び出されます。
    pub(crate) fn refresh_scan_status(&mut self) {
        if let Ok(store) = StateFile::open() {
            if let Ok(state) = store.load() {
                let next_status = state.last_scan_status.clone();
                let status_changed = self.last_scan_status != next_status;

                self.last_scan_status = next_status.clone();
                self.app_vm.scan.last_scan_status = next_status.clone();

                if status_changed && next_status.as_deref() == Some("Scan completed") {
                    self.discover_lyrics_for_current_play_path();
                }
            }
        }
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
    pub(crate) fn save_playlists(&self) {
        playlist::save_playlists(self);
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

    /// キューアイテムを1件削除します。
    pub(crate) fn remove_queue_item(&mut self, index: usize) {
        queue::remove_queue_item(self, index);
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

    /// 現在の再生位置へシークします。
    pub(crate) fn seek_to_position(&mut self) {
        play::seek_to_position(self);
    }

    /// 次のトラックを再生します。
    pub(crate) fn play_next(&mut self) {
        play::play_next(self);
    }

    /// 前のトラックを再生します。
    pub(crate) fn play_previous(&mut self) {
        play::play_previous(self);
    }

    /// 再生/一時停止をトグルします。
    pub(crate) fn toggle_play_pause(&mut self) {
        play::toggle_play_pause(self);
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
    /// この関数はeframeによってアプリケーション起動時に呼び出されます。
    /// 以下の初期化処理を行います：
    /// 
    /// 1. フォントのセットアップ（CJKフォールバックフォントの検出と設定）
    /// 2. 利用可能なオーディオ出力デバイスの列挙
    /// 3. 設定ファイルの読み込み
    /// 4. 保存された状態の復元
    /// 5. プレイリストの読み込みと正規化
    /// 6. キュー状態の復元
    /// 7. 各フィールドの初期化
    /// 8. 歌詞ファイルの検索
    /// 9. 再生メタデータの更新
    /// 10. ViewModelの構築
    /// 
    /// # 引数
    /// * `cc` - eframeの作成コンテキスト（フォント設定用）
    /// 
    /// # 戻り値
    /// 初期化された`GuiApp`インスタンス
    pub fn new(cc: &CreationContext<'_>) -> Self {
        // 1. フォントのセットアップ
        Self::setup_fonts(cc);

        // 2. 利用可能なオーディオ出力デバイスを列挙
        let mut device_names = Vec::new();
        if let Ok(devs) = cpal::default_host().output_devices() {
            for d in devs {
                if let Ok(name) = d.name() {
                    device_names.push(name);
                }
            }
        }

        // 3. 設定ファイルの読み込み
        let cfg = Config::load();
        
        // 4. 優先デバイスの選択インデックスを特定
        let selected = cfg
            .preferred_output_device
            .as_ref()
            .and_then(|n| device_names.iter().position(|s| s == n));

        // 5. 保存された状態の復元
        let state = Self::load_state();
        
        // 6. 最後に開いていたタブを復元（デフォルトはライブラリ）
        let active_tab = state
            .last_tab
            .as_deref()
            .and_then(Tab::from_label)
            .unwrap_or(Tab::Library);

        // 7. プレイリストの読み込みと正規化
        let playlists = Self::load_playlists();
        
        // 8. キュー状態の復元
        let queue = QueueVm::new(state.queue_tracks.clone());
        let selected_queue_index = state.selected_queue_index.filter(|&i| i < queue.tracks.len());

        // 9. GuiAppインスタンスの構築
        let mut gui = GuiApp {
            // ViewModel（表示用データモデル）
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
            
            // タブ状態
            active_tab,
            
            // オーディオ出力デバイス
            device_names,
            selected,
            device_filter: String::new(),
            
            // 再生状態
            play_path: state.last_play_path.clone().unwrap_or_default(),
            lyrics_files: Vec::new(),
            selected_lyrics_index: None,
            loaded_lyrics: None,
            play_position_ms: state.last_play_position_ms.unwrap_or(0),
            play_duration_ms: None,
            play_title: None,
            play_artist: None,
            play_album: None,
            play_artwork_handle: None,
            play_artwork_source: None,
            lyrics_offset_ms: 0,
            play_status: None,
            playback_state: PlaybackState::Stopped,
            playback_mode: state.queue_playback_mode,
            current_queue_index: selected_queue_index,
            
            // 再生バックエンドスレッド管理
            playback_status_receiver: None,
            playback_cancel_sender: None,
            playback_thread_handle: None,
            playback_start_instant: None,
            playback_start_ms: state.last_play_position_ms.unwrap_or(0),
            pending_play_request: false,
            active_output_device_name: None,
            last_seen_default_output_device_name: audio::cpal_output::default_device_name(),
            last_default_device_poll_at: None,
            playback_state_before_seek: None,
            
            // シーク操作
            pending_seek_ms: None,
            seek_last_change_at: None,
            seek_interacting: false,
            seek_long_press_start: None,
            seek_key_hold_direction: 0,
            seek_key_hold_started_at: None,
            seek_key_last_step_at: None,
            
            // キューUI
            queue_scroll_to_playing: active_tab == Tab::Queue,
            
            // アーティスト表示
            artist_view_mode: state.artist_view_mode,
            
            // UI設定
            side_panel_expanded: state.side_panel_expanded,
            
            // スキャン設定
            scan_target_input: String::new(),
            scan_lrc_target_input: String::new(),
            scan_exclude_input: String::new(),
            scan_targets: state.scan_targets,
            scan_lrc_targets: state.scan_lrc_targets,
            scan_excludes: state.scan_excludes,
            
            // ライブラリ選択状態
            selected_album: None,
            selected_artist: None,
            selected_directory: None,
            favorite_directories: state.favorite_directories,
            favorite_directory_input: String::new(),
            library_expanded_directories: HashSet::new(),
            
            // プレイリスト
            playlists,
            new_playlist_name: String::new(),
            selected_playlist: None,
            playlist_track_input: String::new(),
            
            // キュー
            queue,
            queue_item_input: String::new(),
            selected_queue_index,
            selected_queue_indices: selected_queue_index.into_iter().collect(),
            queue_selection_anchor: selected_queue_index,
            queue_handle_hold_index: None,
            queue_handle_hold_started_at: None,
            queue_drag_source_index: None,
            
            // オーディオ処理
            taps: cfg.taps_for_device(cfg.preferred_output_device.as_ref()),
            
            // アプリケーション設定
            theme: cfg.theme,
            log_level: cfg.log_level,
            language: cfg.language,
            last_scan_status: state.last_scan_status,
            
            // アートワーク
            play_artwork_blur_handle: None,
            play_artwork_blur_source: None,
            lrc_overlay_enabled: state.lrc_overlay_enabled,
            fft_overlay_enabled: false,
            
            // アルバムアートワークキャッシュ
            album_meta_cache: MetaCache::open_default().ok(),
            album_artwork_path_cache: HashMap::new(),
            album_artwork_texture_cache: HashMap::new(),
            album_artwork_pending: VecDeque::new(),
            album_artwork_queued: HashSet::new(),
            album_artwork_album_titles: Vec::new(),
            
            // トラック詳細ダイアログ
            track_details_dialog: None,
        };

        // 10. 追加の初期化処理
        gui.discover_lyrics_for_current_play_path();  // 歌詞ファイルを検索
        gui.refresh_play_metadata();                   // 再生メタデータを更新
        gui.update_app_vm();                           // ViewModelを構築
        
        gui
    }
}