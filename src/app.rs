//! アプリケーション全体の薄いラッパー層
//!
//! - `AppFacade` と `JsonStore` をまとめ、UI と内部モデルの橋渡しを行う。
//! - 永続化の閾値判定（位置保存タイミング）やバックグラウンドタスクのポーリングを担う。
//!
//! 注: 実際のビジネスロジックは `app::facade` や `core` 下にあり、`Application` はそれらを統合する役割に徹しています。

pub mod commands;
pub mod events;
pub mod facade;
pub mod view_models;
pub mod track_row_factory;
pub mod vm_cache;

use self::facade::AppFacade;
use crate::app::commands::AppCommand;
use crate::app::events::{AppEvent, PlaybackEvent};
use crate::app::view_models::AppViewModel;

use crate::core::ids::TrackId;
use crate::core::persistence::{JsonStore, PersistencePaths};
use crate::core::playback::PlaybackState;
use crate::core::time::POSITION_SAVE_THRESHOLD_MS;
use crate::core::scan::ScanState;

/// `Application` は UI とアプリ内部ロジックの統合点。
///
/// - `facade`: 実際の操作ロジック（ライブラリ、キュー、再生制御など）を保持
/// - `store`: 永続化層へのアクセスを提供
/// - `last_saved_*`: 自動永続化のための直近保存時点のスナップショット
/// - `cached_view_model`: UI 描画用 ViewModel のキャッシュ
/// - `view_model_dirty`: キャッシュ再構築が必要かどうかのフラグ
pub struct Application {
    facade: AppFacade,
    store: JsonStore,
    last_saved_track: Option<TrackId>,
    last_saved_position_ms: u64,
    cached_view_model: Option<AppViewModel>,
    view_model_dirty: bool,
}

impl Default for Application {
    fn default() -> Self {
        let paths = PersistencePaths::for_app("lrc_player").unwrap_or_else(|_| PersistencePaths {
            config_json: "config.json".into(),
            state_json: "state.json".into(),
            playlists_json: "playlists.json".into(),
        });
        // アプリデータ領域に対する永続化パスを決定。
        // 失敗時はローカル相対パスにフォールバックする（テストや CI 向け）。
        // ログ初期化を呼び、必要があればログ出力先を作成する。
        let _ = crate::logging::init(&paths);

        // AppFacade と JsonStore を用いて Application を構築
        Self::with_store(AppFacade::default(), JsonStore::new(paths))
    }
}

impl Application {
    pub fn with_store(facade: AppFacade, store: JsonStore) -> Self {
        Self {
            last_saved_track: facade.playback.current_track,
            last_saved_position_ms: facade.playback.position_ms,
            facade,
            store,
            cached_view_model: None,
            view_model_dirty: true, // 初期状態ではキャッシュが無効
        }
    }

    pub fn initialize(&mut self) -> std::io::Result<()> {
        self.facade.bootstrap_from_store(&self.store)?;
        self.refresh_save_baseline();
        self.view_model_dirty = true;
        Ok(())
    }

    /// 永続化レイヤから初期状態を読み込み、内部状態を復元する。
    /// `bootstrap_from_store` の成功後に保存時点をリフレッシュする。

    pub fn persist(&mut self) -> std::io::Result<()> {
        self.facade.save_to_store(&self.store)?;
        self.refresh_save_baseline();
        Ok(())
    }

    /// 明示的にアプリ状態を永続化する。内部でエラーが発生すれば `Err` を返す。

    pub fn view_model(&mut self) -> &AppViewModel {
        if self.view_model_dirty || self.cached_view_model.is_none() {
            self.cached_view_model = Some(self.facade.view_model().clone());
            self.view_model_dirty = false;
        }
        self.cached_view_model.as_ref().unwrap()
    }

    /// UI 用の ViewModel を生成して返す。
    /// キャッシュが有効な場合はキャッシュを返し、無効な場合は再構築する。

    pub fn handle_command(&mut self, command: AppCommand) {
        let events = self.facade.handle_command(command);
        self.view_model_dirty = true; // コマンド処理によって状態が変更された可能性がある
        self.maybe_persist_after_events(&events);
    }

    /// UI からのコマンドを受け取り、`facade` に委譲する。
    /// 返ってきた `AppEvent` 列に応じて必要なら自動で永続化を行う。

    pub fn poll_background_tasks(&mut self) {
        let events = self.facade.poll_background_tasks();
        if !events.is_empty() {
            self.view_model_dirty = true;
        }
        self.maybe_persist_after_events(&events);
    }

    /// バックグラウンドワーカーのポーリングを行い、発生したイベントに応じて永続化を検討する。

    pub fn bootstrap_summary(&self) -> String {
        self.facade.bootstrap_summary()
    }

    pub fn run_demo(&mut self) {
        self.facade.load_demo_state();
        self.view_model_dirty = true;
    }

    pub fn facade(&self) -> &AppFacade {
        &self.facade
    }

    pub fn facade_mut(&mut self) -> &mut AppFacade {
        &mut self.facade
    }

    fn maybe_persist_after_events(&mut self, events: &[AppEvent]) {
        let mut save_for_track_change = false;
        let mut save_for_position = false;
        let mut save_for_scan = false;
        let mut save_for_settings = false;

        // 発生したイベント列を走査し、永続化が必要か判定する。
        for event in events {
            if let AppEvent::Playback(playback_event) = event {
                match playback_event {
                    PlaybackEvent::TrackChanged(track) => {
                        if *track != self.last_saved_track {
                            save_for_track_change = true;
                        }
                    }
                    PlaybackEvent::StateChanged(PlaybackState::Paused) => {
                        save_for_position = true;
                    }
                    PlaybackEvent::PositionUpdated { .. } => {
                        save_for_position = true;
                    }
                    _ => {}
                }
            }
            if let AppEvent::ScanStateChanged(state) = event {
                // スキャン完了時はインデックス更新が終わっているため保存対象とする
                if *state == ScanState::Completed {
                    save_for_scan = true;
                }
            }
            if let AppEvent::SettingsChanged = event {
                save_for_settings = true;
            }
        }

        // 永続化判定ロジック：トラック変更、位置差分が閾値を超えた場合、スキャン完了、設定変更
        let should_persist =
            save_for_track_change
                || (save_for_position && self.should_persist_position())
                || save_for_scan
                || save_for_settings;

        if should_persist {
            if let Err(error) = self.persist() {
                // 永続化失敗は致命ではないがログに出力しておく
                eprintln!("failed to persist application state: {error}");
            }
        }
    }

    fn should_persist_position(&self) -> bool {
        self.last_saved_track != self.facade.playback.current_track
            || self
                .facade
                .playback
                .position_ms
                .abs_diff(self.last_saved_position_ms)
                >= POSITION_SAVE_THRESHOLD_MS
    }

    fn refresh_save_baseline(&mut self) {
        self.last_saved_track = self.facade.playback.current_track;
        self.last_saved_position_ms = self.facade.playback.position_ms;
    }
}

// Inline unit tests moved to `tests/app_tests.rs` as integration tests.