//! Queue page: what is playing now and what plays next. Drag to reorder, remove, clear,
//! save as a playlist, and drop tracks from any table to insert them.

use eframe::egui::{self, vec2, Id, Ui};

use super::tracks_summary;
use crate::presentation::app::GuiApp;
use crate::presentation::ui::{
    format::format_count,
    i18n::Tr,
    icons::{icon_button, Icon},
    theme,
    views::{queue_empty, queue_rows, QueueProps},
    widgets::{artwork, button, card_frame, section_header, ButtonKind, Tex},
    Page, UiAction,
};

pub(super) fn show(app: &mut GuiApp, ui: &mut Ui, tr: Tr) -> Vec<UiAction> {
    let mut actions = Vec::new();
    app.refresh_queue_rows();
    let sub = tracks_summary(&app.queue_rows.rows, &tr);
    let has_queue = !app.queue.is_empty();
    // Narrow central area (minimum window size, right panel open): icon-only buttons.
    let compact = ui.available_width() < 560.0;
    section_header(ui, tr.t("queue.title"), Some(&sub), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add_enabled_ui(has_queue, |ui| {
            let save = if compact {
                icon_button(ui, Icon::Playlist, 17.0, false, tr.t("queue.save_as_playlist"))
            } else {
                button(ui, ButtonKind::Secondary, Some(Icon::Playlist), tr.t("queue.save_as_playlist"))
            };
            if save.clicked() {
                actions.push(UiAction::QueueSaveAsPlaylist);
            }
            let clear = if compact {
                icon_button(ui, Icon::Trash, 17.0, false, tr.t("queue.clear"))
            } else {
                button(ui, ButtonKind::Ghost, Some(Icon::Trash), tr.t("queue.clear"))
            };
            if clear.clicked() {
                actions.push(UiAction::QueueClear);
            }
        });
    });
    ui.add_space(16.0);
    let id = Id::new("queue_page");
    let has_track = !app.play_path.is_empty();
    if !has_queue && !has_track {
        actions.extend(queue_empty(ui, id, tr));
        return actions;
    }

    // ---- now playing ----
    if has_track {
        let pal = theme::palette(ui);
        let title = app.current_title();
        let artist = app.play_artist.clone().unwrap_or_default();
        let album = app.play_album.clone().unwrap_or_default();
        let playing = app.is_playing();
        let art = app.play_artwork_handle.as_ref().map(Tex::from);
        card_frame(&pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                artwork(ui, art, 72.0, 8, if album.is_empty() { &title } else { &album });
                ui.add_space(8.0);
                // The text truncates to what the buttons leave free.
                let buttons_w = if compact { 84.0 } else { 260.0 };
                let text_w = (ui.available_width() - buttons_w).max(60.0);
                ui.allocate_ui_with_layout(vec2(text_w, 72.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.add_space(4.0);
                    ui.label(theme::caption(ui, tr.t("queue.now_playing")).color(pal.accent_fg));
                    ui.add(egui::Label::new(theme::title(ui, &title).color(pal.text)).truncate());
                    let line = match (artist.is_empty(), album.is_empty()) {
                        (false, false) => format!("{artist} · {album}"),
                        (false, true) => artist.clone(),
                        (true, false) => album.clone(),
                        (true, true) => String::new(),
                    };
                    ui.add(egui::Label::new(theme::muted(ui, line)).truncate());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (icon, label) = if playing { (Icon::Pause, tr.t("queue.pause")) } else { (Icon::Play, tr.t("queue.resume")) };
                    if compact {
                        if icon_button(ui, Icon::NowPlaying, 17.0, false, tr.t("nav.now_playing")).clicked() {
                            actions.push(UiAction::Navigate(Page::NowPlaying));
                        }
                        if icon_button(ui, icon, 17.0, false, label).clicked() {
                            actions.push(UiAction::PlayPause);
                        }
                    } else {
                        if button(ui, ButtonKind::Ghost, Some(Icon::NowPlaying), tr.t("nav.now_playing")).clicked() {
                            actions.push(UiAction::Navigate(Page::NowPlaying));
                        }
                        if button(ui, ButtonKind::Primary, Some(icon), label).clicked() {
                            actions.push(UiAction::PlayPause);
                        }
                    }
                });
            });
        });
        ui.add_space(20.0);
    }

    // ---- up next ----
    let first = app.current_queue_index.map(|c| c + 1).unwrap_or(0).min(app.queue.len());
    let upcoming = app.queue.len() - first;
    let pal = theme::palette(ui);
    ui.horizontal(|ui| {
        ui.label(theme::title(ui, tr.t("queue.up_next")).color(pal.text));
        ui.label(theme::caption(ui, format!("{} {}", format_count(upcoming), tr.t("table.tracks_count"))));
    });
    ui.add_space(4.0);
    if upcoming == 0 {
        ui.label(theme::muted(ui, tr.t("queue.nothing_next")));
        ui.add_space(4.0);
    }
    ui.add_space(2.0);
    let arts = app.queue_arts();
    let items = app.queue_items(arts);
    let playing = app.is_playing();
    let props = QueueProps { items: &items, current: app.current_queue_index, playing, tr };
    ui.allocate_ui(vec2(ui.available_width(), ui.available_height()), |ui| {
        actions.extend(queue_rows(ui, id, &props, first));
    });
    actions
}
