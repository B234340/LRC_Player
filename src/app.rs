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
pub struct Application {
    facade: AppFacade,
    store: JsonStore,
    last_saved_track: Option<TrackId>,
    last_saved_position_ms: u64,
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
    fn with_store(facade: AppFacade, store: JsonStore) -> Self {
        Self {
            last_saved_track: facade.playback.current_track,
            last_saved_position_ms: facade.playback.position_ms,
            facade,
            store,
        }
    }

    pub fn initialize(&mut self) -> std::io::Result<()> {
        self.facade.bootstrap_from_store(&self.store)?;
        self.refresh_save_baseline();
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

    pub fn view_model(&mut self) -> AppViewModel {
        self.facade.view_model()
    }

    /// UI 用の ViewModel を生成して返す。

    pub fn handle_command(&mut self, command: AppCommand) {
        let events = self.facade.handle_command(command);
        self.maybe_persist_after_events(&events);
    }

    /// UI からのコマンドを受け取り、`facade` に委譲する。
    /// 返ってきた `AppEvent` 列に応じて必要なら自動で永続化を行う。

    pub fn poll_background_tasks(&mut self) {
        let events = self.facade.poll_background_tasks();
        self.maybe_persist_after_events(&events);
    }

    /// バックグラウンドワーカーのポーリングを行い、発生したイベントに応じて永続化を検討する。

    pub fn bootstrap_summary(&self) -> String {
        self.facade.bootstrap_summary()
    }

    pub fn run_demo(&mut self) {
        self.facade.load_demo_state();
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

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::Application;
    use crate::app::commands::AppCommand;
    use crate::app::facade::AppFacade;
    use crate::core::ids::TrackId;
    use crate::core::library::TrackInput;
    use crate::core::persistence::{AppStateFile, JsonStore, PersistencePaths};

    fn temp_paths() -> PersistencePaths {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("lrc-player-app-test-{unique}"));
        PersistencePaths {
            config_json: root.join("config.json"),
            state_json: root.join("state.json"),
            playlists_json: root.join("playlists.json"),
        }
    }

    fn build_application() -> (Application, JsonStore) {
        let paths = temp_paths();
        let store = JsonStore::new(paths);
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
        facade.queue.replace_with_tracks(&[TrackId(0)]);
        facade.queue.set_current_index(0);
        facade.playback.play_track(TrackId(0));

        (Application::with_store(facade, store.clone()), store)
    }

    #[test]
    fn pause_persists_when_position_threshold_is_met() {
        let (mut application, store) = build_application();
        application.facade_mut().playback.position_ms = 5_000;

        application.handle_command(AppCommand::TogglePlayPause);

        let state = store.load_state().expect("state should load");
        assert_eq!(state.playback_position_ms, 5_000);
    }

    #[test]
    fn seek_does_not_persist_when_below_threshold() {
        let (mut application, store) = build_application();
        application.facade_mut().playback.position_ms = 5_000;
        application.handle_command(AppCommand::TogglePlayPause);

        application.handle_command(AppCommand::SeekTo { position_ms: 6_000 });

        let state = store.load_state().expect("state should load");
        assert_eq!(state.playback_position_ms, 5_000);
    }

    #[test]
    fn seek_persists_when_threshold_is_met() {
        let (mut application, store) = build_application();
        application.facade_mut().playback.position_ms = 5_000;
        application.handle_command(AppCommand::TogglePlayPause);

        application.handle_command(AppCommand::SeekTo { position_ms: 9_000 });

        let state: AppStateFile = store.load_state().expect("state should load");
        assert_eq!(state.playback_position_ms, 9_000);
    }
}
