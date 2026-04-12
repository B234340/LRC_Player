//! Core::queue — 再生キューとその振る舞い
//!
//! 再生キューはエントリの順序管理、現在位置、リピート・シャッフル振る舞いなどを管理する
//! 中核的なデータ構造を提供します。UI/コントローラはこの `Queue` を操作して再生順を制御します。

use serde::{Deserialize, Serialize};

use crate::core::ids::{QueueEntryId, TrackId};
use std::time::{SystemTime, UNIX_EPOCH};

/// リピート／シャッフルモード
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RepeatMode {
    /// 1 曲を繰り返す
    RepeatOne,
    /// キュー全体を順次再生（デフォルト）
    #[default]
    RepeatQueue,
    /// シャッフル再生
    ShuffleQueue,
}

/// キュー内の単一エントリ
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEntry {
    pub id: QueueEntryId,
    pub track_id: TrackId,
}

/// シャッフル時の内部状態（順序とカーソル、乱数シード）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShuffleState {
    /// シャッフル順を示すインデックス配列（元の entries インデックスを格納）
    pub order: Vec<u32>,
    /// 現在のカーソル位置（order 内の位置）
    pub cursor: u32,
    /// シャッフルの擬似乱数シード（再現性のため保存される）
    pub seed: u64,
}

/// 再生キュー本体
#[derive(Debug, Clone, Default)]
pub struct Queue {
    /// キュー内のエントリ配列
    pub entries: Vec<QueueEntry>,
    /// 現在の再生インデックス（None の場合は空）
    pub current_index: Option<u32>,
    /// UI で選択されたエントリ ID の集合
    pub selected: Vec<QueueEntryId>,
    /// リピート／シャッフルモード
    pub repeat_mode: RepeatMode,
    /// シャッフル時の内部状態（存在するのは ShuffleQueue モード時）
    pub shuffle_state: Option<ShuffleState>,
    /// 次に付与するエントリ ID（内部連番）
    next_entry_id: u32,
}

impl Queue {
    pub fn replace_with_tracks(&mut self, tracks: &[TrackId]) {
        self.entries = tracks
            .iter()
            .enumerate()
            .map(|(index, track_id)| QueueEntry {
                id: QueueEntryId(index as u32),
                track_id: *track_id,
            })
            .collect();
        self.current_index = if self.entries.is_empty() {
            None
        } else {
            Some(0)
        };
        self.selected.clear();
        self.next_entry_id = self.entries.len() as u32;
        self.shuffle_state = None;
    }

    pub fn set_current_index(&mut self, index: usize) {
        if index < self.entries.len() {
            self.current_index = Some(index as u32);
        }
    }

    pub fn current_track(&self) -> Option<TrackId> {
        self.current_index
            .and_then(|index| self.entries.get(index as usize))
            .map(|entry| entry.track_id)
    }

    pub fn push_track(&mut self, track_id: TrackId) {
        let id = QueueEntryId(self.next_entry_id);
        self.next_entry_id += 1;
        self.entries.push(QueueEntry { id, track_id });
        if self.current_index.is_none() {
            self.current_index = Some(0);
        }
        self.shuffle_state = None;
    }

    pub fn insert_after_current(&mut self, track_id: TrackId) {
        if self.entries.is_empty() {
            self.push_track(track_id);
            return;
        }

        let insert_at = self
            .current_index
            .map(|index| index as usize + 1)
            .unwrap_or(self.entries.len());
        let id = QueueEntryId(self.next_entry_id);
        self.next_entry_id += 1;
        self.entries.insert(insert_at, QueueEntry { id, track_id });
        self.shuffle_state = None;
    }

    pub fn remove_entry(&mut self, entry_id: QueueEntryId) -> Option<TrackId> {
        let remove_index = self.entries.iter().position(|entry| entry.id == entry_id)?;
        let removed = self.entries.remove(remove_index);
        self.selected.retain(|selected| *selected != entry_id);

        match self.current_index.map(|index| index as usize) {
            Some(_) if self.entries.is_empty() => self.current_index = None,
            Some(current) if current > remove_index => {
                self.current_index = Some((current - 1) as u32)
            }
            Some(current) if current == remove_index => {
                if remove_index < self.entries.len() {
                    self.current_index = Some(remove_index as u32);
                } else {
                    self.current_index =
                        self.entries.len().checked_sub(1).map(|index| index as u32);
                }
            }
            _ => {}
        }

        Some(removed.track_id)
    }

