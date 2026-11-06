//! Queue navigation rules (next / previous / gapless preload) for shuffle + repeat.
//!
//! Pure logic with no I/O so the GUI's transport controls and the engine's gapless
//! `set_next` preload share exactly one rule set.
//!
//! * Shuffle uses a persistent play order (a permutation of queue indices) so "Previous"
//!   in shuffle goes back to the track actually played before, and every track plays once
//!   per cycle. The order is regenerated (current track first) when shuffle is switched on
//!   or the queue length changes; [`QueueNavigator::invalidate`] forces it after edits.
//! * `RepeatMode::One` only affects automatic advance (track end); Next/Previous still move
//!   (wrapping like `RepeatMode::All`).
//! * At the end of a shuffled cycle with `RepeatMode::All` the same order is replayed, which
//!   keeps the gapless preload (computed before the boundary) and the real advance identical.

use crate::dto::RepeatMode;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

/// Restart the current track instead of going back when Previous is pressed this far in.
pub const PREVIOUS_RESTART_THRESHOLD_MS: u64 = 3_000;

/// What the Previous button should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviousAction {
    /// Seek the current track to 0.
    RestartCurrent,
    /// Play the queue item at this index.
    PlayIndex(usize),
}

#[derive(Debug, Clone)]
pub struct QueueNavigator {
    shuffle: bool,
    repeat: RepeatMode,
    /// Shuffled play order (queue indices); empty when not shuffling.
    order: Vec<usize>,
    rng: StdRng,
}

impl Default for QueueNavigator {
    fn default() -> Self {
        Self::new(false, RepeatMode::Off)
    }
}

impl QueueNavigator {
    pub fn new(shuffle: bool, repeat: RepeatMode) -> Self {
        Self {
            shuffle,
            repeat,
            order: Vec::new(),
            rng: StdRng::from_entropy(),
        }
    }

    /// Deterministic shuffle order (tests).
    pub fn with_seed(shuffle: bool, repeat: RepeatMode, seed: u64) -> Self {
        Self {
            shuffle,
            repeat,
            order: Vec::new(),
            rng: StdRng::seed_from_u64(seed),
        }
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = repeat;
    }

    /// Enables/disables shuffle. Turning it on builds a new order starting at `current`.
    pub fn set_shuffle(&mut self, on: bool, current: Option<usize>, len: usize) {
        if on && (!self.shuffle || self.order.len() != len) {
            self.shuffle = true;
            self.regenerate(current, len);
        } else if !on {
            self.shuffle = false;
            self.order.clear();
        }
    }

    /// Call after the queue was edited (insert / remove / reorder / replace).
    pub fn invalidate(&mut self) {
        self.order.clear();
    }

    /// Current shuffled play order (empty when not shuffling or not built yet).
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    fn regenerate(&mut self, current: Option<usize>, len: usize) {
        let mut order: Vec<usize> = (0..len).collect();
        order.shuffle(&mut self.rng);
        if let Some(cur) = current.filter(|&c| c < len) {
            if let Some(pos) = order.iter().position(|&i| i == cur) {
                order.remove(pos);
                order.insert(0, cur);
            }
        }
        self.order = order;
    }

    /// Position of `current` in the shuffle order, (re)building the order when needed.
    fn shuffle_pos(&mut self, current: usize, len: usize) -> usize {
        if self.order.len() != len {
            self.regenerate(Some(current), len);
        }
        match self.order.iter().position(|&i| i == current) {
            Some(pos) => pos,
            None => {
                self.regenerate(Some(current), len);
                0
            }
        }
    }

    fn first(&mut self, len: usize) -> Option<usize> {
        if len == 0 {
            return None;
        }
        if self.shuffle {
            if self.order.len() != len {
                self.regenerate(None, len);
            }
            self.order.first().copied()
        } else {
            Some(0)
        }
    }

    /// Track to play when `current` ends on its own (also what to preload for gapless).
    pub fn next_auto(&mut self, current: Option<usize>, len: usize) -> Option<usize> {
        let Some(cur) = current.filter(|&c| c < len) else {
            return self.first(len);
        };
        if self.repeat == RepeatMode::One {
            return Some(cur);
        }
        self.step_forward(cur, len, self.repeat == RepeatMode::All)
    }

    /// Track for the Next button (repeat-one does not pin the track here).
    pub fn next_manual(&mut self, current: Option<usize>, len: usize) -> Option<usize> {
        let Some(cur) = current.filter(|&c| c < len) else {
            return self.first(len);
        };
        self.step_forward(cur, len, self.repeat != RepeatMode::Off)
    }

    /// Track for the Previous button (ignoring the restart threshold).
    pub fn previous_index(&mut self, current: Option<usize>, len: usize) -> Option<usize> {
        let Some(cur) = current.filter(|&c| c < len) else {
            return self.first(len);
        };
        let wrap = self.repeat != RepeatMode::Off;
        if self.shuffle {
            let pos = self.shuffle_pos(cur, len);
            if pos > 0 {
                Some(self.order[pos - 1])
            } else if wrap {
                self.order.last().copied()
            } else {
                None
            }
        } else if cur > 0 {
            Some(cur - 1)
        } else if wrap {
            Some(len - 1)
        } else {
            None
        }
    }

