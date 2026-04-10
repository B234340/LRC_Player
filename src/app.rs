pub mod commands;
pub mod events;
pub mod facade;
pub mod view_models;

use crate::app::commands::AppCommand;
use crate::app::view_models::AppViewModel;
use self::facade::AppFacade;

use crate::core::persistence::{JsonStore, PersistencePaths};

pub struct Application {
    facade: AppFacade,
    store: JsonStore,
}

impl Default for Application {
    fn default() -> Self {
        let paths = PersistencePaths::for_app("lrc_player").unwrap_or_else(|_| PersistencePaths {
            config_json: "config.json".into(),
            state_json: "state.json".into(),
            playlists_json: "playlists.json".into(),
        });
        Self {
            facade: AppFacade::default(),
            store: JsonStore::new(paths),
        }
    }
}

impl Application {
    pub fn initialize(&mut self) -> std::io::Result<()> {
        self.facade.bootstrap_from_store(&self.store)
    }

    pub fn persist(&self) -> std::io::Result<()> {
        self.facade.save_to_store(&self.store)
    }

    pub fn view_model(&self) -> AppViewModel {
        self.facade.view_model()
    }

    pub fn handle_command(&mut self, command: AppCommand) {
        let _ = self.facade.handle_command(command);
    }

    pub fn bootstrap_summary(&self) -> String {
        self.facade.bootstrap_summary()
    }

    pub fn run_demo(&mut self) {
        self.facade.load_demo_state();
    }

    pub fn facade(&self) -> &AppFacade {
        &self.facade
    }

    pub fn facade_mut(&mut self) -> &mut AppFacade {
        &mut self.facade
    }
}
