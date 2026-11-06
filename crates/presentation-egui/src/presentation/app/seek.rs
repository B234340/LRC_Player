//! Seeking.
//!
//! Seeks are cheap with the persistent engine (container seek, no re-decode), so requests are
//! only debounced briefly ([`SEEK_DEBOUNCE`]) to coalesce slider drags and key repeats.
//! Relative seeks (←/→, media keys) add to the *live* position or the still-pending target, so
//! a held arrow key (OS auto-repeat) scans faster than real time instead of lagging behind.

use std::time::{Duration, Instant};

use lrc_player_domain::model::PlaybackState;

use crate::presentation::app::GuiApp;

/// Debounce for seek requests (slider drags / key repeats).
pub(crate) const SEEK_DEBOUNCE: Duration = Duration::from_millis(30);

impl GuiApp {
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
        if self.pending_seek_ms.is_none() {
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
}
