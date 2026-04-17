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


