//! Applies [`UiAction`]s emitted by the `ui` components, plus toast helpers and small
//! platform integrations (open containing folder).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use lrc_player_domain::model::PlaybackState;

use crate::presentation::app::{DetailsDialog, GuiApp, Modal};
use crate::presentation::ui::{
    widgets::{gain_to_ui, ToastKind},
    Page, RightPanel, UiAction,
};

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

    /// Switches the main page (leaving any detail view).
    pub(crate) fn navigate(&mut self, page: Page) {
        if self.ui.page == page && self.ui.album_detail.is_none() && self.ui.artist_detail.is_none() && self.ui.playlist_detail.is_none() {
            return;
        }
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
        self.ui.page = Page::Albums;
        self.ui.album_detail = Some(key);
        self.ui.artist_detail = None;
        self.ui.playlist_detail = None;
        self.reset_detail_table();
        self.save_state();
    }

    pub(crate) fn open_artist(&mut self, name: String) {
        self.ui.page = Page::Artists;
        self.ui.artist_detail = Some(name);
        self.ui.album_detail = None;
        self.ui.playlist_detail = None;
        self.reset_detail_table();
        self.save_state();
    }

    pub(crate) fn open_playlist(&mut self, id: String) {
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
            UiAction::OpenArtist(name) | UiAction::ShowArtist(name) => {
                if name.trim().is_empty() {
                    return;
                }
                self.open_artist(name)
            }

            // ----- library / tracks -----
            UiAction::PlayTracks { paths, start } => self.play_tracks(path_strings(paths), start),
            UiAction::PlayAlbum(key) => {
                let paths = self.library.paths_of(self.library.album_rows(&key));
                self.play_tracks(paths, 0);
            }
            UiAction::PlayNext(paths) => self.play_next_tracks(path_strings(paths)),
            UiAction::Enqueue(paths) => self.enqueue_with_toast(path_strings(paths)),
            UiAction::AddToPlaylist { playlist_id, paths } => self.add_to_playlist(&playlist_id, path_strings(paths)),
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
