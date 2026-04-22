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
    pub(crate) fn handle_play_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
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

        if space_pressed {
            self.toggle_play_pause();
        }

        let now = Instant::now();

        let Some(duration_ms) = self.play_duration_ms else {
            self.set_seek_interacting(false);
            self.seek_key_hold_direction = 0;
            self.seek_key_hold_started_at = None;
            self.seek_key_last_step_at = None;
            return;
        };

        if left_pressed && !right_pressed {
            self.step_seek_by(-1, 10_000, duration_ms);
            self.seek_key_hold_direction = -1;
            self.seek_key_hold_started_at = Some(now);
            self.seek_key_last_step_at = Some(now);
            self.set_seek_interacting(false);
        } else if right_pressed && !left_pressed {
            self.step_seek_by(1, 10_000, duration_ms);
            self.seek_key_hold_direction = 1;
            self.seek_key_hold_started_at = Some(now);
            self.seek_key_last_step_at = Some(now);
            self.set_seek_interacting(false);
        }

        let hold_direction = match (left_down, right_down) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };

        if hold_direction == 0 {
            self.set_seek_interacting(false);
            self.seek_key_hold_direction = 0;
            self.seek_key_hold_started_at = None;
            self.seek_key_last_step_at = None;
            return;
        }

        if self.seek_key_hold_direction != hold_direction {
            self.seek_key_hold_direction = hold_direction;
            self.seek_key_hold_started_at = Some(now);
            self.seek_key_last_step_at = Some(now);
            self.set_seek_interacting(false);
        }

        let hold_delay = Duration::from_millis(350);
        let repeat_every = Duration::from_millis(120);
        let started_at = self.seek_key_hold_started_at.unwrap_or(now);
        let hold_elapsed = now.duration_since(started_at);

        if hold_elapsed < hold_delay {
            ctx.request_repaint_after(hold_delay - hold_elapsed);
            return;
        }

        self.set_seek_interacting(true);

        let can_step = self
            .seek_key_last_step_at
            .map(|last| now.duration_since(last) >= repeat_every)
            .unwrap_or(true);

        if can_step {
            self.step_seek_by(hold_direction, 1_000, duration_ms);
            self.seek_key_last_step_at = Some(now);
        }

        ctx.request_repaint_after(repeat_every);
    }

    pub(crate) fn queue_seek_position(&mut self, position_ms: u64) {
        play::queue_seek_position(self, position_ms);
    }

    pub(crate) fn flush_pending_seek(&mut self) {
        let Some(target_ms) = self.pending_seek_ms else {
            return;
        };

        if self.seek_interacting {
            return;
        }

        let debounce = Duration::from_millis(150);
        if let Some(last_change) = self.seek_last_change_at {
            if last_change.elapsed() < debounce {
                return;
            }
        }

        self.pending_seek_ms = None;
        self.seek_last_change_at = None;
        self.play_position_ms = target_ms;

        if self.playback_state != PlaybackState::Stopped {
            self.seek_to_position();
        }
    }

    pub(crate) fn step_seek_by(&mut self, direction: i8, step_ms: u64, duration_ms: u64) {
        let next = if direction < 0 {
            self.play_position_ms.saturating_sub(step_ms)
        } else {
            self.play_position_ms.saturating_add(step_ms).min(duration_ms)
        };
        if next != self.play_position_ms {
            self.queue_seek_position(next);
        }
    }

    pub(crate) fn set_seek_interacting(&mut self, interacting: bool) {
        self.seek_interacting = interacting;
    }

    pub(crate) fn is_seek_interacting(&self) -> bool {
        self.seek_interacting
    }

    pub(crate) fn set_seek_long_press_start(&mut self, start: Option<Instant>) {
        self.seek_long_press_start = start;
    }

    pub(crate) fn seek_long_press_elapsed(&self) -> Option<Duration> {
        self.seek_long_press_start.map(|start| start.elapsed())
    }
}
