//! シーク操作の管理を担当
//! - キーボードシーク（長押し、リピート）
//! - シークデバウンス
//! - シーク状態管理

use std::time::{Duration, Instant};

use eframe::egui;
use lrc_player_domain::model::PlaybackState;

use crate::presentation::actions::play;
use crate::presentation::app::GuiApp;

impl GuiApp {
    /// 再生タブでのキーボードショートカットを処理します。
    /// 
    /// サポートされているショートカット：
    /// - **Space**: 再生/一時停止のトグル
    /// - **← / → 矢印キー**: シーク操作
    /// 
    /// 矢印キーによるシークの動作：
    /// - **単押し**: 10秒スキップ
    /// - **長押し**:
    ///   1. 最初の350msは待機（誤操作防止）
    ///   2. その後120ms間隔で1秒ずつ連続シーク
    /// 
    /// この関数は毎フレーム呼び出され、キー状態をチェックして
    /// 適切なシーク動作を実行します。
    /// 
    /// # 引数
    /// * `ctx` - eguiコンテキスト（キー入力取得と再描画要求用）
    pub(crate) fn handle_play_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        // 現在のキー状態を取得
        // - key_pressed: このフレームで押下された瞬間のみtrue
        // - key_down: 押下されている間ずっとtrue
        let (space_pressed, left_pressed, right_pressed, left_down, right_down) =
            ctx.input(|input| {
                (
                    input.key_pressed(egui::Key::Space),
                    input.key_pressed(egui::Key::ArrowLeft),
                    input.key_pressed(egui::Key::ArrowRight),
                    input.key_down(egui::Key::ArrowLeft),
                    input.key_down(egui::Key::ArrowRight),
                )
            });

        // Space: 再生/一時停止トグル
        if space_pressed {
            self.toggle_play_pause();
        }

        let now = Instant::now();

        // 総再生時間が不明な場合はシーク処理をスキップ
        let Some(duration_ms) = self.play_duration_ms else {
            self.set_seek_interacting(false);
            self.seek_key_hold_direction = 0;
            self.seek_key_hold_started_at = None;
            self.seek_key_last_step_at = None;
            return;
        };

        // 矢印キーの単押し処理（最初の5秒シーク）
        // 左右同時押しの場合は無視
        if left_pressed && !right_pressed {
            self.step_seek_by(-1, 5_000, duration_ms);
            // ホールド状態を初期化（長押し検出用）
            self.seek_key_hold_direction = -1;
            self.seek_key_hold_started_at = Some(now);
            self.seek_key_last_step_at = Some(now);
            self.set_seek_interacting(false);
        } else if right_pressed && !left_pressed {
            self.step_seek_by(1, 5_000, duration_ms);
            // ホールド状態を初期化（長押し検出用）
            self.seek_key_hold_direction = 1;
            self.seek_key_hold_started_at = Some(now);
            self.seek_key_last_step_at = Some(now);
            self.set_seek_interacting(false);
        }

