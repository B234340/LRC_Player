//! Queue editing used by the queue panel, track context menus and drag & drop.
//!
//! Every edit keeps `current_queue_index` pointing at the playing entry and tells the
//! engine the (possibly changed) gapless successor.

use lrc_player_domain::model::PlaybackState;

use crate::presentation::app::GuiApp;
use crate::presentation::ui::{widgets::ToastKind, Page, UiAction};

/// Index of `current` after moving `from` to insertion index `to` (`apply_reorder` semantics).
pub(crate) fn remap_after_move(current: usize, from: usize, to: usize) -> usize {
    let insert_at = if to > from { to - 1 } else { to };
    if current == from {
        insert_at
    } else if from < current && current <= insert_at {
        current - 1
    } else if insert_at <= current && current < from {
        current + 1
    } else {
        current
    }
}

impl GuiApp {
    /// Call after any queue edit.
    pub(crate) fn on_queue_changed(&mut self) {
        self.navigator.invalidate();
        if self.navigator.shuffle() {
            let len = self.queue.len();
            self.navigator.set_shuffle(true, self.current_queue_index, len);
        }
        self.update_gapless_next();
        self.save_state();
    }

    /// Replaces the queue with `paths` and plays `start`.
    pub(crate) fn play_tracks(&mut self, paths: Vec<String>, start: usize) {
        let paths: Vec<String> = paths.into_iter().filter(|p| !p.trim().is_empty()).collect();
        if paths.is_empty() {
            return;
        }
        let start = start.min(paths.len() - 1);
        self.queue = paths;
        self.current_queue_index = None;
        self.navigator.invalidate();
        if self.navigator.shuffle() {
            let len = self.queue.len();
            self.navigator.set_shuffle(true, Some(start), len);
        }
        self.start_queue_index(start, true);
        self.update_gapless_next();
    }

    /// Inserts `paths` right after the current track (or at the front when idle).
    pub(crate) fn play_next_tracks(&mut self, paths: Vec<String>) {
        let paths: Vec<String> = paths.into_iter().filter(|p| !p.trim().is_empty()).collect();
        if paths.is_empty() {
            return;
        }
        let count = paths.len();
        let at = self.current_queue_index.map(|i| i + 1).unwrap_or(0).min(self.queue.len());
        self.queue.splice(at..at, paths);
        if let Some(cur) = self.current_queue_index {
            if cur >= at {
                self.current_queue_index = Some(cur + count);
            }
        }
        self.on_queue_changed();
        let tr = self.tr();
        self.toast_with_action(ToastKind::Success, tr.f("toast.play_next", &[&count]), tr.t("toast.show_queue"), UiAction::Navigate(Page::Queue));
    }

    /// Appends to the queue with a toast.
    pub(crate) fn enqueue_with_toast(&mut self, paths: Vec<String>) {
        let count = paths.len();
        if count == 0 {
            return;
        }
        let single = (count == 1).then(|| self.library.display_row(&paths[0]).title);
        self.enqueue(paths);
        let tr = self.tr();
        let msg = match single {
            Some(title) => tr.f("toast.enqueued_one", &[&title]),
            None => tr.f("toast.enqueued_many", &[&count]),
        };
        self.toast_with_action(ToastKind::Success, msg, tr.t("toast.show_queue"), UiAction::Navigate(Page::Queue));
    }

    /// Inserts `paths` before queue index `at` (drag & drop onto the queue).
    pub(crate) fn queue_insert(&mut self, paths: Vec<String>, at: usize) {
        let paths: Vec<String> = paths.into_iter().filter(|p| !p.trim().is_empty()).collect();
        if paths.is_empty() {
            return;
        }
        let count = paths.len();
        let at = at.min(self.queue.len());
        self.queue.splice(at..at, paths);
        if let Some(cur) = self.current_queue_index {
            if cur >= at {
                self.current_queue_index = Some(cur + count);
            }
        }
        self.on_queue_changed();
    }

    pub(crate) fn queue_reorder(&mut self, from: usize, to: usize) {
        if from >= self.queue.len() {
            return;
        }
        crate::presentation::ui::views::apply_reorder(&mut self.queue, from, to);
        self.current_queue_index = self.current_queue_index.map(|c| remap_after_move(c, from, to));
        self.on_queue_changed();
    }

    pub(crate) fn queue_remove(&mut self, index: usize) {
        if index >= self.queue.len() {
            return;
        }
        let was_current = self.current_queue_index == Some(index);
        self.queue.remove(index);
        if was_current {
            if self.queue.is_empty() {
                self.current_queue_index = None;
                self.clear_current_track();
            } else {
                let next = index.min(self.queue.len() - 1);
                match self.playback_state {
                    PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => {
                        self.start_queue_index(next, true)
                    }
                    PlaybackState::Paused => self.start_queue_index(next, false),
                    PlaybackState::Stopped => {
                        if let Some(path) = self.queue.get(next).cloned() {
                            self.stop();
                            self.on_track_changed(Some(next), path);
                            self.play_position_ms = 0;
                        }
                    }
                }
            }
        } else if let Some(cur) = self.current_queue_index {
            if cur > index {
                self.current_queue_index = Some(cur - 1);
            }
        }
        self.on_queue_changed();
    }

    /// Empties the queue; the current track keeps playing (it is just no longer queued).
    pub(crate) fn queue_clear(&mut self) {
        self.queue.clear();
        self.current_queue_index = None;
        self.on_queue_changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::ui::views::apply_reorder;

    #[test]
    fn current_index_follows_reorder() {
        for len in 1..6usize {
            for from in 0..len {
                for to in 0..=len {
                    for cur in 0..len {
                        let mut v: Vec<usize> = (0..len).collect();
                        apply_reorder(&mut v, from, to);
                        let new = remap_after_move(cur, from, to);
                        assert_eq!(v[new], cur, "len={len} from={from} to={to} cur={cur}");
                    }
                }
            }
        }
    }
}
