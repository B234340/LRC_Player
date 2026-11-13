//! Window composition: transport bar (bottom), sidebar (left), optional right panel
//! (queue / lyrics), the selected page, the EQ popover, dialogs, toasts and the file-drop
//! overlay. Everything here only reads app state and collects [`UiAction`]s, which are
//! applied at the end of the frame.

use std::path::Path;

use eframe::egui::{self, vec2, Align2, CentralPanel, Id, Panel};
use lrc_player_application::dto::{volume_to_linear, RepeatMode as DtoRepeat};
use lrc_player_domain::model::PlaybackState;

use crate::presentation::app::{library_index::file_title, lyrics::lyric_lines, playlists, GuiApp, Modal};
use crate::presentation::ui::{
    format::format_duration_opt,
    i18n::Tr,
    shell::{self, sidebar, transport_bar, PlaylistEntry, SidebarProps, TransportProps},
    shortcuts,
    theme::{self, metrics},
    views::{
        eq_panel, lyrics_view, queue_panel, EqPanelProps, LyricsProps, QueueItem, QueueProps, TrackRow,
    },
    format::format_count,
    widgets::{button, dialog, dialog_error, dialog_text, dialog_text_field, empty_state, ButtonKind, DialogButton, DialogResult, DialogSpec, Tex},
    Icon, Page, RepeatMode, RightPanel, UiAction,
};

/// Queue rows resolved for display (rebuilt only when the queue or the library changes).
#[derive(Default)]
pub(crate) struct QueueRowsCache {
    paths: Vec<String>,
    library_revision: u64,
    pub(crate) rows: Vec<TrackRow>,
}

/// Sidebar / context-menu playlist entries (names localized).
pub(crate) fn playlist_entries<'a>(lists: &'a [lrc_player_application::dto::PlaylistDto], tr: &Tr) -> Vec<PlaylistEntry<'a>> {
    lists
        .iter()
        .map(|p| PlaylistEntry { id: &p.id, name: playlists::display_name(p, tr), track_count: p.tracks.len() })
        .collect()
}

pub(crate) fn ui_repeat(r: DtoRepeat) -> RepeatMode {
    match r {
        DtoRepeat::Off => RepeatMode::Off,
        DtoRepeat::All => RepeatMode::All,
        DtoRepeat::One => RepeatMode::One,
    }
}

impl GuiApp {
    pub(crate) fn is_playing(&self) -> bool {
        matches!(self.playback_state, PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking)
    }

