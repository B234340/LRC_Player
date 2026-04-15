use std::path::Path;

pub struct JsonStore {
    path: String,
}

impl JsonStore {
    pub fn open<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref().to_string_lossy().to_string();
        Ok(JsonStore { path })
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}