        // 現在のホールド方向を判定
        // -1: 左（戻る）
        //  1: 右（進む）
        //  0: なし
        let hold_direction = match (left_down, right_down) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };

        // ホールドなしの場合、状態をリセットして終了
        if hold_direction == 0 {
            self.set_seek_interacting(false);
            self.seek_key_hold_direction = 0;
            self.seek_key_hold_started_at = None;
            self.seek_key_last_step_at = None;
            return;
        }

        // ホールド方向が変わった場合、状態をリセット
        // （例：左から右に切り替え）
        if self.seek_key_hold_direction != hold_direction {
            self.seek_key_hold_direction = hold_direction;
            self.seek_key_hold_started_at = Some(now);
            self.seek_key_last_step_at = Some(now);
            self.set_seek_interacting(false);
        }

        // 長押しの設定値
        let hold_delay = Duration::from_millis(350);     // 長押し判定までの待機時間
        let repeat_every = Duration::from_millis(120);   // 連続シークの間隔
        
        // ホールド開始からの経過時間を計算
        let started_at = self.seek_key_hold_started_at.unwrap_or(now);
        let hold_elapsed = now.duration_since(started_at);

        // まだ長押し待機時間に達していない場合
        if hold_elapsed < hold_delay {
            // 待機時間の残り時間後に再描画を要求
            ctx.request_repaint_after(hold_delay - hold_elapsed);
            return;
        }

        // 長押し状態に突入
        self.set_seek_interacting(true);

        // シーク間隔が経過しているかチェック
        let can_step = self
            .seek_key_last_step_at
            .map(|last| now.duration_since(last) >= repeat_every)
            .unwrap_or(true);

        // シーク実行
        if can_step {
            self.step_seek_by(hold_direction, 0_050, duration_ms); // 0.05秒シーク
            self.seek_key_last_step_at = Some(now);
        }

        // 次のシークタイミングで再描画を要求
        ctx.request_repaint_after(repeat_every);
    }

    /// シーク位置をキューに設定します。
    /// 
    /// この関数は即座にシークを実行するのではなく、シーク要求を
    /// キューに入れます。実際のシークは`flush_pending_seek()`によって
    /// デバウンス処理された後に実行されます。
    /// 
    /// これにより、スライダードラッグ中などの連続的なシーク要求による
    /// 過剰な処理を抑制します。
    /// 
    /// # 引数
    /// * `position_ms` - シーク先の位置（ミリ秒単位）
    pub(crate) fn queue_seek_position(&mut self, position_ms: u64) {
        play::queue_seek_position(self, position_ms);
    }

    /// 保留中のシーク要求を実行します。
    /// 
    /// 以下の条件がすべて満たされた場合にのみ、実際のシークを実行します：
    /// 1. 保留中のシーク要求が存在する
    /// 2. シーク操作中でない（`seek_interacting == false`）
    /// 3. 最後のシーク位置変更から150ms以上経過している（デバウンス）
    /// 
    /// シーク実行後、再生状態が停止中でない場合は`seek_to_position()`を呼び出します。
    /// 停止中の場合は位置情報のみを更新します。
    /// 
    /// この関数は`poll_playback_status()`から毎フレーム呼び出されます。
    pub(crate) fn flush_pending_seek(&mut self) {
        // 保留中のシーク要求がなければ終了
        let Some(target_ms) = self.pending_seek_ms else {
            return;
        };

        // シーク操作中（スライダードラッグ中など）は実行しない
        if self.seek_interacting {
            return;
        }

        // デバウンス: 最後の変更から150ms待機
        // これにより連続的なシーク要求のほとんどを無視し、最後の要求のみを処理する
        let debounce = Duration::from_millis(150);
        if let Some(last_change) = self.seek_last_change_at {
            if last_change.elapsed() < debounce {
                return;
            }
        }

        // シーク要求をクリア
        self.pending_seek_ms = None;
        self.seek_last_change_at = None;
        
        // 再生位置を更新
        self.play_position_ms = target_ms;

        // 停止中以外の場合のみシークを実行
        // 停止中は位置だけ更新して、再生開始時にその位置から開始されるようにする
        if self.playback_state != PlaybackState::Stopped {
            self.seek_to_position();
        }
    }

    /// 指定された方向とステップ幅でシーク位置を変更します。
    /// 
    /// シーク位置は以下の制約に従います：
    /// - 0未満にはならない（負の方向の場合は0で停止）
    /// - 総再生時間を超えない（正の方向の場合は総再生時間でクランプ）
    /// 
    /// 位置が実際に変更された場合のみ、`queue_seek_position()`を呼び出します。
    /// 
    /// # 引数
    /// * `direction` - シーク方向（負: 戻る, 正: 進む）
    /// * `step_ms` - シークステップ幅（ミリ秒）
    /// * `duration_ms` - トラックの総再生時間（ミリ秒）
    pub(crate) fn step_seek_by(&mut self, direction: i8, step_ms: u64, duration_ms: u64) {
        // 次のシーク位置を計算
        let next = if direction < 0 {
            // 負の方向: 減算（0未満にならないようにサチュレーティング減算）
            self.play_position_ms.saturating_sub(step_ms)
        } else {
            // 正の方向: 加算（総再生時間を超えないようにクランプ）
            self.play_position_ms.saturating_add(step_ms).min(duration_ms)
        };
        
        // 位置が変わった場合のみシークを要求
        if next != self.play_position_ms {
            self.queue_seek_position(next);
        }
    }

    /// シーク操作中かどうかを設定します。
    /// 
    /// シーク操作中フラグが`true`の場合：
    /// - 保留中のシークは実行されない（`flush_pending_seek`）
    /// - 再生位置のクロック更新が一時停止される場合がある
    /// 
    /// 主にスライダードラッグ中やキーボード長押しシーク中に使用されます。
    /// 
    /// # 引数
    /// * `interacting` - シーク操作中なら`true`
    pub(crate) fn set_seek_interacting(&mut self, interacting: bool) {
        self.seek_interacting = interacting;
    }

    /// シーク操作中かどうかを返します。
    /// 
    /// # 戻り値
    /// シーク操作中なら`true`、そうでなければ`false`
    pub(crate) fn is_seek_interacting(&self) -> bool {
        self.seek_interacting
    }

    /// シークの長押し開始時刻を設定します。
    /// 
    /// この関数はUIのシークボタン（長押しで連続シーク）の状態管理に使用されます。
    /// 
    /// # 引数
    /// * `start` - 長押し開始時刻（`None`で長押し解除）
    pub(crate) fn set_seek_long_press_start(&mut self, start: Option<Instant>) {
        self.seek_long_press_start = start;
    }

    /// シーク長押しの経過時間を返します。
    /// 
    /// 長押しが開始されていない場合は`None`を返します。
    /// 
    /// # 戻り値
    /// 長押し開始からの経過時間。長押し中でない場合は`None`
    pub(crate) fn seek_long_press_elapsed(&self) -> Option<Duration> {
        self.seek_long_press_start.map(|start| start.elapsed())
    }
}