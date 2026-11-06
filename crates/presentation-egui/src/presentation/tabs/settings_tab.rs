use lrc_player_application::config::{Config, LogLevel, Theme};
use crate::presentation::app::GuiApp;
use lrc_player_application::localization::{translate};
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
    if let Some(device) = &app.active_output_device {
        ui.label(format!("Active device: {}", device));
    }

    ui.separator();
    ui.label(translate("theme", app.language));
    let previous_theme = app.theme;
    egui::ComboBox::from_id_salt("theme_combo")
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
    // Applied immediately; the button persists it.
    if app.theme != previous_theme {
        crate::presentation::app::apply_theme(ui.ctx(), app.theme);
    }
    if ui.button(translate("save_theme", app.language)).clicked() {
        let mut cfg = Config::load();
        cfg.theme = app.theme;
        if let Err(err) = cfg.save() {
            tracing::warn!("saving config failed: {err:#}");
        }
    }

    ui.separator();
    ui.label(translate("log_level", app.language));
    egui::ComboBox::from_id_salt("log_level_combo")
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
        if let Err(err) = cfg.save() {
            tracing::warn!("saving config failed: {err:#}");
        }
    }
    ui.label("Log level changes apply on the next start (RUST_LOG overrides it).");

    ui.separator();
    ui.label(translate("keyboard_shortcuts", app.language));
    ui.group(|ui| {
        ui.label("Space: Play/Pause");
        ui.label("Arrow Left/Right: Seek 5 s backward/forward (hold to scan)");
    });
}
