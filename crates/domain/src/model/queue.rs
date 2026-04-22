#[derive(Debug, Clone, Default)]
pub struct Queue {
    pub track_ids: Vec<String>,
    pub position: Option<usize>,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }
}
