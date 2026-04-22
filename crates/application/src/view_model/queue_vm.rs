#[derive(Debug, Clone)]
pub struct QueueVm {
    pub tracks: Vec<String>,
}

impl QueueVm {
    pub fn new(tracks: Vec<String>) -> Self {
        Self { tracks }
    }
}
