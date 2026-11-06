//! Folders: library and lyrics folders (add via the native dialog, remove), rescan / cancel,
//! scan progress and the last result, exclude patterns, and clearing the library.

use eframe::egui::{self, vec2, Ui};

use crate::presentation::app::{
    scan::{scan_phase, ScanPhase},
    GuiApp, Modal,
};
use crate::presentation::ui::{
    format::format_count,
    i18n::Tr,
    icons::{icon_button, paint_icon, Icon},
    theme,
    views::{settings_row, settings_section},
    widgets::{button, empty_state, section_header, ButtonKind},
    UiAction,
};

fn folder_row(ui: &mut Ui, first: bool, path: &str, tip: &str) -> bool {
    let pal = theme::palette(ui);
    if !first {
        let r = ui.available_rect_before_wrap();
        ui.painter().hline(r.x_range(), r.top(), egui::Stroke::new(1.0, pal.border));
    }
    ui.horizontal(|ui| {
        ui.set_min_height(44.0);
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
        paint_icon(ui.painter(), r, Icon::Folder, pal.text_muted);
        ui.add_space(6.0);
        let w = ui.available_width() - 44.0;
        ui.allocate_ui(vec2(w, 20.0), |ui| {
            ui.add(egui::Label::new(egui::RichText::new(path).color(pal.text)).truncate());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| icon_button(ui, Icon::Close, 14.0, false, tip).clicked())
            .inner
    })
    .inner
}

fn status_text(app: &GuiApp, tr: &Tr) -> String {
    let Some(status) = app.last_scan_status.as_deref() else {
        return tr.t("folders.status_none").to_string();
    };
    match scan_phase(status) {
        ScanPhase::Requested => tr.t("folders.status_requested").to_string(),
        ScanPhase::Started | ScanPhase::Progress => tr.t("folders.status_running").to_string(),
        ScanPhase::Completed => tr.t("folders.status_completed").to_string(),
        ScanPhase::Cancelled => tr.t("folders.status_cancelled").to_string(),
        ScanPhase::Failed => format!("{}: {status}", tr.t("folders.status_failed")),
        ScanPhase::Other => status.to_string(),
    }
}

pub(super) fn show(app: &mut GuiApp, ui: &mut Ui, tr: Tr) -> Vec<UiAction> {
    let mut actions = Vec::new();
    let ctx = ui.ctx().clone();
    let sub = format!(
        "{} {} · {} {}",
        format_count(app.library.rows.len()),
        tr.t("table.tracks_count"),
        format_count(app.library.albums.len()),
        tr.t("albums.count")
    );
    let scanning = app.scanning;
    section_header(ui, tr.t("nav.folders"), Some(&sub), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if scanning {
            if button(ui, ButtonKind::Secondary, Some(Icon::Close), tr.t("folders.cancel_scan")).clicked() {
                app.request_scan_cancel();
            }
        } else if button(ui, ButtonKind::Secondary, Some(Icon::Refresh), tr.t("folders.rescan")).clicked() {
            actions.push(UiAction::Rescan);
        }
        let picking = app.ui.folder_picker.is_some();
        ui.add_enabled_ui(!picking, |ui| {
            if button(ui, ButtonKind::Primary, Some(Icon::Plus), tr.t("library.add_folder")).clicked() {
                actions.push(UiAction::AddFolder);
            }
        });
    });
    ui.add_space(16.0);

    egui::ScrollArea::vertical().id_salt("folders_page").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(820.0);
        let pal = theme::palette(ui);

        // ---- scan status ----
        settings_section(ui, tr.t("folders.scan_section"), None, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(48.0);
                if app.scanning {
                    ui.add(egui::Spinner::new().size(16.0).color(pal.accent_fg));
                } else {
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
                    let ok = !matches!(app.last_scan_status.as_deref().map(scan_phase), Some(ScanPhase::Failed));
                    paint_icon(ui.painter(), r, if ok { Icon::Success } else { Icon::Error }, if ok { pal.success } else { pal.danger });
                }
                ui.add_space(6.0);
                ui.label(egui::RichText::new(status_text(app, &tr)).color(pal.text));
            });
            if app.scanning {
                let r = ui.available_rect_before_wrap();
                let bar = egui::Rect::from_min_size(r.min, vec2(r.width(), 4.0));
                ui.painter().rect_filled(bar, 2.0, pal.hover);
                // Indeterminate: a sliding segment.
                let t = (ui.input(|i| i.time) * 0.6).fract() as f32;
                let seg_w = bar.width() * 0.25;
                let x0 = bar.left() + (bar.width() + seg_w) * t - seg_w;
                let seg = egui::Rect::from_min_max(egui::pos2(x0.max(bar.left()), bar.top()), egui::pos2((x0 + seg_w).min(bar.right()), bar.bottom()));
                ui.painter().rect_filled(seg, 2.0, pal.accent);
                ui.allocate_space(vec2(r.width(), 10.0));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
            }
            let (scanned, parsed, registered) = app.scan_counts;
            if scanned.is_some() || parsed.is_some() || registered.is_some() {
                let fmt = |v: Option<u64>| v.map(|v| format_count(v as usize)).unwrap_or_else(|| "—".into());
                settings_row(ui, false, tr.t("folders.counts"), None, |ui| {
                    ui.label(theme::muted(
                        ui,
                        format!(
                            "{} {} · {} {} · {} {}",
                            tr.t("folders.scanned"),
                            fmt(scanned),
                            tr.t("folders.parsed"),
                            fmt(parsed),
                            tr.t("folders.registered"),
                            fmt(registered)
                        ),
                    ));
                });
            }
        });

        // ---- music folders ----
        settings_section(ui, tr.t("folders.music_section"), Some(tr.t("folders.music_hint")), |ui| {
            if app.scan_targets.is_empty() {
                ui.add_space(8.0);
                if empty_state(ui, Icon::Folder, tr.t("folders.none"), tr.t("library.empty_hint"), Some((Icon::Plus, tr.t("library.add_folder")))) {
                    actions.push(UiAction::AddFolder);
                }
            }
            let mut remove = None;
            for (i, p) in app.scan_targets.iter().enumerate() {
                if folder_row(ui, i == 0, p, tr.t("folders.remove")) {
                    remove = Some(p.clone());
                }
            }
            if let Some(p) = remove {
                actions.push(UiAction::RemoveFolder(p.into()));
            }
        });

        // ---- lyrics folders ----
        settings_section(ui, tr.t("folders.lyrics_section"), Some(tr.t("folders.lyrics_hint")), |ui| {
            let mut remove = None;
            for (i, p) in app.scan_lrc_targets.iter().enumerate() {
                if folder_row(ui, i == 0, p, tr.t("folders.remove")) {
                    remove = Some(p.clone());
                }
            }
            if let Some(p) = remove {
                actions.push(UiAction::RemoveFolder(p.into()));
            }
            ui.add_space(6.0);
            ui.add_enabled_ui(app.ui.folder_picker.is_none(), |ui| {
                if button(ui, ButtonKind::Ghost, Some(Icon::Plus), tr.t("folders.add_lyrics")).clicked() {
                    app.pick_folder(&ctx, true);
                }
            });
            ui.add_space(6.0);
        });

        // ---- excludes ----
        settings_section(ui, tr.t("folders.exclude_section"), Some(tr.t("folders.exclude_hint")), |ui| {
            let mut remove = None;
            for (i, p) in app.scan_excludes.iter().enumerate() {
                if folder_row(ui, i == 0, p, tr.t("folders.remove")) {
                    remove = Some(i);
                }
            }
            if let Some(i) = remove {
                app.scan_excludes.remove(i);
                app.save_state();
            }
            ui.horizontal(|ui| {
                ui.set_min_height(44.0);
                let mut add = false;
                let mut enter = false;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    add = button(ui, ButtonKind::Secondary, Some(Icon::Plus), tr.t("folders.exclude_add")).clicked();
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut app.ui.exclude_input)
                            .hint_text(tr.t("folders.exclude_placeholder"))
                            .desired_width(ui.available_width()),
                    );
                    enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                });
                let value = app.ui.exclude_input.trim().to_string();
                if (enter || add) && !value.is_empty() && !app.scan_excludes.contains(&value) {
                    app.scan_excludes.push(value);
                    app.ui.exclude_input.clear();
                    app.save_state();
                }
            });
        });

        // ---- danger zone ----
        settings_section(ui, tr.t("folders.maintenance"), None, |ui| {
            settings_row(ui, true, tr.t("folders.clear_library"), Some(tr.t("folders.clear_library_hint")), |ui| {
                if button(ui, ButtonKind::Danger, Some(Icon::Trash), tr.t("folders.clear")).clicked() {
                    app.ui.modal = Some(Modal::ClearLibrary);
                }
            });
        });
    });
    actions
}