    /// Title for the current track (tags, library, or the file name).
    pub(crate) fn current_title(&self) -> String {
        self.play_title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| file_title(&self.play_path))
    }

    pub(crate) fn refresh_queue_rows(&mut self) {
        let cache = &self.queue_rows;
        if cache.paths == self.queue && cache.library_revision == self.library.revision {
            return;
        }
        self.library.ensure_probed(self.queue.iter());
        let rows = self.queue.iter().map(|p| self.library.display_row(p)).collect();
        self.queue_rows = QueueRowsCache { paths: self.queue.clone(), library_revision: self.library.revision, rows };
    }

    pub(crate) fn draw_shell(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let tr = self.tr();
        let pal = theme::palette(ui);
        let mut actions: Vec<UiAction> = Vec::new();

        // ---- keyboard ----
        let dialog_open = self.ui.modal.is_some() || self.ui.details.is_some();
        if !dialog_open {
            actions.extend(shortcuts::global_shortcuts(&ctx, volume_to_linear(self.volume)));
            let esc = !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Escape));
            if esc && self.ui.eq_open {
                self.ui.eq_open = false;
            }
        }

        // ---- transport bar ----
        let title = self.current_title();
        let artist = self.play_artist.clone().unwrap_or_default();
        let seed = self.play_album.clone().unwrap_or_else(|| title.clone());
        Panel::bottom("transport")
            .exact_size(shell::transport::HEIGHT)
            .frame(shell::chrome_frame(&pal))
            .show_separator_line(false)
            .show(ui, |ui| {
                actions.extend(transport_bar(
                    ui,
                    &TransportProps {
                        has_track: !self.play_path.is_empty(),
                        title: &title,
                        artist: &artist,
                        artwork: self.play_artwork_handle.as_ref().map(Tex::from),
                        artwork_seed: &seed,
                        playing: self.is_playing(),
                        position_ms: self.play_position_ms,
                        duration_ms: self.play_duration_ms.unwrap_or(0),
                        shuffle: self.shuffle(),
                        repeat: ui_repeat(self.repeat()),
                        volume: volume_to_linear(self.volume),
                        muted: self.muted,
                        right_panel: self.ui.right_panel,
                        eq_open: self.ui.eq_open,
                        eq_enabled: self.eq.enabled,
                        queue_len: self.queue.len(),
                        no_output: self.no_output_device,
                        favorite: (!self.play_path.is_empty()).then(|| self.is_favorite_path(&self.play_path)),
                        tr,
                    },
                ));
            });

        // ---- sidebar ----
        {
            let entries = playlist_entries(&self.playlists, &tr);
            let active_playlist = (self.ui.page == Page::Playlists).then_some(self.ui.playlist_detail.as_deref()).flatten();
            let scanning = self.scanning.then_some(f32::NAN);
            let mut sidebar_state = self.ui.sidebar.clone();
            let page = self.ui.page;
            Panel::left("sidebar")
                .exact_size(sidebar_state.width())
                .frame(shell::chrome_frame(&pal))
                .show_separator_line(false)
                .resizable(false)
                .show(ui, |ui| {
                    if let Some(a) = sidebar(
                        ui,
                        &SidebarProps { active: page, active_playlist, playlists: &entries, scanning, tr },
                        &mut sidebar_state,
                    ) {
                        actions.push(a);
                    }
                });
            drop(entries);
            self.ui.sidebar = sidebar_state;
        }

        // ---- right panel ----
        if let Some(panel) = self.ui.right_panel {
            if panel == RightPanel::Queue {
                self.refresh_queue_rows();
            }
            Panel::right("right_panel")
                .exact_size(metrics::RIGHT_PANEL_WIDTH)
                .frame(shell::right_panel_frame(&pal))
                .show_separator_line(false)
                .resizable(false)
                .show(ui, |ui| match panel {
                    RightPanel::Queue => actions.extend(self.queue_panel_ui(ui, tr)),
                    RightPanel::Lyrics => actions.extend(self.side_lyrics_ui(ui, tr)),
                });
        }

        // ---- page ----
        let central = if self.ui.page == Page::NowPlaying {
            egui::Frame::new().fill(pal.bg).inner_margin(egui::Margin { left: 0, right: 8, top: 8, bottom: 8 })
        } else {
            shell::central_frame(&pal)
        };
        CentralPanel::default().frame(central).show(ui, |ui| {
            actions.extend(self.page_ui(ui, tr));
        });

        // ---- overlays ----
        if self.ui.eq_open {
            actions.extend(self.eq_popover(&ctx, tr));
        }
        actions.extend(self.dialogs_ui(&ctx, tr));
        actions.extend(self.ui.toasts.show(&ctx, shell::transport::HEIGHT + 8.0, &tr));
        if let Some(a) = shell::file_drop(&ctx, &tr) {
            actions.push(a);
        }

        self.ui.focus_search = false;
        self.dispatch_all(&ctx, actions);
    }

    fn queue_panel_ui(&mut self, ui: &mut egui::Ui, tr: Tr) -> Vec<UiAction> {
        let arts = self.queue_arts();
        let items: Vec<QueueItem> = self.queue_items(arts);
        queue_panel(ui, &QueueProps { items: &items, current: self.current_queue_index, playing: self.is_playing(), tr })
    }

    /// Queue rows as panel / page items (call [`Self::refresh_queue_rows`] first).
    pub(crate) fn queue_items(&self, arts: Vec<Option<Tex>>) -> Vec<QueueItem<'_>> {
        self.queue_rows
            .rows
            .iter()
            .zip(arts)
            .map(|(r, artwork)| QueueItem {
                title: &r.title,
                artist: &r.artist,
                duration_ms: r.duration_ms,
                artwork,
                seed: if r.album.is_empty() { &r.title } else { &r.album },
            })
            .collect()
    }

    /// Thumbnails for the queue rows, only around the current entry (the loader keeps a
    /// bounded backlog).
    pub(crate) fn queue_arts(&mut self) -> Vec<Option<Tex>> {
        let cur = self.current_queue_index.unwrap_or(0);
        let lo = cur.saturating_sub(10);
        let hi = (cur + 60).min(self.queue_rows.rows.len());
        (0..self.queue_rows.rows.len())
            .map(|i| {
                if (lo..hi).contains(&i) || i < 30 {
                    let path = self.queue_rows.rows[i].path.to_string_lossy();
                    self.artwork.track_thumbnail(&path, super::ArtworkMode::FileFirst).as_ref().map(Tex::from)
                } else {
                    None
                }
            })
            .collect()
    }

    fn side_lyrics_ui(&mut self, ui: &mut egui::Ui, tr: Tr) -> Vec<UiAction> {
        let pal = theme::palette(ui);
        ui.label(theme::title(ui, tr.t("lyrics.title")).color(pal.text));
        ui.add_space(6.0);
        let Some(timeline) = self.loaded_timeline.as_ref().filter(|t| !t.is_empty()) else {
            let hint = if self.play_path.is_empty() { tr.t("now.no_track") } else { tr.t("now.no_lyrics_hint") };
            empty_state(ui, Icon::Lyrics, tr.t("now.no_lyrics"), hint, None);
            return Vec::new();
        };
        let lines = lyric_lines(timeline, self.config.karaoke);
        let pos = timeline.position_at(self.play_position_ms);
        let props = LyricsProps {
            lines: &lines,
            revision: self.lyrics_revision,
            current: pos.map(|p| p.line_index),
            highlighted_chars: pos.map(|p| p.highlighted_chars).unwrap_or(0.0),
            offset_ms: self.lyrics_offset_ms,
            font_size: (self.config.lyric_font_size * 0.75).clamp(14.0, 40.0),
            align: egui::Align::Min,
            on_backdrop: false,
            show_controls: true,
            can_save_offset: self.selected_lyrics_index.is_some(),
            line_shift: self.lyrics_shift.lines,
            char_shift: self.lyrics_shift.chars,
            can_char_shift: self.loaded_timeline.as_ref().is_some_and(|t| t.karaoke_char_count() > 1),
            can_generate_timing: !self.config.align_command.trim().is_empty() && self.selected_lyrics_index.is_some(),
            generating_timing: self.lyric_alignment_running(),
            tr,
        };
        lyrics_view(ui, &props, &mut self.ui.side_lyrics)
    }

    fn eq_popover(&mut self, ctx: &egui::Context, tr: Tr) -> Vec<UiAction> {
        let pal = theme::palette_for(ctx, ctx.global_style().visuals.dark_mode);
        let mut actions = Vec::new();
        let mut close = false;
        egui::Area::new(Id::new("eq_popover"))
            .order(egui::Order::Foreground)
            .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -(shell::transport::HEIGHT + 8.0)))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.elevated)
                    .stroke(egui::Stroke::new(1.0, pal.border))
                    .corner_radius(theme::radius::LG)
                    .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 28, spread: 0, color: pal.shadow })
                    .inner_margin(16)
                    .show(ui, |ui| {
                        ui.set_width(540.0f32.min(ctx.content_rect().width() - 64.0));
                        actions.extend(eq_panel(ui, &EqPanelProps { settings: self.eq, tr }));
                        ui.add_space(6.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Same 8 px inset as the EQ panel's own content.
                            ui.add_space(8.0);
                            if button(ui, ButtonKind::Ghost, None, tr.t("dialog.close")).clicked() {
                                close = true;
                            }
                        });
                    });
            });
        if close {
            self.ui.eq_open = false;
        }
        actions
    }

    fn dialogs_ui(&mut self, ctx: &egui::Context, tr: Tr) -> Vec<UiAction> {
        let mut actions = Vec::new();
        if let Some(modal) = self.ui.modal.clone() {
            // One id per dialog kind, so switching dialogs counts as opening a new one.
            let id = Id::new(("app_modal", std::mem::discriminant(&modal)));
            let mut next: Option<Option<Modal>> = None;
            match modal {
                Modal::CreatePlaylist { mut name, tracks } => {
                    let from_queue = !tracks.is_empty();
                    let title = if from_queue { tr.t("queue.save_as_playlist") } else { tr.t("playlist.new") };
                    let spec = DialogSpec {
                        id,
                        title,
                        width: 400.0,
                        confirm: DialogButton::primary(tr.t("playlist.create")),
                        cancel: Some(tr.t("dialog.cancel")),
                        extra: None,
                    };
                    let playlists = &self.playlists;
                    let mut valid = false;
                    let result = dialog(ctx, &spec, |ui, opened| {
                        if from_queue {
                            dialog_text(ui, &tr.f("playlist.from_queue_body", &[&format_count(tracks.len())]));
                        }
                        dialog_text_field(ui, &mut name, opened, tr.t("playlist.name_hint"));
                        let err = playlists::validate_name(playlists, &name, None).err();
                        if let Some(e) = err.filter(|e| *e != playlists::NameError::Empty) {
                            dialog_error(ui, tr.t(e.key()));
                        }
                        valid = err.is_none();
                        valid
                    });
                    match result {
                        DialogResult::Confirm if valid => {
                            if let Ok(id) = self.create_playlist(&name, tracks.clone()) {
                                actions.push(UiAction::OpenPlaylist(id));
                            }
                            next = Some(None);
                        }
                        DialogResult::Cancel => next = Some(None),
                        _ => next = Some(Some(Modal::CreatePlaylist { name, tracks })),
                    }
                }
                Modal::RenamePlaylist { id: pid, mut name } => {
                    let spec = DialogSpec {
                        id,
                        title: tr.t("playlist.rename"),
                        width: 400.0,
                        confirm: DialogButton::primary(tr.t("playlist.rename_ok")),
                        cancel: Some(tr.t("dialog.cancel")),
                        extra: None,
                    };
                    let playlists = &self.playlists;
                    let mut valid = false;
                    let result = dialog(ctx, &spec, |ui, opened| {
                        dialog_text_field(ui, &mut name, opened, tr.t("playlist.name_hint"));
                        let err = playlists::validate_name(playlists, &name, Some(&pid)).err();
                        if let Some(e) = err.filter(|e| *e != playlists::NameError::Empty) {
                            dialog_error(ui, tr.t(e.key()));
                        }
                        valid = err.is_none();
                        valid
                    });
                    match result {
                        DialogResult::Confirm if valid => {
                            let _ = self.rename_playlist(&pid, &name);
                            next = Some(None);
                        }
                        DialogResult::Cancel => next = Some(None),
                        _ => next = Some(Some(Modal::RenamePlaylist { id: pid, name })),
                    }
                }
                Modal::ClearLibrary => {
                    let spec = DialogSpec {
                        id,
                        title: tr.t("folders.clear_library"),
                        width: 420.0,
                        confirm: DialogButton::danger(tr.t("folders.clear")),
                        cancel: Some(tr.t("dialog.cancel")),
                        extra: None,
                    };
                    match dialog(ctx, &spec, |ui, _| {
                        dialog_text(ui, tr.t("folders.clear_confirm"));
                        true
                    }) {
                        DialogResult::Confirm => {
                            self.clear_library();
                            next = Some(None);
                        }
                        DialogResult::Cancel => next = Some(None),
                        _ => {}
                    }
                }
                Modal::DeletePlaylist { id: pid, name } => {
                    let spec = DialogSpec {
                        id,
                        title: tr.t("playlist.delete_title"),
                        width: 420.0,
                        confirm: DialogButton::danger(tr.t("playlist.delete")),
                        cancel: Some(tr.t("dialog.cancel")),
                        extra: None,
                    };
                    match dialog(ctx, &spec, |ui, _| {
                        dialog_text(ui, &tr.f("playlist.delete_body", &[&name]));
                        true
                    }) {
                        DialogResult::Confirm => {
                            self.delete_playlist(&pid);
                            next = Some(None);
                        }
                        DialogResult::Cancel => next = Some(None),
                        _ => {}
                    }
                }
            }
            if let Some(n) = next {
                self.ui.modal = n;
            }
        }

        if let Some(details) = self.ui.details.clone() {
            let row = self.library.display_row(&details.path);
            let spec = DialogSpec {
                id: Id::new("details_modal"),
                title: tr.t("details.title"),
                width: 480.0,
                confirm: DialogButton::primary(tr.t("dialog.close")),
                cancel: None,
                extra: Some(DialogButton::secondary(Some(Icon::Folder), tr.t("menu.open_folder"))),
            };
            let result = dialog(ctx, &spec, |ui, _| {
                let pal = theme::palette(ui);
                let none = "—".to_string();
                let fields: [(&str, String); 8] = [
                    (tr.t("table.title"), row.title.clone()),
                    (tr.t("table.artist"), if row.artist.is_empty() { none.clone() } else { row.artist.clone() }),
                    (tr.t("table.album"), if row.album.is_empty() { none.clone() } else { row.album.clone() }),
                    (tr.t("table.duration"), format_duration_opt(row.duration_ms)),
                    (tr.t("details.track_no"), row.track_no.map(|n| n.to_string()).unwrap_or_else(|| none.clone())),
                    (tr.t("table.year"), row.year.map(|y| y.to_string()).unwrap_or_else(|| none.clone())),
                    (tr.t("table.genre"), if row.genre.is_empty() { none.clone() } else { row.genre.clone() }),
                    (tr.t("details.path"), details.path.clone()),
                ];
                egui::Grid::new("details_grid").num_columns(2).spacing(vec2(16.0, 8.0)).show(ui, |ui| {
                    for (k, v) in fields {
                        ui.label(egui::RichText::new(k).color(pal.text_muted));
                        // Values are user content (title, artist, album, genre, path).
                        ui.add(egui::Label::new(crate::presentation::ui::text::rich(v, theme::size::BODY, false).color(pal.text)).wrap());
                        ui.end_row();
                    }
                });
                true
            });
            match result {
                DialogResult::Extra => actions.push(UiAction::OpenContainingFolder(Path::new(&details.path).to_path_buf())),
                DialogResult::Confirm | DialogResult::Cancel => self.ui.details = None,
                DialogResult::Open => {}
            }
        }
        actions
    }
}
