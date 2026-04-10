pub mod commands;
pub mod events;
pub mod facade;
pub mod view_models;

use self::facade::AppFacade;

pub struct Application {
    facade: AppFacade,
}

impl Default for Application {
    fn default() -> Self {
        Self {
            facade: AppFacade::default(),
        }
    }
}

impl Application {
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
