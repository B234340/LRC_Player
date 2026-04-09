use super::types::AppTab;
use super::LrcPlayerApp;
use crate::core::player::PlaybackStatus;
use eframe::egui;
use std::time::Duration;

impl eframe::App for LrcPlayerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.state.refresh();
        self.apply_theme(ctx);

        ctx.style_mut(|style| {
            style.spacing.interact_size = egui::vec2(84.0, 42.0);
            style.spacing.button_padding = egui::vec2(12.0, 10.0);
            style.spacing.scroll = egui::style::ScrollStyle {
                bar_width: 24.0,
                bar_inner_margin: 6.0,
                bar_outer_margin: 4.0,
                ..egui::style::ScrollStyle::solid()
            };
            style.text_styles.insert(
                egui::TextStyle::Button,
                egui::FontId::new(22.0, egui::FontFamily::Proportional),
            );
        });

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("LRC Player");
                ui.separator();
                ui.label("Portable queue-driven music player");
            });

            ui.add_space(8.0);
            self.render_tab_bar(ui);
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(&self.state.status_line);
                ui.separator();
                ui.label(format!("Theme: {}", self.state.config.theme));
                ui.separator();
                ui.label(format!("Library: {} tracks", self.state.library.track_count()));
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.active_tab {
            AppTab::Queue | AppTab::Album | AppTab::Artist => {
                ui.set_min_width(ui.available_width());
                self.render_active_tab(ui, ctx);
            }
            _ => {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        self.render_active_tab(ui, ctx);
                    });
            }
        });

        self.render_track_detail_window(ctx);

        if self.state.playback_snapshot().status == PlaybackStatus::Playing {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

impl LrcPlayerApp {
    fn apply_theme(&self, ctx: &egui::Context) {
        match self.state.config.theme.as_str() {
            "light" => ctx.set_visuals(egui::Visuals::light()),
            "dark" => ctx.set_visuals(egui::Visuals::dark()),
            _ => {}
        }
    }

    fn render_tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            self.render_tab_button(ui, AppTab::Queue, "Queue");
            self.render_tab_button(ui, AppTab::Play, "Play");
            self.render_tab_button(ui, AppTab::Album, "Album");
            self.render_tab_button(ui, AppTab::Artist, "Artist");
            self.render_tab_button(ui, AppTab::Playlist, "Playlist");
            self.render_tab_button(ui, AppTab::Library, "Library");
            self.render_tab_button(ui, AppTab::Scan, "Scan");
            self.render_tab_button(ui, AppTab::Settings, "Settings");
        });
    }

    fn render_tab_button(&mut self, ui: &mut egui::Ui, tab: AppTab, label: &str) {
        let selected = self.active_tab == tab;
        let button = egui::Button::new(
            egui::RichText::new(label)
                .size(26.0)
                .family(egui::FontFamily::Proportional),
        )
        .selected(selected)
        .min_size(egui::vec2(140.0, 56.0));

        if ui.add(button).clicked() {
            self.active_tab = tab;
            if tab == AppTab::Queue {
                self.revealed_queue_anchor_id = None;
                self.queue_follow_paused_by_user = false;
            }
        }
    }

    fn render_active_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        match self.active_tab {
            AppTab::Queue => self.render_queue_tab(ui, ctx),
            AppTab::Play => self.render_play_tab(ui, ctx),
            AppTab::Album => self.render_album_tab(ui, ctx),
            AppTab::Artist => self.render_artist_tab(ui, ctx),
            AppTab::Playlist => self.render_playlist_tab(ui, ctx),
            AppTab::Library => self.render_library_tab(ui),
            AppTab::Scan => self.render_scan_tab(ui),
            AppTab::Settings => self.render_settings_tab(ui),
        }
    }
}