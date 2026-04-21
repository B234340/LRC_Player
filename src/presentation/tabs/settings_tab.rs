use crate::config::{Config, LogLevel, Theme};
use crate::presentation::app::GuiApp;
use crate::localization::{translate};
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    ui.heading(translate("settings_header", app.language));
    ui.label(translate("settings_description", app.language));
    ui.separator();

    ui.label(format!(
        "{} {}",
        translate("output_device", app.language),
        app.app_vm
            .settings
            .preferred_output_device
            .as_deref()
            .unwrap_or("<default>")
    ));
    ui.label(format!("{} {}", translate("resampler_taps", app.language), app.taps));

    ui.separator();
    ui.label(translate("theme", app.language));
    egui::ComboBox::from_id_source("theme_combo")
        .selected_text(match app.theme {
            Theme::System => translate("theme_system", app.language),
            Theme::Light => translate("theme_light", app.language),
            Theme::Dark => translate("theme_dark", app.language),
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut app.theme, Theme::System, translate("theme_system", app.language));
            ui.selectable_value(&mut app.theme, Theme::Light, translate("theme_light", app.language));
            ui.selectable_value(&mut app.theme, Theme::Dark, translate("theme_dark", app.language));
        });
    if ui.button(translate("save_theme", app.language)).clicked() {
        let mut cfg = Config::load();
        cfg.theme = app.theme;
        let _ = cfg.save();
    }

    ui.separator();
    ui.label(translate("log_level", app.language));
    egui::ComboBox::from_id_source("log_level_combo")
        .selected_text(match app.log_level {
            LogLevel::Error => translate("log_level_error", app.language),
            LogLevel::Warn => translate("log_level_warn", app.language),
            LogLevel::Info => translate("log_level_info", app.language),
            LogLevel::Debug => translate("log_level_debug", app.language),
            LogLevel::Trace => translate("log_level_trace", app.language),
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut app.log_level, LogLevel::Error, translate("log_level_error", app.language));
            ui.selectable_value(&mut app.log_level, LogLevel::Warn, translate("log_level_warn", app.language));
            ui.selectable_value(&mut app.log_level, LogLevel::Info, translate("log_level_info", app.language));
            ui.selectable_value(&mut app.log_level, LogLevel::Debug, translate("log_level_debug", app.language));
            ui.selectable_value(&mut app.log_level, LogLevel::Trace, translate("log_level_trace", app.language));
        });
    if ui.button(translate("save_log_level", app.language)).clicked() {
        let mut cfg = Config::load();
        cfg.log_level = app.log_level;
        let _ = cfg.save();
    }

    ui.separator();
    ui.label(translate("keyboard_shortcuts", app.language));
    ui.group(|ui| {
        ui.label("Space: Play/Pause");
        ui.label("Arrow Left/Right: Seek backward/forward");
        ui.label("Ctrl+S: Save current playlist");
        ui.label("Ctrl+O: Open library scan dialog");
    });
}
