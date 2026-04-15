#[derive(Debug, Clone)]
pub struct AppVm {
    pub title: String,
}

impl AppVm {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
        }
    }
}
