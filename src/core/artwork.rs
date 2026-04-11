use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtworkKey {
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtworkHandle {
    pub key: ArtworkKey,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Serialize, Deserialize)]
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

    /// Save the artwork cache to the given directory as JSON.
    pub fn save_to_dir<P: AsRef<Path>>(&self, dir: P) -> io::Result<()> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        let file = dir.join("artwork_cache.json");
        let contents = serde_json::to_vec_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        fs::write(file, contents)?;
        Ok(())
    }

    /// Load the artwork cache from the given directory. If the file does not exist,
    /// returns an empty cache with the given capacity.
    pub fn load_from_dir<P: AsRef<Path>>(dir: P, capacity: usize) -> io::Result<Self> {
        let dir = dir.as_ref();
        let file = dir.join("artwork_cache.json");
        if !file.exists() {
            return Ok(ArtworkCache::new(capacity));
        }
        let data = fs::read(&file)?;
        let cache: ArtworkCache = serde_json::from_slice(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(cache)
    }
}
