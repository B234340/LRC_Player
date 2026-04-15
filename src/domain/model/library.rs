use crate::domain::model::track::Track;

#[derive(Debug, Default, Clone)]
pub struct Library {
    pub tracks: Vec<Track>,
}

impl Library {
    pub fn new() -> Self {
        Self { tracks: Vec::new() }
    }
}