    fn ensure_shuffle_state(&mut self) {
        if self.repeat_mode != RepeatMode::ShuffleQueue {
            return;
        }

        let len = self.entries.len();
        if len == 0 {
            self.shuffle_state = None;
            return;
        }

        let seed = self
            .shuffle_state
            .as_ref()
            .map(|s| s.seed)
            .unwrap_or_else(|| {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                now
            });

        // initialize order
        let mut order: Vec<u32> = (0..len as u32).collect();

        // simple splitmix64 RNG
        let mut x = seed.wrapping_add(0x9E3779B97F4A7C15);
        let mut rng = || {
            x = x.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };

        // Fisher-Yates shuffle
        for i in (1..order.len()).rev() {
            let r = (rng() as usize) % (i + 1);
            order.swap(i, r);
        }

        // determine cursor position (place current_index at correct cursor)
        let cursor = self
            .current_index
            .map(|idx| idx as u32)
            .and_then(|cur_idx| order.iter().position(|&v| v == cur_idx).map(|p| p as u32))
            .unwrap_or(0);

        self.shuffle_state = Some(ShuffleState {
            order,
            cursor,
            seed,
        });
    }

    pub fn remove_selected(&mut self) {
        let mut selected = self.selected.clone();
        selected.sort_by_key(|entry_id| entry_id.0);
        selected.reverse();

        for entry_id in selected {
            let _ = self.remove_entry(entry_id);
        }
    }

    pub fn advance(&mut self) {
        if self.entries.is_empty() {
            self.current_index = None;
            return;
        }

        match self.repeat_mode {
            RepeatMode::RepeatOne => {
                // no change
            }
            RepeatMode::RepeatQueue => {
                let current = self.current_index.unwrap_or(0) as usize;
                let next = if current + 1 < self.entries.len() {
                    current + 1
                } else {
                    0
                };
                self.current_index = Some(next as u32);
            }
            RepeatMode::ShuffleQueue => {
                self.ensure_shuffle_state();
                if let Some(state) = self.shuffle_state.as_mut() {
                    let len = state.order.len();
                    if len == 0 {
                        self.current_index = None;
                        return;
                    }
                    state.cursor = (state.cursor + 1) % len as u32;
                    let idx = state.order[state.cursor as usize] as usize;
                    self.current_index = Some(idx as u32);
                }
            }
        }
    }

    pub fn previous(&mut self) {
        if self.entries.is_empty() {
            self.current_index = None;
            return;
        }

        match self.repeat_mode {
            RepeatMode::RepeatOne => {
                // no change
            }
            RepeatMode::RepeatQueue => {
                let current = self.current_index.unwrap_or(0) as usize;
                let previous = if current == 0 {
                    self.entries.len() - 1
                } else {
                    current - 1
                };
                self.current_index = Some(previous as u32);
            }
            RepeatMode::ShuffleQueue => {
                self.ensure_shuffle_state();
                if let Some(state) = self.shuffle_state.as_mut() {
                    let len = state.order.len();
                    if len == 0 {
                        self.current_index = None;
                        return;
                    }
                    state.cursor = if state.cursor == 0 {
                        (len - 1) as u32
                    } else {
                        state.cursor - 1
                    };
                    let idx = state.order[state.cursor as usize] as usize;
                    self.current_index = Some(idx as u32);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Queue, RepeatMode};
    use crate::core::ids::{QueueEntryId, TrackId};

    #[test]
    fn removing_current_track_moves_to_next_track() {
        let mut queue = Queue::default();
        queue.replace_with_tracks(&[TrackId(1), TrackId(2), TrackId(3)]);
        queue.set_current_index(1);

        let removed = queue.remove_entry(QueueEntryId(1));

        assert_eq!(removed, Some(TrackId(2)));
        assert_eq!(queue.current_track(), Some(TrackId(3)));
    }

    #[test]
    fn repeat_one_keeps_same_track() {
        let mut queue = Queue::default();
        queue.replace_with_tracks(&[TrackId(1), TrackId(2)]);
        queue.repeat_mode = RepeatMode::RepeatOne;

        queue.advance();

        assert_eq!(queue.current_track(), Some(TrackId(1)));
    }
}
