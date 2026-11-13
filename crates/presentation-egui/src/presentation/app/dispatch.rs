//! Applies [`UiAction`]s emitted by the `ui` components, plus toast helpers and small
//! platform integrations (open containing folder).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use lrc_player_domain::model::PlaybackState;

use crate::presentation::app::{ArtistViewMode, DetailsDialog, GuiApp, Modal, NavLocation};
use crate::presentation::ui::{
    widgets::{gain_to_ui, ToastKind},
    Page, RightPanel, UiAction,
};

/// Locations kept for "Back".
const HISTORY_LIMIT: usize = 50;

/// Identical toasts within this window are dropped (e.g. repeated engine errors).
const TOAST_DEDUP: Duration = Duration::from_secs(2);

fn path_strings(paths: Vec<PathBuf>) -> Vec<String> {
    paths.into_iter().map(|p| p.to_string_lossy().into_owned()).collect()
}

/// Opens the system file manager at `path` (selecting it where supported).
pub(crate) fn open_containing_folder(path: &Path) -> std::io::Result<()> {
    let mut cmd;
    #[cfg(target_os = "windows")]
    {
        cmd = std::process::Command::new("explorer");
        cmd.arg(format!("/select,{}", path.display()));
    }
    #[cfg(target_os = "macos")]
    {
        cmd = std::process::Command::new("open");
        cmd.arg("-R").arg(path);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        cmd = std::process::Command::new("xdg-open");
        cmd.arg(path.parent().unwrap_or(path));
    }
    let mut child = cmd.spawn()?;
    // Reap the child without blocking the UI.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

impl GuiApp {
    fn push_toast(&mut self, kind: ToastKind, text: String) {
        if let Some((last, at)) = &self.ui.last_toast {
            if *last == text && at.elapsed() < TOAST_DEDUP {
                return;
            }
        }
        self.ui.last_toast = Some((text.clone(), Instant::now()));
        self.ui.toasts.push(kind, text, kind.default_secs());
    }

    pub(crate) fn toast_info(&mut self, text: String) {
        self.push_toast(ToastKind::Info, text);
    }
    pub(crate) fn toast_success(&mut self, text: String) {
        self.push_toast(ToastKind::Success, text);
    }
    pub(crate) fn toast_warning(&mut self, text: String) {
        self.push_toast(ToastKind::Warning, text);
    }
    pub(crate) fn toast_error(&mut self, text: String) {
        self.push_toast(ToastKind::Error, text);
    }

    /// Toast with an action button (e.g. "Show queue").
    pub(crate) fn toast_with_action(&mut self, kind: ToastKind, text: String, label: &str, action: UiAction) {
        self.ui.last_toast = Some((text.clone(), Instant::now()));
        self.ui.toasts.push_with_action(kind, text, kind.default_secs() + 1.5, label, action);
    }

    fn location(&self) -> NavLocation {
        NavLocation {
            page: self.ui.page,
            album: self.ui.album_detail.clone(),
            artist: self.ui.artist_detail.clone(),
            playlist: self.ui.playlist_detail.clone(),
        }
    }

    /// Remembers the current location before moving elsewhere (for [`UiAction::Back`]).
    fn push_history(&mut self) {
        let here = self.location();
        if self.ui.history.last() != Some(&here) {
            self.ui.history.push(here);
            if self.ui.history.len() > HISTORY_LIMIT {
                self.ui.history.remove(0);
            }
        }
    }

    /// Label of the location [`UiAction::Back`] returns to (the page name).
    pub(crate) fn back_label(&self, tr: &crate::presentation::ui::Tr) -> Option<&'static str> {
        let loc = self.ui.history.last()?;
        Some(tr.t(crate::presentation::ui::shell::sidebar::page_label_key(loc.page)))
    }

    /// Returns to the previous location; the library search and table scroll are kept.
    pub(crate) fn go_back(&mut self) {
        let Some(loc) = self.ui.history.pop() else { return };
        let detail_changed = loc.album != self.ui.album_detail || loc.artist != self.ui.artist_detail || loc.playlist != self.ui.playlist_detail;
        self.ui.page = loc.page;
        self.ui.album_detail = loc.album;
        self.ui.artist_detail = loc.artist;
        self.ui.playlist_detail = loc.playlist;
        if detail_changed {
            self.reset_detail_table();
        }
        self.save_state();
    }

    /// Switches the main page (leaving any detail view).
    pub(crate) fn navigate(&mut self, page: Page) {
        if self.ui.page == page && self.ui.album_detail.is_none() && self.ui.artist_detail.is_none() && self.ui.playlist_detail.is_none() {
            return;
        }
        self.push_history();
        self.ui.page = page;
        self.ui.album_detail = None;
        self.ui.artist_detail = None;
        self.ui.playlist_detail = None;
        self.reset_detail_table();
        self.save_state();
    }

    pub(crate) fn reset_detail_table(&mut self) {
        self.ui.detail_table = Default::default();
        self.ui.detail_query.clear();
    }

    pub(crate) fn open_album(&mut self, key: String) {
        self.push_history();
        self.ui.page = Page::Albums;
        self.ui.album_detail = Some(key);
        self.ui.artist_detail = None;
        self.ui.playlist_detail = None;
        self.reset_detail_table();
        self.save_state();
    }

    pub(crate) fn open_artist(&mut self, name: String) {
        self.push_history();
        self.ui.page = Page::Artists;
        self.ui.artist_detail = Some(name);
        self.ui.album_detail = None;
        self.ui.playlist_detail = None;
        self.reset_detail_table();
        self.save_state();
    }

    pub(crate) fn open_playlist(&mut self, id: String) {
        self.push_history();
        self.ui.page = Page::Playlists;
        self.ui.playlist_detail = Some(id);
        self.ui.album_detail = None;
        self.ui.artist_detail = None;
        self.reset_detail_table();
        self.save_state();
    }

    pub(crate) fn dispatch_all(&mut self, ctx: &egui::Context, actions: Vec<UiAction>) {
        for action in actions {
            self.dispatch(ctx, action);
        }
    }

    pub(crate) fn dispatch(&mut self, ctx: &egui::Context, action: UiAction) {
        let tr = self.tr();
        match action {
            // ----- transport -----
            UiAction::PlayPause => self.toggle_play_pause(),
            UiAction::Next => self.next(),
            UiAction::Previous => self.previous(),
            UiAction::Seek(ms) => self.seek_to(ms),
            UiAction::SeekRelative(delta) => {
                if !self.play_path.is_empty() {
                    self.seek_relative(delta);
                }
            }
            UiAction::SetVolume(gain) => {
                self.set_volume(gain_to_ui(gain));
                if self.muted && gain > 0.0 {
                    self.set_muted(false);
                }
            }
            UiAction::ToggleMute => self.toggle_mute(),
            UiAction::ToggleShuffle => {
                let on = !self.shuffle();
                self.set_shuffle(on);
            }
            UiAction::CycleRepeat => self.cycle_repeat(),

            // ----- navigation / chrome -----
            UiAction::Navigate(page) => self.navigate(page),
            UiAction::Back => self.go_back(),
            UiAction::ToggleSidebar => self.save_state(),
            UiAction::ToggleRightPanel(panel) => {
                self.ui.right_panel = if self.ui.right_panel == Some(panel) { None } else { Some(panel) };
                if self.ui.right_panel == Some(RightPanel::Lyrics) {
                    self.ui.side_lyrics.follow_current();
                }
                self.save_state();
            }
            UiAction::ToggleEq => self.ui.eq_open = !self.ui.eq_open,
            UiAction::FocusSearch => {
                let has_search = (self.ui.page == Page::Library)
                    || (self.ui.page == Page::Artists && self.ui.artist_detail.is_none());
                if !has_search {
                    self.navigate(Page::Library);
                }
                self.ui.focus_search = true;
                ctx.request_repaint();
            }
            UiAction::OpenPlaylist(id) => self.open_playlist(id),
            UiAction::OpenAlbum(key) => self.open_album(key),
            UiAction::OpenArtist(name) => {
                if name.trim().is_empty() {
                    return;
                }
                self.open_artist(name)
            }
            UiAction::OpenArtistAs { name, album_artist } => {
                if name.trim().is_empty() {
                    return;
                }
                let mode = if album_artist { ArtistViewMode::AlbumArtist } else { ArtistViewMode::Artist };
                if self.artist_view_mode != mode {
                    self.artist_view_mode = mode;
                }
                self.open_artist(name)
            }
            UiAction::ShowArtist(name) => {
                if name.trim().is_empty() {
                    return;
                }
                // A track's artist: show it in the "Artist" grouping.
                if self.artist_view_mode == ArtistViewMode::AlbumArtist {
                    self.artist_view_mode = ArtistViewMode::Artist;
                }
                self.open_artist(name)
            }

            // ----- library / tracks -----
            UiAction::PlayTracks { paths, start } => {
                let paths = path_strings(paths);
                let replaced = !self.queue.is_empty() && self.queue != paths;
                let count = paths.len();
                self.play_tracks(paths, start);
                if replaced && count > 0 {
                    self.toast_with_action(ToastKind::Info, tr.f("toast.queue_replaced", &[&count]), tr.t("toast.show_queue"), UiAction::Navigate(Page::Queue));
                }
            }
            UiAction::PlayAlbum(key) => {
                let paths = self.library.paths_of(self.library.album_rows(&key));
                self.play_tracks(paths, 0);
            }
            UiAction::PlayNext(paths) => self.play_next_tracks(path_strings(paths)),
            UiAction::Enqueue(paths) => self.enqueue_with_toast(path_strings(paths)),
            UiAction::AddToPlaylist { playlist_id, paths } => self.add_to_playlist(&playlist_id, path_strings(paths)),
            UiAction::SetFavorite { paths, favorite } => self.set_favorite(path_strings(paths), favorite),
            UiAction::ToggleFavoriteCurrent => self.toggle_favorite_current(),
            UiAction::ShowAlbumOfTrack(path) => {
                let key = self.library.album_key_of(&path).or_else(|| {
                    let row = self.library.display_row(&path.to_string_lossy());
                    self.library.find_album(&row.album, &row.artist)
                });
                match key {
                    Some(key) => self.open_album(key),
                    None => {
                        let row = self.library.display_row(&path.to_string_lossy());
                        if !row.album.is_empty() {
                            self.toast_info(tr.f("toast.album_not_in_library", &[&row.album]));
                        }
                    }
                }
            }
            UiAction::SetColumnVisible { column, visible } => {
                self.ui.library_columns.retain(|c| *c != column);
                if visible {
                    self.ui.library_columns.push(column);
                }
                self.save_state();
            }
            UiAction::ShowAlbum { album, artist } => match self.library.find_album(&album, &artist) {
                Some(key) => self.open_album(key),
                None => self.toast_info(tr.f("toast.album_not_in_library", &[&album])),
            },
            UiAction::OpenContainingFolder(path) => {
                if let Err(err) = open_containing_folder(&path) {
                    self.toast_error(tr.f("toast.open_folder_failed", &[&err]));
                }
            }
            UiAction::ShowDetails(path) => {
                self.ui.details = Some(DetailsDialog { path: path.to_string_lossy().into_owned() });
            }
            UiAction::Sort(_) | UiAction::Search(_) => {}
            // Page-local (handled by the page that owns the table).
            UiAction::Reorder { .. } | UiAction::RemoveRows(_) => {}

            // ----- queue -----
            UiAction::QueuePlayIndex(i) => self.play_queue_index(i),
            UiAction::QueueReorder { from, to } => self.queue_reorder(from, to),
            UiAction::QueueRemove(i) => self.queue_remove(i),
            UiAction::QueueClear => self.queue_clear(),
            UiAction::QueueInsert { paths, at } => self.queue_insert(path_strings(paths), at),
            UiAction::QueueSaveAsPlaylist => {
                if !self.queue.is_empty() {
                    let name = self.suggest_playlist_name(tr.t("playlist.from_queue"));
                    self.ui.modal = Some(Modal::CreatePlaylist { name, tracks: self.queue.clone() });
                }
            }

            // ----- lyrics -----
            UiAction::SeekToLyricLine { index, start_ms } => {
                let target = self
                    .loaded_timeline
                    .as_ref()
                    .and_then(|t| t.seek_position_for_line(index))
                    .unwrap_or(start_ms);
                self.seek_to(target);
                if self.playback_state == PlaybackState::Stopped && !self.play_path.is_empty() {
                    self.play();
                }
            }
            UiAction::LyricOffsetDelta(delta) => self.set_lyrics_offset(self.lyrics_offset_ms + delta),
            UiAction::LyricOffsetReset => self.set_lyrics_offset(0),
            UiAction::SaveLyricOffset => self.save_lyrics_offset_to_selected_file(),
            UiAction::LyricShiftLines(d) => self.set_lyrics_shift(self.lyrics_shift.lines + d, self.lyrics_shift.chars),
            UiAction::LyricShiftChars(d) => self.set_lyrics_shift(self.lyrics_shift.lines, self.lyrics_shift.chars + d),
            UiAction::LyricShiftReset => self.set_lyrics_shift(0, 0),
            UiAction::GenerateLyricTiming => self.start_lyric_alignment(ctx),
            UiAction::FetchLyricsOnline => self.start_lyrics_fetch(ctx, true),
            UiAction::SetLyricFontSize(size) => {
                self.config.lyric_font_size = size.clamp(14.0, 56.0);
                self.save_config();
            }

            // ----- EQ -----
            UiAction::SetEq(eq) => self.set_eq(eq),

            // ----- folders / scan -----
            UiAction::AddFolder => self.pick_folder(ctx, false),
            UiAction::RemoveFolder(path) => self.remove_folder(&path.to_string_lossy()),
            UiAction::Rescan => self.request_scan(),
            UiAction::DropFiles(paths) => self.handle_dropped_files(ctx, paths),

            // ----- appearance -----
            UiAction::SetTheme(mode) => {
                self.ui.theme = mode;
                self.config.theme = super::state::config_theme(mode);
                self.save_config();
            }
            UiAction::SetAccent(accent) => {
                self.ui.accent = accent;
                self.config.accent = accent.key().to_string();
                self.save_config();
            }
        }
    }
}
