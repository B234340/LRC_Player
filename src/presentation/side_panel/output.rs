use crate::config::Config;
use crate::domain::model::PlaybackState;
use crate::localization::{language_name, supported_languages, translate};
use crate::presentation::egui_app::GuiApp;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    ui.horizontal(|ui| {
        let toggle_label = if app.side_panel_expanded { "<" } else { ">" };
        if ui.button(toggle_label).clicked() {
            app.side_panel_expanded = !app.side_panel_expanded;
            app.save_state();
        }
        if app.side_panel_expanded {
            ui.heading(translate("output_device", app.language));
        }
    });

    if !app.side_panel_expanded {
        return;
    }

    ui.separator();
    ui.heading("Playback Info");
    if app.play_path.trim().is_empty() {
        ui.label("No track selected.");
    } else {
        ui.label(format!("Path: {}", app.play_path));
        ui.label(format!("State: {}", app.playback_state));
        if let Some(status) = &app.play_status {
            ui.label(format!("Status: {}", status));
        }
    }
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        ui.label(translate("filter", app.language));
        ui.text_edit_singleline(&mut app.device_filter);
    });

    let filter = app.device_filter.to_lowercase();
    let default_label = translate("default_output_device_label", app.language);
    let mut filtered: Vec<(Option<usize>, &str)> = Vec::new();

    if filter.is_empty() || default_label.to_lowercase().contains(&filter) {
        filtered.push((None, default_label));
    }
    for (idx, name) in app.device_names.iter().enumerate() {
        if filter.is_empty() || name.to_lowercase().contains(&filter) {
            filtered.push((Some(idx), name.as_str()));
        }
    }

    if filtered.is_empty() {
        ui.label(translate("no_matching_devices", app.language));
    } else {
        let mut sel_in_filtered = 0usize;
        for (i, (orig_idx, _)) in filtered.iter().enumerate() {
            if *orig_idx == app.selected {
                sel_in_filtered = i;
                break;
            }
        }

        egui::ComboBox::from_id_source("device_combo")
            .selected_text(filtered.get(sel_in_filtered).map(|(_, s)| *s).unwrap_or(default_label))
            .show_ui(ui, |ui| {
                for (i, (_orig_idx, name)) in filtered.iter().enumerate() {
                    ui.selectable_value(&mut sel_in_filtered, i, *name);
                }
            });

        if let Some((orig_idx, _)) = filtered.get(sel_in_filtered) {
            let new_selected = *orig_idx;
            if app.selected != new_selected {
                app.selected = new_selected;
                if let Some(idx) = new_selected {
                    if let Some(name) = app.device_names.get(idx) {
                        let cfg = Config::load();
                        app.taps = cfg.taps_for_device(Some(name));
                    }
                }

                if !app.play_path.trim().is_empty()
                    && matches!(
                        app.playback_state,
                        PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking
                    )
                {
                    app.playback_state = PlaybackState::Buffering;
                    app.play_status = Some("Switching output device...".to_string());
                    app.request_play();
                }
            }
        }
    }

    if ui.button(translate("save_device", app.language)).clicked() {
        let mut cfg = Config::load();
        if let Some(idx) = app.selected {
            let name = app.device_names[idx].clone();
            cfg.preferred_output_device = Some(name.clone());
            if let Some(settings) = cfg.device_settings.get(&name) {
                app.taps = settings.resampler_taps;
            }
        } else {
            cfg.preferred_output_device = None;
        }
        let _ = cfg.save();
        app.update_app_vm();
    }

    ui.add_space(8.0);
    ui.label(translate("resampler_taps", app.language));
    ui.add(egui::DragValue::new(&mut app.taps).clamp_range(4..=256));
    if ui.button(translate("save_taps", app.language)).clicked() {
        let mut cfg = Config::load();
        if let Some(idx) = app.selected {
            let name = app.device_names[idx].clone();
            cfg.set_taps_for_device(name, app.taps);
        } else {
            cfg.resampler_taps = app.taps;
        }
        let _ = cfg.save();
        app.update_app_vm();
    }

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(translate("language", app.language));
        let mut selected_lang = app.language;
        egui::ComboBox::from_id_source("language_combo")
            .selected_text(language_name(app.language))
            .show_ui(ui, |ui| {
                for lang in supported_languages() {
                    ui.selectable_value(&mut selected_lang, *lang, language_name(*lang));
                }
            });

        if selected_lang != app.language {
            app.language = selected_lang;
        }
    });

    if ui.button("Save language").clicked() {
        let mut cfg = Config::load();
        cfg.language = app.language;
        let _ = cfg.save();
        app.update_app_vm();
    }
}
