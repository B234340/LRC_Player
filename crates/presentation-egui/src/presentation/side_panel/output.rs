use crate::presentation::app::GuiApp;
use eframe::egui;
use lrc_player_application::config::{Config, ResamplerSetting};
use lrc_player_application::localization::{language_name, supported_languages, translate};

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
        if let Some(title) = &app.play_title {
            ui.label(format!("Title: {}", title));
        }
        if let Some(artist) = &app.play_artist {
            ui.label(format!("Artist: {}", artist));
        }
        if let Some(album) = &app.play_album {
            ui.label(format!("Album: {}", album));
        }
        if let Some(index) = app.selected_queue_index {
            if let Some(track) = app.app_vm.queue.tracks.get(index) {
                ui.label(format!("Selected queue item: {}", track));
            }
        } else {
            ui.label("No queue item selected.");
        }
    }
    if let Some(device) = &app.active_output_device {
        ui.label(format!("Playing on: {}", device));
    }
    if let Some(snap) = &app.engine_snapshot {
        if snap.underruns > 0 {
            ui.label(format!("Buffer underruns: {}", snap.underruns));
        }
    }
    ui.add_space(8.0);

    // ---- Output device ----
    ui.horizontal(|ui| {
        ui.label(translate("filter", app.language));
        ui.text_edit_singleline(&mut app.device_filter);
    });

    let filter = app.device_filter.to_lowercase();
    let default_label = translate("default_output_device_label", app.language);
    let current_label = app
        .preferred_output_device
        .clone()
        .unwrap_or_else(|| default_label.to_string());
    let mut choice: Option<Option<String>> = None;
    let combo = egui::ComboBox::from_id_salt("device_combo")
        .selected_text(current_label)
        .show_ui(ui, |ui| {
            if (filter.is_empty() || default_label.to_lowercase().contains(&filter))
                && ui
                    .selectable_label(app.preferred_output_device.is_none(), default_label)
                    .clicked()
            {
                choice = Some(None);
            }
            let mut any = false;
            for name in &app.device_names {
                if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                    continue;
                }
                any = true;
                let selected = app.preferred_output_device.as_deref() == Some(name.as_str());
                if ui.selectable_label(selected, name).clicked() {
                    choice = Some(Some(name.clone()));
                }
            }
            if !any && !app.device_names.is_empty() && !filter.is_empty() {
                ui.label(translate("no_matching_devices", app.language));
            }
        });
    // Enumerate devices only when the list is opened (not every frame).
    if combo.response.clicked() {
        app.refresh_output_devices();
    }
    if let Some(device) = choice {
        // Applied and saved immediately; the engine switches without restarting the track.
        app.set_output_device(device);
        app.update_app_vm();
    }

    // ---- Resampler (applies when the engine starts) ----
    ui.add_space(8.0);
    ui.label("Resampler (applies after restart)");
    let mut resampler = app.resampler;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut resampler, ResamplerSetting::Fft, "FFT");
        ui.selectable_value(&mut resampler, ResamplerSetting::Sinc, "Sinc");
    });
    if resampler != app.resampler {
        app.resampler = resampler;
        let mut cfg = Config::load();
        cfg.resampler = resampler;
        if let Err(err) = cfg.save() {
            tracing::warn!("saving config failed: {err:#}");
        }
    }
    if app.resampler == ResamplerSetting::Sinc {
        ui.label(translate("resampler_taps", app.language));
        ui.add(egui::DragValue::new(&mut app.taps).range(16..=512));
        if ui.button(translate("save_taps", app.language)).clicked() {
            let mut cfg = Config::load();
            match app.preferred_output_device.clone() {
                Some(name) => cfg.set_taps_for_device(name, app.taps),
                None => cfg.resampler_taps = app.taps,
            }
            if let Err(err) = cfg.save() {
                tracing::warn!("saving config failed: {err:#}");
            }
            app.update_app_vm();
        }
    }

    // ---- Language (applied and saved immediately) ----
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(translate("language", app.language));
        let mut selected_lang = app.language;
        egui::ComboBox::from_id_salt("language_combo")
            .selected_text(language_name(app.language))
            .show_ui(ui, |ui| {
                for lang in supported_languages() {
                    ui.selectable_value(&mut selected_lang, *lang, language_name(*lang));
                }
            });
        if selected_lang != app.language {
            app.language = selected_lang;
            let mut cfg = Config::load();
            cfg.language = selected_lang;
            if let Err(err) = cfg.save() {
                tracing::warn!("saving config failed: {err:#}");
            }
        }
    });
}
