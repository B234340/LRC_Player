use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtworkKey {
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtworkHandle {
    pub key: ArtworkKey,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
pub struct ArtworkCache {
    capacity: usize,
    order: VecDeque<ArtworkKey>,
    entries: HashMap<ArtworkKey, ArtworkHandle>,
}

impl ArtworkCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            order: VecDeque::new(),
            entries: HashMap::new(),
        }
    }

    pub fn get(&mut self, key: &ArtworkKey) -> Option<ArtworkHandle> {
        let handle = self.entries.get(key).cloned()?;
        self.order.retain(|existing| existing != key);
        self.order.push_back(key.clone());
        Some(handle)
    }

    pub fn insert(&mut self, handle: ArtworkHandle) {
        if self.capacity == 0 {
            return;
        }

        if self.entries.contains_key(&handle.key) {
            self.order.retain(|existing| existing != &handle.key);
        }

        self.order.push_back(handle.key.clone());
        self.entries.insert(handle.key.clone(), handle);

        while self.entries.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}