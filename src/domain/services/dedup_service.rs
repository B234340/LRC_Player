pub struct DedupService;

impl DedupService {
    pub fn new() -> Self {
        Self
    }
    pub fn schedule_full_hash(&self, _path: &str) {
        // skeleton: enqueue hashing job
    }
}
