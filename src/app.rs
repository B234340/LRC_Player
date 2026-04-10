mod layout;
mod tabs;
mod types;
mod utils;

use self::types::AppTab;
use self::utils::configure_fonts;

pub struct LrcPlayerApp {
    _active_tab: AppTab,
}

impl LrcPlayerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_fonts(&cc.egui_ctx);

        Self {
            _active_tab: AppTab::Play,
        }
    }
}
