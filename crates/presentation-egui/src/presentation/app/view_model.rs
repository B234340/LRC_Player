//! ViewModelの構築を担当
//! - AppVmの生成
//! - LibraryDtoの構築

use lrc_player_application::config::Config;
use lrc_player_application::dto::{LibraryDto, PlaylistDto, SettingsDto, StateDto};
use lrc_player_application::infrastructure::persistence::MetaCache;
use lrc_player_application::usecases::build_app_vm::build_app_vm;
use lrc_player_application::view_model::AppVm;
use lrc_player_domain::model::PlaybackMode;

use crate::presentation::app::{GuiApp, Tab};

impl GuiApp {
    /// デフォルトのAppVm（表示用データモデル）を構築します。
    /// 
    /// この関数は、与えられたパラメータからアプリケーションのViewModelを生成します。
    /// ViewModelはUIとドメイン層の間のデータ変換を担当し、UIに表示するための
    /// 整形されたデータを提供します。
    /// 
    /// 構築の流れ：
    /// 1. 設定からSettingsDtoを構築
    /// 2. 状態情報をStateDtoにまとめる
    /// 3. usecase層の`build_app_vm`関数を呼び出してViewModelを生成
    /// 
    /// # 引数
    /// * `active_tab` - 現在アクティブなタブ
    /// * `config` - アプリケーション設定
    /// * `last_scan_status` - 最後のスキャン操作のステータスメッセージ
    /// * `scan_targets` - スキャン対象パスのリスト
    /// * `scan_lrc_targets` - LRC歌詞スキャン対象パスのリスト
    /// * `scan_excludes` - スキャン除外パターンのリスト
    /// * `library` - ライブラリ情報のDTO
    /// * `playlists` - プレイリストのリスト
    /// * `queue` - キュー（再生待ちリスト）のViewModel
    /// 
    /// # 戻り値
    /// 構築されたAppVmインスタンス
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn default_app_vm(
        active_tab: Tab,
        config: &Config,
        last_scan_status: Option<String>,
        scan_targets: Vec<String>,
        scan_lrc_targets: Vec<String>,
        scan_excludes: Vec<String>,
        library: LibraryDto,
        playlists: &[PlaylistDto],
        queue: lrc_player_application::view_model::QueueVm,
    ) -> AppVm {
        // 設定からSettingsDtoを構築
        // - preferred_output_device: 優先出力デバイス名
        // - taps: リサンプリング品質（タップ数）
        let settings = SettingsDto::new(
            config.preferred_output_device.clone(),
            config.taps_for_device(config.preferred_output_device.as_ref()),
        );

        // 状態情報をStateDtoにまとめる
        // 注: 一部のフィールドは初期化時のデフォルト値が使用される
        let state = StateDto {
            last_tab: Some(active_tab.label().to_string()),
            last_play_path: None,                    // 起動時は再生パスなし
            last_play_position_ms: None,             // 起動時は再生位置なし
            last_scan_status,                         // 前回終了時のスキャンステータス
            scan_scanned: None,                       // ランタイムで更新される
            scan_parsed: None,                        // ランタイムで更新される
            scan_registered: None,                    // ランタイムで更新される
            scan_targets,                             // ユーザー設定のスキャン対象
            scan_lrc_targets,                         // ユーザー設定のLRCスキャン対象
            scan_excludes,                            // ユーザー設定の除外パターン
            queue_tracks: Vec::new(),                 // キューは別途管理
            selected_queue_index: None,               // キュー選択は別途管理
            queue_playback_mode: PlaybackMode::Sequential, // デフォルトは順次再生
            artist_view_mode: crate::presentation::app::ArtistViewMode::Artist,
            lrc_overlay_enabled: false,               // デフォルトは無効
            side_panel_expanded: true,                // デフォルトは展開
            favorite_directories: Vec::new(),         // お気に入りディレクトリは別途管理
        };

        // usecase層の関数を使用してViewModelを構築
        build_app_vm("LRC Player", &settings, &state, playlists, &library, queue)
    }

