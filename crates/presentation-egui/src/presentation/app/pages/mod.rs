//! Page bodies for the central area. Each page reads `GuiApp` state, draws `ui` views and
//! returns [`UiAction`]s (page-local actions such as playlist reordering are applied here).

mod albums;
mod artists;
mod folders;
mod library;
mod now_playing;
mod playlists;
mod queue;
mod settings;

use std::hash::{Hash, Hasher};

use eframe::egui::{self, vec2, Rect, Sense, Ui, Vec2};

use crate::presentation::app::GuiApp;
use crate::presentation::ui::{
    format::{format_count, format_total},
    i18n::Tr,
    icons::{paint_icon, Icon},
    shell,
    theme,
    views::TrackRow,
    widgets::{button, ButtonKind},
    Page, UiAction,
};

/// Cached rows of a detail page (album / artist / playlist).
#[derive(Default)]
pub(crate) struct DetailRows {
    key: u64,
    pub(crate) rows: Vec<TrackRow>,
    /// Bumped when `rows` is rebuilt (track-table cache key).
    pub(crate) revision: u64,
}

impl DetailRows {
    /// Rebuilds `rows` with `build` when `key` changed.
    pub(crate) fn ensure(&mut self, key: u64, build: impl FnOnce() -> Vec<TrackRow>) {
        if self.key != key || self.revision == 0 {
            self.rows = build();
            self.key = key;
            self.revision += 1;
        }
    }
}

pub(crate) fn hash_key(parts: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

/// "N 曲 · 1:02:03" for a set of rows.
pub(crate) fn tracks_summary(rows: &[TrackRow], tr: &Tr) -> String {
    let total: u64 = rows.iter().filter_map(|r| r.duration_ms).sum();
    if total > 0 {
        format!("{} {} · {}", format_count(rows.len()), tr.t("table.tracks_count"), format_total(total))
    } else {
        format!("{} {}", format_count(rows.len()), tr.t("table.tracks_count"))
    }
}

/// "← Albums" link used on detail pages.
pub(crate) fn back_link(ui: &mut Ui, label: &str) -> bool {
    button(ui, ButtonKind::Ghost, Some(Icon::ChevronLeft), label).clicked()
}

/// Back link of a detail page: returns to the previous location (e.g. the library table
/// with its search and scroll position) when there is one, else to `parent`.
pub(crate) fn back_nav(app: &GuiApp, ui: &mut Ui, tr: &Tr, parent: Page) -> Option<UiAction> {
    match app.back_label(tr) {
        Some(label) => back_link(ui, label).then_some(UiAction::Back),
        None => back_link(ui, tr.t(shell::sidebar::page_label_key(parent))).then_some(UiAction::Navigate(parent)),
    }
}

/// Round avatar with the first letter (artists) or an icon.
pub(crate) fn avatar(ui: &mut Ui, size: f32, name: &str, icon: Option<Icon>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let pal = theme::palette(ui);
    let hue = crate::presentation::ui::widgets::artwork::seed_hue(name);
    let fill: egui::Color32 = egui::ecolor::Hsva::new(hue, 0.35, if pal.dark { 0.38 } else { 0.78 }, 1.0).into();
    ui.painter().circle_filled(rect.center(), size / 2.0, fill);
    match icon {
        Some(icon) => paint_icon(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(size * 0.45)), icon, egui::Color32::WHITE),
        None => {
            let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            let g = ui.painter().layout_no_wrap(initial, theme::bold_font(ui.ctx(), size * 0.42), egui::Color32::WHITE);
            ui.painter().galley(rect.center() - g.size() / 2.0, g, egui::Color32::WHITE);
        }
    }
    resp
}

/// Centered spinner + text while the library loads.
pub(crate) fn loading(ui: &mut Ui, text: &str) {
    let pal = theme::palette(ui);
    ui.add_space((ui.available_height() * 0.35).max(24.0));
    ui.vertical_centered(|ui| {
        ui.add(egui::Spinner::new().size(28.0).color(pal.accent_fg));
        ui.add_space(10.0);
        ui.label(theme::muted(ui, text));
    });
}

impl GuiApp {
    pub(crate) fn page_ui(&mut self, ui: &mut Ui, tr: Tr) -> Vec<UiAction> {
        if self.ui.page == Page::NowPlaying {
            return now_playing::show(self, ui, tr);
        }
        let pal = theme::palette(ui);
        let mut actions = Vec::new();
        shell::page_frame(&pal).show(ui, |ui| {
            ui.set_min_size(ui.available_size() - vec2(0.0, 0.0));
            actions = match self.ui.page {
                Page::Library => library::show(self, ui, tr),
                Page::Albums => albums::show(self, ui, tr),
                Page::Artists => artists::show(self, ui, tr),
                Page::Playlists => playlists::show(self, ui, tr),
                Page::Folders => folders::show(self, ui, tr),
                Page::Queue => queue::show(self, ui, tr),
                Page::Settings => settings::show(self, ui, tr),
                Page::NowPlaying => Vec::new(),
            };
        });
        actions
    }
}
