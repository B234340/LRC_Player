//! Settings: appearance, playback, lyrics, keyboard shortcuts and about.

use eframe::egui::{self, vec2, Sense, Ui, Vec2};
use lrc_player_application::config::{Language, LogLevel, ResamplerSetting};

use crate::presentation::app::GuiApp;
use crate::presentation::ui::{
    i18n::Tr,
    shortcuts::SHORTCUT_LIST,
    theme,
    views::{settings_row, settings_section},
    widgets::{badge, button, section_header, segmented, toggle_switch, ButtonKind},
    Accent, Icon, ThemeMode, UiAction,
};

const THEMES: [ThemeMode; 3] = [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System];

fn data_dir() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.join("data"))
}

pub(super) fn show(app: &mut GuiApp, ui: &mut Ui, tr: Tr) -> Vec<UiAction> {
    let mut actions = Vec::new();
    section_header(ui, tr.t("nav.settings"), None, |_| {});
    ui.add_space(16.0);
    egui::ScrollArea::vertical().id_salt("settings_page").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(760.0);
        let pal = theme::palette(ui);
        let restart = tr.t("settings.restart_required");

        // ---------------- appearance ----------------
        settings_section(ui, tr.t("settings.appearance"), None, |ui| {
            settings_row(ui, true, tr.t("settings.theme"), None, |ui| {
                let labels = [tr.t("settings.theme_light"), tr.t("settings.theme_dark"), tr.t("settings.theme_system")];
                let current = THEMES.iter().position(|m| *m == app.ui.theme).unwrap_or(2);
                if let Some(i) = segmented(ui, egui::Id::new("settings_theme"), &labels, current) {
                    actions.push(UiAction::SetTheme(THEMES[i]));
                }
            });
            settings_row(ui, false, tr.t("settings.accent"), None, |ui| {
                for a in Accent::ALL.iter().rev() {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tr.t(accent_key(*a)));
                    let scale = if resp.hovered() { 1.08 } else { 1.0 };
                    ui.painter().circle_filled(r.center(), 10.0 * scale, a.color(pal.dark));
                    if *a == app.ui.accent {
                        ui.painter().circle_stroke(r.center(), 12.5, egui::Stroke::new(2.0, pal.text));
                    }
                    if resp.clicked() {
                        actions.push(UiAction::SetAccent(*a));
                    }
                }
            });
            settings_row(ui, false, tr.t("settings.language"), None, |ui| {
                let current = usize::from(app.config.language != Language::Ja);
                if let Some(i) = segmented(ui, egui::Id::new("settings_lang"), &["日本語", "English"], current) {
                    app.config.language = if i == 0 { Language::Ja } else { Language::En };
                    app.save_config();
                }
            });
            settings_row(ui, false, tr.t("settings.lyric_size"), Some(tr.t("settings.lyric_size_hint")), |ui| {
                let mut size = app.config.lyric_font_size;
                let resp = ui.add(egui::Slider::new(&mut size, 14.0..=56.0).step_by(1.0).fixed_decimals(0).suffix(" pt"));
                if resp.changed() {
                    app.config.lyric_font_size = size;
                }
                if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                    app.save_config();
                }
            });
        });

        // ---------------- playback ----------------
        settings_section(ui, tr.t("settings.playback"), Some(tr.t("settings.playback_hint")), |ui| {
            let active = app.active_output_device.clone();
            settings_row(ui, true, tr.t("settings.output_device"), active.as_deref(), |ui| {
                let default_label = tr.t("settings.device_default");
                let current = app.preferred_output_device.clone().unwrap_or_else(|| default_label.to_string());
                let mut choice: Option<Option<String>> = None;
                let combo = egui::ComboBox::from_id_salt("settings_device").selected_text(current).width(240.0).show_ui(ui, |ui| {
                    if ui.selectable_label(app.preferred_output_device.is_none(), default_label).clicked() {
                        choice = Some(None);
                    }
                    for name in &app.device_names {
                        let selected = app.preferred_output_device.as_deref() == Some(name.as_str());
                        if ui.selectable_label(selected, name).clicked() {
                            choice = Some(Some(name.clone()));
                        }
                    }
                });
                // Enumerate devices only when the list is opened (not every frame).
                if combo.response.clicked() {
                    app.refresh_output_devices();
                }
                if let Some(device) = choice {
                    app.set_output_device(device);
                }
            });
            settings_row(ui, false, tr.t("settings.resampler"), Some(restart), |ui| {
                let current = usize::from(app.config.resampler == ResamplerSetting::Sinc);
                let labels = [tr.t("settings.resampler_fft"), tr.t("settings.resampler_sinc")];
                if let Some(i) = segmented(ui, egui::Id::new("settings_resampler"), &labels, current) {
                    app.config.resampler = if i == 0 { ResamplerSetting::Fft } else { ResamplerSetting::Sinc };
                    app.save_config();
                    app.toast_info(restart.to_string());
                }
            });
            if app.config.resampler == ResamplerSetting::Sinc {
                settings_row(ui, false, tr.t("settings.resampler_taps"), Some(restart), |ui| {
                    let device = app.preferred_output_device.clone();
                    let mut taps = app.config.taps_for_device(device.as_ref());
                    if ui.add(egui::DragValue::new(&mut taps).range(16..=512)).changed() {
                        match device {
                            Some(name) => app.config.set_taps_for_device(name, taps),
                            None => app.config.resampler_taps = taps,
                        }
                        app.save_config();
                    }
                });
            }
            settings_row(ui, false, tr.t("settings.log_level"), Some(restart), |ui| {
                let levels = [LogLevel::Error, LogLevel::Warn, LogLevel::Info, LogLevel::Debug, LogLevel::Trace];
                let mut level = app.config.log_level;
                egui::ComboBox::from_id_salt("settings_log").selected_text(level.as_filter_str()).show_ui(ui, |ui| {
                    for l in levels {
                        ui.selectable_value(&mut level, l, l.as_filter_str());
                    }
                });
                if level != app.config.log_level {
                    app.config.log_level = level;
                    app.save_config();
                }
            });
        });

        // ---------------- lyrics ----------------
        settings_section(ui, tr.t("settings.lyrics"), None, |ui| {
            settings_row(ui, true, tr.t("settings.karaoke"), Some(tr.t("settings.karaoke_hint")), |ui| {
                if toggle_switch(ui, &mut app.config.karaoke).changed() {
                    app.save_config();
                }
            });
            settings_row(ui, false, tr.t("settings.spectrum"), Some(tr.t("settings.spectrum_hint")), |ui| {
                if toggle_switch(ui, &mut app.config.show_spectrum).changed() {
                    app.save_config();
                    app.save_state();
                }
            });
            settings_row(ui, false, tr.t("settings.offsets"), Some(tr.t("settings.offsets_hint")), |ui| {
                ui.label(theme::muted(ui, format!("{}", app.lyrics_offsets.len())));
            });
        });

        // ---------------- shortcuts ----------------
        settings_section(ui, tr.t("settings.shortcuts"), None, |ui| {
            for (i, (keys, desc)) in SHORTCUT_LIST.iter().enumerate() {
                settings_row(ui, i == 0, tr.t(desc), None, |ui| {
                    let keys = if cfg!(target_os = "macos") { keys.replace("Ctrl", "⌘") } else { keys.to_string() };
                    badge(ui, &keys, pal.hover, pal.text);
                });
            }
            settings_row(ui, false, tr.t("shortcut.table"), None, |ui| {
                badge(ui, "Enter / Delete / Ctrl + A", pal.hover, pal.text);
            });
            settings_row(ui, false, tr.t("shortcut.media_keys"), None, |ui| {
                badge(ui, tr.t("shortcut.media_keys_badge"), pal.hover, pal.text);
            });
        });

        // ---------------- about ----------------
        settings_section(ui, tr.t("settings.about"), None, |ui| {
            settings_row(ui, true, tr.t("app.name"), Some(tr.t("settings.about_hint")), |ui| {
                ui.label(theme::muted(ui, format!("v{}", env!("CARGO_PKG_VERSION"))));
            });
            if let Some(dir) = data_dir() {
                settings_row(ui, false, tr.t("settings.data_dir"), Some(&dir.to_string_lossy()), |ui| {
                    if button(ui, ButtonKind::Secondary, Some(Icon::Folder), tr.t("settings.open")).clicked() {
                        actions.push(UiAction::OpenContainingFolder(dir.join("config.json")));
                    }
                });
            }
        });
        ui.add_space(8.0);
        ui.allocate_space(vec2(1.0, 8.0));
    });
    actions
}

fn accent_key(a: Accent) -> &'static str {
    match a {
        Accent::Teal => "accent.teal",
        Accent::Blue => "accent.blue",
        Accent::Violet => "accent.violet",
        Accent::Rose => "accent.rose",
        Accent::Amber => "accent.amber",
        Accent::Green => "accent.green",
    }
}
