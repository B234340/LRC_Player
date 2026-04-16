use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct JsonStore {
    path: PathBuf,
}

impl JsonStore {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        Ok(JsonStore { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load<T>(&self) -> Result<T>
    where
        T: DeserializeOwned,
    {
        if !self.path.exists() {
            anyhow::bail!("file does not exist: {}", self.path.display());
        }
        let contents = fs::read_to_string(&self.path)
            .with_context(|| format!("reading json file {}", self.path.display()))?;
        let value = serde_json::from_str(&contents)
            .with_context(|| format!("parsing json file {}", self.path.display()))?;
        Ok(value)
    }

    pub fn save<T>(&self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating parent directory {}", parent.display()))?;
        }

        let tmp_path = self.path.with_extension("tmp");
        let mut tmp_file = File::create(&tmp_path)
            .with_context(|| format!("creating temp json file {}", tmp_path.display()))?;
        let contents = serde_json::to_string_pretty(value)
            .with_context(|| format!("serializing json for {}", self.path.display()))?;
        tmp_file.write_all(contents.as_bytes())?;
        tmp_file.sync_all()?;
        fs::rename(&tmp_path, &self.path)
            .with_context(|| format!("renaming {} to {}", tmp_path.display(), self.path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::tempdir;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct TestData {
        name: String,
        value: i32,
    }

    #[test]
    fn saves_and_loads_json_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.json");
        let store = JsonStore::open(&path).unwrap();

        let original = TestData {
            name: "hello".into(),
            value: 42,
        };
        store.save(&original).unwrap();

        let loaded: TestData = store.load().unwrap();
        assert_eq!(loaded, original);
    }
}
