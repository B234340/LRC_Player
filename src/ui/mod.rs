pub mod tabs;

use crate::app::view_models::AppViewModel;

#[derive(Debug, Clone)]
pub struct UiShell {
    pub view_model: AppViewModel,
}

impl UiShell {
    pub fn new(view_model: AppViewModel) -> Self {
        Self { view_model }
    }
}