    /// 現在の状態からLibraryDtoを構築します。
    /// 
    /// この関数は、メタデータキャッシュからライブラリ情報を取得し、
    /// UI表示用のDTO（Data Transfer Object）に変換します。
    /// 
    /// 取得される情報：
    /// - ライブラリ統計（アルバム数、トラック数）
    /// - 最近追加されたトラックとディレクトリ
    /// - ディレクトリツリー
    /// - アルバム一覧
    /// - アーティスト一覧（現在の表示モードに応じて）
    /// - 選択中のアルバム/アーティスト/ディレクトリのトラック一覧
    /// 
    /// メタキャッシュのオープンに失敗した場合は、デフォルト値（空）の
    /// LibraryDtoが返されます。
    /// 
    /// # 戻り値
    /// 現在のライブラリ状態を表すLibraryDto
    pub(crate) fn library_dto(&self) -> LibraryDto {
        // デフォルト値で初期化
        let mut library = LibraryDto {
            album_count: 0,
            track_count: 0,
            recent_locations: Vec::new(),
            recent_directories: Vec::new(),
            directory_tree: Vec::new(),
            favorite_directories: self.favorite_directories.clone(),
            albums: Vec::new(),
            selected_album: self.selected_album.clone(),
            selected_album_tracks: Vec::new(),
            artists: Vec::new(),
            selected_artist: self.selected_artist.clone(),
            selected_artist_tracks: Vec::new(),
            selected_directory: self.selected_directory.clone(),
            selected_directory_tracks: Vec::new(),
        };

        // メタキャッシュから情報を取得
        if let Ok(cache) = MetaCache::open_default() {
            // ライブラリ統計（アルバム数、トラック数）
            if let Ok((album_count, track_count)) = cache.library_counts() {
                library.album_count = album_count;
                library.track_count = track_count;
            }
            
            // 最近追加されたトラック（最大20件）
            if let Ok(recent_locations) = cache.recent_locations(20) {
                library.recent_locations = recent_locations;
            }
            
            // 最近追加されたディレクトリ（最大20件）
            if let Ok(recent_directories) = cache.recent_directories(20) {
                library.recent_directories = recent_directories;
            }
            
            // ディレクトリツリー（最大深度20）
            if let Ok(directory_tree) = cache.library_directories(20) {
                library.directory_tree = directory_tree;
            }
            
            // アルバム一覧
            if let Ok(albums) = cache.library_albums() {
                library.albums = albums;
            }
            
            // アーティスト一覧（現在の表示モードに応じて）
            // artist_view_mode.key() は "album_artist" または "track_artist" を返す
            if let Ok(artists) = cache.library_artists_by_kind(self.artist_view_mode.key()) {
                library.artists = artists;
            }
            
            // 選択中のアルバムのトラック一覧
            if let Some(album) = self.selected_album.as_deref() {
                if let Ok(tracks) = cache.library_tracks_by_album_id(album) {
                    library.selected_album_tracks = tracks;
                }
            }
            
            // 選択中のアーティストのトラック一覧
            if let Some(artist) = self.selected_artist.as_deref() {
                if let Ok(tracks) =
                    cache.library_track_locations_by_artist_kind(self.artist_view_mode.key(), artist)
                {
                    library.selected_artist_tracks = tracks;
                }
            }
            
            // 選択中のディレクトリのトラック一覧
            if let Some(directory) = self.selected_directory.as_deref() {
                if let Ok(tracks) = cache.library_tracks_by_directory(directory) {
                    library.selected_directory_tracks = tracks;
                }
            }
        }

        library
    }

    /// アプリケーションのViewModelを更新します。
    /// 
    /// この関数は、アプリケーション状態が変更された際に呼び出され、
    /// UIに表示されるデータを最新の状態に更新します。
    /// 
    /// 処理の流れ：
    /// 1. 設定ファイルを再読み込み（言語設定などが変更されている可能性がある）
    /// 2. 現在の状態からLibraryDtoを構築
    /// 3. 新しいAppVmを生成
    /// 4. アルバムアートワークキャッシュをライブラリと同期
    /// 5. 状態をファイルに保存
    /// 
    /// この関数は以下のタイミングで呼び出されます：
    /// - タブ切り替え時
    /// - ライブラリ選択変更時
    /// - プレイリスト変更時
    /// - キュー変更時
    /// - スキャン設定変更時
    /// - 言語設定変更時
    pub(crate) fn update_app_vm(&mut self) {
        // 設定ファイルを再読み込み（言語設定の変更を反映）
        let cfg = Config::load();
        self.language = cfg.language;
        
        // 現在の状態からLibraryDtoを構築
        let library = self.library_dto();
        
        // 新しいAppVmを生成
        self.app_vm = Self::default_app_vm(
            self.active_tab,
            &cfg,
            self.last_scan_status.clone(),
            self.scan_targets.clone(),
            self.scan_lrc_targets.clone(),
            self.scan_excludes.clone(),
            library,
            &self.playlists,
            self.queue.clone(),
        );
        
        // アルバムアートワークキャッシュをライブラリと同期
        // 削除されたアルバムのキャッシュをクリーンアップする
        self.sync_album_artwork_cache_with_library();
        
        // 状態をファイルに保存
        self.save_state();
    }

    /// 再生タブに切り替えます。
    /// 
    /// この関数は、他のタブから再生操作が行われた際に呼び出され、
    /// 自動的に再生タブに遷移します。
    /// 
    /// 例：
    /// - ライブラリでトラックをダブルクリック
    /// - プレイリストで「再生」ボタンをクリック
    /// - キューでトラックを選択して再生
    /// 
    /// すでに再生タブが表示されている場合は何もしません。
    pub(crate) fn switch_to_play_tab(&mut self) {
        if self.active_tab != Tab::Play {
            self.active_tab = Tab::Play;
            self.update_app_vm();  // ViewModelを更新してUIに反映
        }
    }
}