#[derive(Debug, Clone)]
pub struct Queue {
    pub track_ids: Vec<String>,
    pub position: Option<usize>,
}

impl Queue {
    pub fn new() -> Self { Self { track_ids: Vec::new(), position: None } }
}
