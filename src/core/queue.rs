use serde::{Deserialize, Serialize};

use crate::core::ids::{QueueEntryId, TrackId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RepeatMode {
    RepeatOne,
    #[default]
    RepeatQueue,
    ShuffleQueue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEntry {
    pub id: QueueEntryId,
    pub track_id: TrackId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShuffleState {
    pub order: Vec<u32>,
    pub cursor: u32,
    pub seed: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Queue {
    pub entries: Vec<QueueEntry>,
    pub current_index: Option<u32>,
    pub selected: Vec<QueueEntryId>,
    pub repeat_mode: RepeatMode,
    pub shuffle_state: Option<ShuffleState>,
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
        self.current_index = if self.entries.is_empty() { None } else { Some(0) };
        self.selected.clear();
        self.next_entry_id = self.entries.len() as u32;
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
    }

    pub fn insert_after_current(&mut self, track_id: TrackId) {
        if self.entries.is_empty() {
            self.push_track(track_id);
            return;
        }

        let insert_at = self.current_index.map(|index| index as usize + 1).unwrap_or(self.entries.len());
        let id = QueueEntryId(self.next_entry_id);
        self.next_entry_id += 1;
        self.entries.insert(insert_at, QueueEntry { id, track_id });
    }

    pub fn remove_entry(&mut self, entry_id: QueueEntryId) -> Option<TrackId> {
        let remove_index = self.entries.iter().position(|entry| entry.id == entry_id)?;
        let removed = self.entries.remove(remove_index);
        self.selected.retain(|selected| *selected != entry_id);

        match self.current_index.map(|index| index as usize) {
            Some(_) if self.entries.is_empty() => self.current_index = None,
            Some(current) if current > remove_index => self.current_index = Some((current - 1) as u32),
            Some(current) if current == remove_index => {
                if remove_index < self.entries.len() {
                    self.current_index = Some(remove_index as u32);
                } else {
                    self.current_index = self.entries.len().checked_sub(1).map(|index| index as u32);
                }
            }
            _ => {}
        }

        Some(removed.track_id)
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

        let current = self.current_index.unwrap_or(0) as usize;
        let next = match self.repeat_mode {
            RepeatMode::RepeatOne => current,
            RepeatMode::RepeatQueue | RepeatMode::ShuffleQueue => {
                if current + 1 < self.entries.len() { current + 1 } else { 0 }
            }
        };
        self.current_index = Some(next as u32);
    }

    pub fn previous(&mut self) {
        if self.entries.is_empty() {
            self.current_index = None;
            return;
        }

        let current = self.current_index.unwrap_or(0) as usize;
        let previous = if current == 0 { self.entries.len() - 1 } else { current - 1 };
        self.current_index = Some(previous as u32);
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