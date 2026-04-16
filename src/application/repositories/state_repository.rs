use crate::application::dto::StateDto;
use crate::infrastructure::persistence::state_file::StateFile;
use anyhow::Result;

/// Application-level repository for persistent state.
pub trait StateRepository {
    fn load_state(&self) -> Result<StateDto>;
    fn save_state(&self, state: &StateDto) -> Result<()>;
}

impl StateRepository for StateFile {
    fn load_state(&self) -> Result<StateDto> {
        self.load()
    }

    fn save_state(&self, state: &StateDto) -> Result<()> {
        self.save(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn state_repository_roundtrips_state() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        let store = StateFile::open_with_path(path).unwrap();

        let original = StateDto {
            last_tab: Some("scan".into()),
        };
        store.save(&original).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.last_tab, original.last_tab);
    }
}
