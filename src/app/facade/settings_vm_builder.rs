// Settings VM builder: extracted from `AppFacade` to reduce facade size.
use std::sync::Arc;

use crate::app::view_models::{SettingItemVm, SettingsTabVm};

pub fn build_settings_tab(facade: &super::AppFacade) -> SettingsTabVm {
    SettingsTabVm {
        items: vec![
            SettingItemVm {
                key: "accent_color".to_string(),
                value: facade.settings.accent_color.clone(),
            },
            SettingItemVm {
                key: "theme".to_string(),
                value: facade.settings.theme.clone(),
            },
            SettingItemVm {
                key: "seek_step_ms".to_string(),
                value: facade.settings.seek_step_ms.to_string(),
            },
            SettingItemVm {
                key: "restore_position_policy".to_string(),
                value: facade.settings.restore_position_policy.clone(),
            },
            SettingItemVm {
                key: "full_auto_scan".to_string(),
                value: facade.settings.full_auto_scan.to_string(),
            },
            SettingItemVm {
                key: "fallback_artwork_path".to_string(),
                value: facade
                    .settings
                    .fallback_artwork_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "none".to_string()),
            },
            SettingItemVm {
                key: "play_empty_state".to_string(),
                value: facade.settings.play_empty_state.clone(),
            },
        ],
        available_output_devices: crate::audio::output::AudioDeviceOutput::list_output_devices(),
        preferred_output_device: facade.settings.preferred_output_device.clone(),
        debug_mode: facade.settings.debug_mode,
        available_log_categories: crate::logging::available_log_categories(),
        selected_log_filters: facade.settings.log_filters.clone(),
    }
}
