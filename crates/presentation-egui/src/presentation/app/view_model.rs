//! ViewModelの構築を担当
//! - AppVmの生成
//! - LibraryDtoの構築

use lrc_player_application::config::Config;
use lrc_player_application::dto::{LibraryDto, PlaylistDto, SettingsDto, StateDto};
use lrc_player_application::infrastructure::library_snapshot::LibrarySelection;
use lrc_player_application::usecases::build_app_vm::build_app_vm;
use lrc_player_application::view_model::AppVm;

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
            last_scan_status,
            scan_targets,
            scan_lrc_targets,
            scan_excludes,
            ..Default::default()
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
    pub(crate) fn library_dto(&mut self) -> LibraryDto {
        let selection = LibrarySelection {
            favorite_directories: self.favorite_directories.clone(),
            selected_album: self.selected_album.clone(),
            selected_artist: self.selected_artist.clone(),
            selected_directory: self.selected_directory.clone(),
        };
        let empty = || LibraryDto {
            album_count: 0,
            track_count: 0,
            recent_locations: Vec::new(),
            recent_directories: Vec::new(),
            directory_tree: Vec::new(),
            favorite_directories: selection.favorite_directories.clone(),
            albums: Vec::new(),
            selected_album: selection.selected_album.clone(),
            selected_album_tracks: Vec::new(),
            artists: Vec::new(),
            selected_artist: selection.selected_artist.clone(),
            selected_artist_tracks: Vec::new(),
            selected_directory: selection.selected_directory.clone(),
            selected_directory_tracks: Vec::new(),
        };
        let kind = self.artist_view_mode.key();
        let Some(meta) = &self.meta else {
            return empty();
        };
        // The shared connection is only used from the UI thread (workers open their own),
        // so this does not wait on other threads; queries run only when the DB changed.
        let snapshot = &mut self.library_snapshot;
        match meta.with(|cache| snapshot.library_dto(cache, kind, &selection)) {
            Ok(dto) => dto,
            Err(err) => {
                tracing::warn!("loading library data failed: {err:#}");
                empty()
            }
        }
    }

    /// アプリケーションのViewModelを更新します。
    ///
    /// 安価: ライブラリは [`LibrarySnapshotCache`] から（DB が変わった時だけ再クエリ）、
    /// 設定ファイルの再読込みや同期的な state.json 書き込みは行いません。
    /// キュー変更後のギャップレス次曲の再計算もここで行います。
    ///
    /// [`LibrarySnapshotCache`]: lrc_player_application::infrastructure::library_snapshot::LibrarySnapshotCache
    pub(crate) fn update_app_vm(&mut self) {
        let library = self.library_dto();
        let cfg = Config {
            preferred_output_device: self.preferred_output_device.clone(),
            resampler_taps: self.taps,
            ..Config::default()
        };
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
        self.app_vm.scan.scan_scanned = self.scan_counts.0;
        self.app_vm.scan.scan_parsed = self.scan_counts.1;
        self.app_vm.scan.scan_registered = self.scan_counts.2;
        self.rebuild_favorite_set();
        self.update_gapless_next();
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