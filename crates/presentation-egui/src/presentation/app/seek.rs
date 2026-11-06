//! Seeking and global transport shortcuts.
//!
//! Seeks are cheap with the persistent engine (container seek, no re-decode), so requests are
//! only debounced briefly ([`SEEK_DEBOUNCE`]) to coalesce slider drags. Keyboard seeking adds
//! to the *live* position (or the still-pending target), so holding an arrow key scans
//! forward/backward faster than real time instead of lagging behind it.

use std::time::{Duration, Instant};

use eframe::egui;
use lrc_player_domain::model::PlaybackState;

use crate::presentation::app::GuiApp;

/// Debounce for seek requests (slider drags / key repeats).
pub(crate) const SEEK_DEBOUNCE: Duration = Duration::from_millis(30);
/// Single arrow key press.
pub(crate) const SEEK_STEP_MS: u64 = 5_000;
/// Delay before a held arrow key starts scanning.
const HOLD_DELAY: Duration = Duration::from_millis(350);
/// Scan step interval while an arrow key is held.
const HOLD_REPEAT: Duration = Duration::from_millis(120);
/// First scan step; grows by `HOLD_STEP_GROWTH_MS` per step up to `HOLD_STEP_MAX_MS`.
const HOLD_STEP_START_MS: u64 = 2_000;
const HOLD_STEP_GROWTH_MS: u64 = 1_000;
const HOLD_STEP_MAX_MS: u64 = 10_000;

/// Step for the `n`-th repeat of a held arrow key (accelerating, always faster than real time).
pub(crate) fn hold_step_ms(steps_done: u32) -> u64 {
    (HOLD_STEP_START_MS + HOLD_STEP_GROWTH_MS * steps_done as u64).min(HOLD_STEP_MAX_MS)
}

impl GuiApp {
    /// Global shortcuts (any tab, ignored while a text field has keyboard focus):
    /// - **Space**: play / pause
    /// - **← / →**: seek 5 s; hold to scan (2 s steps every 120 ms, accelerating to 10 s)
    pub(crate) fn handle_global_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.text_edit_focused() {
            self.reset_key_hold();
            return;
        }
        let (space_pressed, left_pressed, right_pressed, left_down, right_down) = ctx.input(|input| {
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
        if self.play_duration_ms.is_none() && self.play_path.is_empty() {
            self.reset_key_hold();
            return;
        }

        // Key press (also delivered as OS auto-repeat; the hold logic below ignores repeats).
        if left_pressed && !right_pressed && self.seek_key_hold_direction != -1 {
            self.seek_relative(-(SEEK_STEP_MS as i64));
            self.begin_key_hold(-1, now);
        } else if right_pressed && !left_pressed && self.seek_key_hold_direction != 1 {
            self.seek_relative(SEEK_STEP_MS as i64);
            self.begin_key_hold(1, now);
        }

        let hold_direction = match (left_down, right_down) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        if hold_direction == 0 {
            self.reset_key_hold();
            return;
        }
        if self.seek_key_hold_direction != hold_direction {
            self.begin_key_hold(hold_direction, now);
        }

        let started_at = self.seek_key_hold_started_at.unwrap_or(now);
        let held = now.duration_since(started_at);
        if held < HOLD_DELAY {
            ctx.request_repaint_after(HOLD_DELAY - held);
            return;
        }
        let due = self
            .seek_key_last_step_at
            .is_none_or(|last| now.duration_since(last) >= HOLD_REPEAT);
        if due {
            let step = hold_step_ms(self.seek_key_hold_steps) as i64;
            self.seek_relative(step * hold_direction as i64);
            self.seek_key_hold_steps = self.seek_key_hold_steps.saturating_add(1);
            self.seek_key_last_step_at = Some(now);
        }
        ctx.request_repaint_after(HOLD_REPEAT);
    }

    fn begin_key_hold(&mut self, direction: i8, now: Instant) {
        self.seek_key_hold_direction = direction;
        self.seek_key_hold_started_at = Some(now);
        // The first repeat happens HOLD_DELAY + HOLD_REPEAT after the press.
        self.seek_key_last_step_at = Some(now + HOLD_DELAY);
        self.seek_key_hold_steps = 0;
    }

    fn reset_key_hold(&mut self) {
        self.seek_key_hold_direction = 0;
        self.seek_key_hold_started_at = None;
        self.seek_key_last_step_at = None;
        self.seek_key_hold_steps = 0;
    }

    /// Seek to `position_ms` (clamped to the duration). Debounced by [`SEEK_DEBOUNCE`]; the
    /// target is shown immediately.
    pub fn seek_to(&mut self, position_ms: u64) {
        let target = match self.play_duration_ms {
            Some(d) => position_ms.min(d),
            None => position_ms,
        };
        self.play_position_ms = target;
        self.pending_seek_ms = Some(target);
        self.seek_last_change_at = Some(Instant::now());
    }

    /// Seek relative to the current (or still pending) position.
    pub fn seek_relative(&mut self, delta_ms: i64) {
        let base = self.pending_seek_ms.unwrap_or(self.play_position_ms);
        let target = if delta_ms < 0 {
            base.saturating_sub(delta_ms.unsigned_abs())
        } else {
            base.saturating_add(delta_ms as u64)
        };
        self.seek_to(target);
    }

    /// Sends a debounced seek once the user paused for [`SEEK_DEBOUNCE`] (and is not
    /// dragging the slider). Called every frame from `poll_engine`.
    pub(crate) fn flush_pending_seek(&mut self) {
        if self.pending_seek_ms.is_none() || self.seek_interacting {
            return;
        }
        if self
            .seek_last_change_at
            .is_some_and(|last| last.elapsed() < SEEK_DEBOUNCE)
        {
            return;
        }
        self.flush_seek_now();
    }

    /// Sends any pending seek immediately (before pause/resume so it is not lost).
    pub(crate) fn flush_seek_now(&mut self) {
        let Some(target) = self.pending_seek_ms.take() else {
            return;
        };
        self.seek_last_change_at = None;
        self.play_position_ms = target;
        if self.engine_has_current_track() {
            if let Some(engine) = &self.engine {
                engine.seek(target);
            }
            self.seek_display_hold = Some((target, Instant::now()));
        } else if self.playback_state != PlaybackState::Stopped {
            // The engine has no track loaded (e.g. right after startup): the position is used
            // on the next `play()`.
            self.playback_state = PlaybackState::Stopped;
        }
        self.save_state();
    }

    /// The seek slider is being dragged / held (position updates and seeks are deferred).
    pub(crate) fn set_seek_interacting(&mut self, interacting: bool) {
        self.seek_interacting = interacting;
    }

    pub(crate) fn is_seek_interacting(&self) -> bool {
        self.seek_interacting
    }
}
