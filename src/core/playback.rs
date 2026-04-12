//! Core::playback — 再生状態と playback ロジックの小さなモデル
//!
//! 再生の現在状態、現在トラック、位置、バッファ量、音量など再生に関する状態を表す
//! 軽量なデータ構造を提供します。UI 層や `AppFacade` がこれを読み書きして再生を制御します。

use serde::{Deserialize, Serialize};

use crate::core::ids::TrackId;

/// 再生状態の列挙
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlaybackState {
    /// 再生停止中（デフォルト）
    #[default]
    Stopped,
    /// 再生準備（バッファリング中）
    Buffering,
    /// 再生中
    Playing,
    /// 一時停止
    Paused,
    /// シーク中（内部的な遷移用）
    Seeking,
}

/// 再生に関する現在の状態を保持するモデル
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackModel {
    /// 現在の再生状態
    pub state: PlaybackState,
    /// 現在再生中（あるいは最後に設定された）トラック ID
    pub current_track: Option<TrackId>,
    /// 現在の再生位置（ミリ秒）
    pub position_ms: u64,
    /// 再生バッファの長さ（ミリ秒）
    pub buffered_ms: u32,
    /// 音量（0.0〜1.0）
    pub volume: f32,
    /// シーク後に復帰すべき状態を保持する内部フィールド
    /// シーク中に一時的に使うフィールド（外部から操作しない）。
    /// シーク完了後にこの状態へ復帰します。
    resume_after_seek: PlaybackState,
}

impl Default for PlaybackModel {
    fn default() -> Self {
        Self {
            state: PlaybackState::Stopped,
            current_track: None,
            position_ms: 0,
            buffered_ms: 0,
            volume: 1.0,
            resume_after_seek: PlaybackState::Playing,
        }
    }
}

impl PlaybackModel {
    /// 指定トラックの再生を開始する（位置は 0 にリセット）
    /// 指定トラックの再生を開始する（位置は 0 にリセット）。
    /// 内部では一旦 `Buffering` にしてから `Playing` へ遷移します。
    pub fn play_track(&mut self, track_id: TrackId) {
        self.current_track = Some(track_id);
        self.position_ms = 0;
        self.buffered_ms = 0;
        self.state = PlaybackState::Buffering;
        self.state = PlaybackState::Playing;
    }

    /// 一時停止/再開のトグル
    /// Playing ↔ Paused を切り替える。`Buffering`/`Seeking` の場合は状態を維持し、
    /// `Stopped` は影響を受けません（呼び出し側が意味を評価する）。
    pub fn toggle_pause(&mut self) {
        self.state = match self.state {
            PlaybackState::Playing => PlaybackState::Paused,
            PlaybackState::Paused => PlaybackState::Playing,
            PlaybackState::Buffering | PlaybackState::Seeking => self.state,
            PlaybackState::Stopped => PlaybackState::Stopped,
        };
    }

    /// 絶対位置へシークする
    /// 指定した絶対位置へシークする。シーク中は一時的に `Seeking` に入り、
    /// 完了後は呼び出し前の適切な状態へ復帰します（再生中なら Playing、
    /// ポーズ中なら Paused）。
    pub fn seek_to(&mut self, position_ms: u64) {
        // シーク後に戻すべき状態を決定
        self.resume_after_seek = match self.state {
            PlaybackState::Paused => PlaybackState::Paused,
            _ => PlaybackState::Playing,
        };
        // 実際のシーク遷移（ここではモデル上の状態更新のみ）
        self.state = PlaybackState::Seeking;
        self.position_ms = position_ms;
        // モデル上は外部のバッファ完了イベントがないため即復帰する
        self.state = self.resume_after_seek;
    }

    /// 相対シークを行う（正負両対応）
    pub fn seek_by(&mut self, delta_ms: i64) {
        let next_position = if delta_ms.is_negative() {
            self.position_ms.saturating_sub(delta_ms.unsigned_abs())
        } else {
            self.position_ms.saturating_add(delta_ms as u64)
        };
        self.seek_to(next_position);
    }

    /// 永続化されたトラック情報を復元して一時停止状態に入る
    pub fn restore_track(&mut self, track_id: TrackId, position_ms: u64) {
        self.current_track = Some(track_id);
        self.position_ms = position_ms;
        self.buffered_ms = 0;
        self.state = PlaybackState::Paused;
        self.resume_after_seek = PlaybackState::Paused;
    }

    /// 再生を停止し、状態を初期化する
    pub fn stop(&mut self) {
        self.state = PlaybackState::Stopped;
        self.current_track = None;
        self.position_ms = 0;
        self.buffered_ms = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{PlaybackModel, PlaybackState};
    use crate::core::ids::TrackId;

    #[test]
    fn seek_returns_to_paused_state() {
        let mut playback = PlaybackModel::default();
        playback.play_track(TrackId(10));
        playback.toggle_pause();

        playback.seek_to(42_000);

        assert_eq!(playback.state, PlaybackState::Paused);
        assert_eq!(playback.position_ms, 42_000);
    }

    #[test]
    fn restore_track_enters_paused_state() {
        let mut playback = PlaybackModel::default();

        playback.restore_track(TrackId(7), 12_345);

        assert_eq!(playback.current_track, Some(TrackId(7)));
        assert_eq!(playback.position_ms, 12_345);
        assert_eq!(playback.state, PlaybackState::Paused);
    }
}
