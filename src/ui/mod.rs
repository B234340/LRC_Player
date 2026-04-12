//! UI 層の公開エントリ
//!
//! `UiShell` は UI のスナップショット（`AppViewModel`）をラップする軽量ホルダです。
pub mod egui_app;
pub mod tabs;

use crate::app::view_models::AppViewModel;

#[derive(Debug, Clone)]
pub struct UiShell {
    /// 現在の ViewModel スナップショット
    pub view_model: AppViewModel,
}

impl UiShell {
    pub fn new(view_model: AppViewModel) -> Self {
        Self { view_model }
    }
}
