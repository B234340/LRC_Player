//! Folders: library and lyrics folders (add via the native dialog, remove), rescan / cancel,
//! scan progress and the last result, exclude patterns, and clearing the library.

use eframe::egui::{self, vec2, Align, Layout, TextStyle, Ui};

use crate::presentation::app::{
    scan::{scan_phase, ScanPhase},
    GuiApp, Modal,
};
use crate::presentation::ui::{
    format::format_count,
    i18n::Tr,
    icons::{icon_button, paint_icon, Icon},
    theme,
    views::{
        settings::{middle_ellipsis, page_column, SECTION_MAX_WIDTH},
        settings_row, settings_section,
    },
    widgets::{button, empty_state, ButtonKind},
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
        // Room for the text: everything but the remove button (~28 px) and two gaps.
        let w = (ui.available_width() - 28.0 - ui.spacing().item_spacing.x * 2.0).max(0.0);
        let font = TextStyle::Body.resolve(ui.style());
        // Middle ellipsis keeps both the root and the most specific folder visible.
        let (shown, cut) = middle_ellipsis(ui, path, &font, w);
        ui.allocate_ui_with_layout(vec2(w, 20.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_max_width(w);
            let label = ui.add(egui::Label::new(egui::RichText::new(shown).color(pal.text)).truncate());
            if cut {
                label.on_hover_text(path);
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| icon_button(ui, Icon::Close, 14.0, false, tip).clicked()).inner
    })
    .inner
}

/// Page title and subtitle with the action buttons on the right while they fit, otherwise
/// on their own (wrapping) line below. `buttons` gets `true` when laid out left to right.
fn page_header(ui: &mut Ui, title: &str, subtitle: &str, buttons: impl FnOnce(&mut Ui, bool)) {
    let pal = theme::palette(ui);
    let id = ui.make_persistent_id("folders_header_buttons");
    let prev_w: Option<f32> = ui.data(|d| d.get_temp(id));
    let mut buttons = Some(buttons);
    let mut measured = None;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(egui::Label::new(theme::heading(ui, title).color(pal.text)).wrap());
            ui.add(egui::Label::new(theme::caption(ui, subtitle)).wrap());
        });
        let remaining = ui.available_width();
        if prev_w.is_some_and(|w| w + 16.0 <= remaining) {
            if let Some(b) = buttons.take() {
                let r = ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    b(ui, false);
                });
                measured = Some(r.response.rect.width());
            }
        }
    });
    if let Some(b) = buttons {
        ui.add_space(10.0);
        let r = ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            b(ui, true);
        });
        measured = Some(r.response.rect.width());
    }
    if let Some(w) = measured {
        if prev_w.is_none_or(|p| (p - w).abs() > 0.5) {
            ui.data_mut(|d| d.insert_temp(id, w));
            ui.ctx().request_discard("folders header layout");
        }
    }
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
    let picking = app.ui.folder_picker.is_some();
    let mut cancel_scan = false;
    page_header(ui, tr.t("nav.folders"), &sub, |ui, ltr| {
        let mut scan_button = |ui: &mut Ui| {
            if scanning {
                cancel_scan |= button(ui, ButtonKind::Secondary, Some(Icon::Close), tr.t("folders.cancel_scan")).clicked();
            } else if button(ui, ButtonKind::Secondary, Some(Icon::Refresh), tr.t("folders.rescan")).clicked() {
                actions.push(UiAction::Rescan);
            }
        };
        let mut add = false;
        let mut add_button = |ui: &mut Ui| {
            ui.add_enabled_ui(!picking, |ui| {
                add = button(ui, ButtonKind::Primary, Some(Icon::Plus), tr.t("library.add_folder")).clicked();
            });
        };
        // Visual order is always [Add folder] [Rescan].
        if ltr {
            add_button(ui);
            scan_button(ui);
        } else {
            scan_button(ui);
            add_button(ui);
        }
        if add {
            actions.push(UiAction::AddFolder);
        }
    });
    if cancel_scan {
        app.request_scan_cancel();
    }
    ui.add_space(16.0);

    egui::ScrollArea::vertical().id_salt("folders_page").auto_shrink([false, false]).show(ui, |ui| {
        page_column(ui, SECTION_MAX_WIDTH, |ui| {
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
                // Failure messages can be long: wrap within the card.
                ui.add(egui::Label::new(egui::RichText::new(status_text(app, &tr)).color(pal.text)).wrap());
            });
            // Breathing room above the next row's hairline when the status wraps.
            ui.add_space(4.0);
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
                    ui.add(egui::Label::new(theme::muted(
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
                    ))
                    .wrap());
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
                    // Same height as the 32 px "add" button next to it (egui sizes a
                    // single-line edit from its row height plus the vertical margin).
                    let row_h = ui.fonts_mut(|f| f.row_height(&TextStyle::Body.resolve(ui.style())));
                    let margin_y = ((32.0 - row_h) / 2.0).round().clamp(2.0, 12.0) as i8;
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut app.ui.exclude_input)
                            .hint_text(tr.t("folders.exclude_placeholder"))
                            .desired_width(ui.available_width())
                            .margin(egui::Margin::symmetric(8, margin_y))
                            .vertical_align(Align::Center),
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
    });
    actions
}
