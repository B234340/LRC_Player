pub struct FileScanner;

impl FileScanner {
    pub fn new() -> Self {
        Self
    }
    pub fn enumerate_paths(&self, _path: &std::path::Path) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
}