    /// Previous button: restart the track when more than 3 s in (or when there is nothing
    /// before it), otherwise go to the previous track.
    pub fn previous(&mut self, current: Option<usize>, len: usize, position_ms: u64) -> PreviousAction {
        if position_ms > PREVIOUS_RESTART_THRESHOLD_MS {
            return PreviousAction::RestartCurrent;
        }
        match self.previous_index(current, len) {
            Some(index) if Some(index) != current => PreviousAction::PlayIndex(index),
            _ => PreviousAction::RestartCurrent,
        }
    }

    fn step_forward(&mut self, cur: usize, len: usize, wrap: bool) -> Option<usize> {
        if self.shuffle {
            let pos = self.shuffle_pos(cur, len);
            if pos + 1 < len {
                Some(self.order[pos + 1])
            } else if wrap {
                self.order.first().copied()
            } else {
                None
            }
        } else if cur + 1 < len {
            Some(cur + 1)
        } else if wrap {
            Some(0)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_stops_at_end() {
        let mut nav = QueueNavigator::new(false, RepeatMode::Off);
        assert_eq!(nav.next_auto(Some(0), 3), Some(1));
        assert_eq!(nav.next_auto(Some(2), 3), None);
        assert_eq!(nav.next_manual(Some(2), 3), None);
        assert_eq!(nav.next_auto(None, 3), Some(0));
        assert_eq!(nav.next_auto(None, 0), None);
    }

    #[test]
    fn repeat_all_wraps_both_ways() {
        let mut nav = QueueNavigator::new(false, RepeatMode::All);
        assert_eq!(nav.next_auto(Some(2), 3), Some(0));
        assert_eq!(nav.previous_index(Some(0), 3), Some(2));
    }

    #[test]
    fn repeat_one_pins_auto_advance_only() {
        let mut nav = QueueNavigator::new(false, RepeatMode::One);
        assert_eq!(nav.next_auto(Some(1), 3), Some(1));
        assert_eq!(nav.next_manual(Some(1), 3), Some(2));
        assert_eq!(nav.next_manual(Some(2), 3), Some(0));
    }

    #[test]
    fn previous_restarts_after_three_seconds() {
        let mut nav = QueueNavigator::new(false, RepeatMode::Off);
        assert_eq!(nav.previous(Some(2), 3, 3_001), PreviousAction::RestartCurrent);
        assert_eq!(nav.previous(Some(2), 3, 3_000), PreviousAction::PlayIndex(1));
        assert_eq!(nav.previous(Some(0), 3, 500), PreviousAction::RestartCurrent);
    }

    #[test]
    fn shuffle_visits_every_track_once_and_previous_walks_back() {
        let mut nav = QueueNavigator::with_seed(false, RepeatMode::Off, 42);
        nav.set_shuffle(true, Some(3), 8);
        assert_eq!(nav.order()[0], 3);
        let mut seen = vec![3];
        let mut cur = 3;
        while let Some(next) = nav.next_auto(Some(cur), 8) {
            seen.push(next);
            cur = next;
        }
        let mut sorted = seen.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..8).collect::<Vec<_>>());
        // Walk back through the same order.
        for expected in seen.iter().rev().skip(1) {
            let prev = nav.previous_index(Some(cur), 8).unwrap();
            assert_eq!(prev, *expected);
            cur = prev;
        }
        assert_eq!(nav.previous_index(Some(cur), 8), None);
    }

    #[test]
    fn shuffle_repeat_all_replays_same_order_and_preload_matches_advance() {
        let mut nav = QueueNavigator::with_seed(true, RepeatMode::All, 7);
        nav.set_shuffle(true, Some(0), 5);
        let order = nav.order().to_vec();
        let last = *order.last().unwrap();
        let preload = nav.next_auto(Some(last), 5);
        assert_eq!(preload, Some(order[0]));
        // Asking again gives the same answer (gapless preload == actual advance).
        assert_eq!(nav.next_auto(Some(last), 5), preload);
    }

    #[test]
    fn queue_length_change_rebuilds_order_with_current_first() {
        let mut nav = QueueNavigator::with_seed(true, RepeatMode::Off, 1);
        nav.set_shuffle(true, Some(1), 4);
        let _ = nav.next_auto(Some(1), 6);
        assert_eq!(nav.order().len(), 6);
        assert_eq!(nav.order()[0], 1);
    }

    #[test]
    fn disabling_shuffle_returns_to_sequential() {
        let mut nav = QueueNavigator::with_seed(true, RepeatMode::Off, 3);
        nav.set_shuffle(true, Some(0), 4);
        nav.set_shuffle(false, Some(0), 4);
        assert!(nav.order().is_empty());
        assert_eq!(nav.next_auto(Some(0), 4), Some(1));
    }
}
