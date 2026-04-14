// Minimal persistence skeleton. Implement SQLite-backed meta cache here.
pub struct MetaCache;

impl MetaCache {
    pub fn new(path: &str) -> Self {
        let _ = path;
        MetaCache
    }

    pub fn init(&self) -> Result<(), String> { Ok(()) }
}
