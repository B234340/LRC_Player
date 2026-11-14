//! GUIアプリケーションのメイン構造体と App トレイト実装を提供するモジュール。
//!
//! Playback is driven by one long-lived [`AudioEngine`] (see `app/playback.rs`). The public
//! transport/volume/queue methods the UI calls are documented there
//! (`GuiApp::toggle_play_pause`, `seek_to`, `set_volume`, ...).
//!
//! The window is composed from the `ui` component library (`app/shell.rs`): sidebar,
//! transport bar, optional right panel (queue / lyrics), the selected page (`app/pages`),
//! toasts and the file-drop overlay. Components return [`UiAction`]s which
//! `app/dispatch.rs` applies.
//!
//! Repaint policy (see [`GuiApp::schedule_repaint`]): the UI is event driven. Engine events
//! and workers wake it through `request_repaint`; while audio is playing it repaints at
//! ~30 fps only when time-based content is visible (Now Playing, lyrics panel), otherwise
//! at ~4 fps for the transport time; nothing is scheduled while paused, stopped or
//! minimized. Animations (lyrics scroll easing, toasts, hover fades) request their own
//! frames only while they run.
//!
//! [`UiAction`]: crate::presentation::ui::UiAction

mod artwork;
mod audio_quality;
mod artwork_loader;
mod dispatch;
mod drop;
pub(crate) mod library_index;
mod loudness;
mod lyric_align;
mod lyric_fetch;
mod lyrics;
mod media_controls;
mod pages;
mod playback;
mod playlists;
mod queue_ops;
mod scan;
pub mod scan_status;
mod seek;
mod shell;
mod spectrum;
mod state;
mod track_info;
mod window_scale;

#[cfg(test)]
mod screenshot_tests;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use eframe::{egui, App};
use lrc_player_application::config::Config;
use lrc_player_application::dto::PlaylistDto;
use lrc_player_application::infrastructure::audio::engine::{AudioEngine, EngineSnapshot, EqSettings, ParametricEq};
use lrc_player_application::infrastructure::lyrics::{LyricsCandidate, LyricsTimeline};
use lrc_player_application::infrastructure::persistence::{SharedMetaCache, StateSaver};
use lrc_player_application::usecases::queue_navigation::QueueNavigator;
use lrc_player_domain::model::PlaybackState;

pub use lrc_player_application::config::ArtistViewMode;
pub use lrc_player_application::dto::RepeatMode;
pub use playback::PlaybackView;
pub use state::{initial_window, AppOptions};

pub use artwork_loader::ArtworkMode;
pub(crate) use artwork_loader::ArtworkLoader;
pub(crate) use audio_quality::EXCLUSIVE_SUPPORTED;

use crate::presentation::ui::{
    shell::SidebarState,
    views::{Column, EqCurve, EqTab, LyricsViewState, NowPlayingState, TrackTableState},
    widgets::Toasts,
    Accent, Lang, Page, RightPanel, ThemeMode, Tr,
};

/// Repaint interval while playing and time-based content (lyrics / spectrum) is visible.
const REPAINT_TIME_BASED: Duration = Duration::from_millis(33);
/// Repaint interval while playing elsewhere (enough for the transport time label).
const REPAINT_IDLE_PLAYING: Duration = Duration::from_millis(250);
/// Wake-up while minimized and playing (logic only: media overlay position, state saving).
const REPAINT_BACKGROUND_PLAYING: Duration = Duration::from_secs(1);
/// How long a hot-path state change may stay unsaved before it is handed to the saver.
const STATE_SUBMIT_INTERVAL: Duration = Duration::from_millis(1000);
/// While playing, the position is persisted this often so a crash resumes nearby.
const POSITION_PERSIST_INTERVAL: Duration = Duration::from_secs(10);

/// A confirmation / input dialog shown over the shell.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Modal {
    /// New playlist (optionally pre-filled with tracks, e.g. "save queue as playlist").
    CreatePlaylist { name: String, tracks: Vec<String> },
    RenamePlaylist { id: String, name: String },
    DeletePlaylist { id: String, name: String },
    /// Remove every track from the library database.
    ClearLibrary,
}

/// A place in the app (page + detail view), for "Back".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NavLocation {
    pub(crate) page: Page,
    pub(crate) album: Option<String>,
    pub(crate) artist: Option<String>,
    pub(crate) playlist: Option<String>,
}

/// The "Details" dialog for one track.
#[derive(Debug, Clone)]
pub(crate) struct DetailsDialog {
    pub(crate) path: String,
}

/// Shell / navigation state that only the UI uses.
pub(crate) struct UiShell {
    pub(crate) page: Page,
    pub(crate) album_detail: Option<String>,
    pub(crate) artist_detail: Option<String>,
    pub(crate) playlist_detail: Option<String>,
    pub(crate) right_panel: Option<RightPanel>,
    pub(crate) sidebar: SidebarState,
    pub(crate) eq_open: bool,
    /// Tab shown in the EQ popover.
    pub(crate) eq_tab: EqTab,
    /// Cached frequency response for the EQ popover graph.
    pub(crate) eq_curve: Option<EqCurve>,
    /// Signal-path popover (opened from the transport's quality indicator).
    pub(crate) signal_open: bool,
    /// Frames left in which Settings scrolls its 音質 section into view.
    pub(crate) scroll_to_sound: u8,
    /// Pending AutoEq file picker (see `app/audio_quality.rs`).
    pub(crate) autoeq_picker: Option<Receiver<audio_quality::AutoEqPick>>,
    pub(crate) toasts: Toasts,
    pub(crate) library_table: TrackTableState,
    pub(crate) library_query: String,
    /// Optional library columns that are shown (Year / Genre; persisted).
    pub(crate) library_columns: Vec<Column>,
    /// Previous locations (most recent last) for "Back".
    pub(crate) history: Vec<NavLocation>,
    /// Album grid card width (px, persisted).
    pub(crate) album_card_size: f32,
    /// Table of the album / artist / playlist detail page (reset on navigation).
    pub(crate) detail_table: TrackTableState,
    pub(crate) detail_query: String,
    pub(crate) artist_query: String,
    pub(crate) focus_search: bool,
    pub(crate) now: NowPlayingState,
    pub(crate) side_lyrics: LyricsViewState,
    pub(crate) theme: ThemeMode,
    pub(crate) accent: Accent,
    pub(crate) modal: Option<Modal>,
    pub(crate) details: Option<DetailsDialog>,
    /// Pending native folder picker: (is lyrics folder, result channel).
    pub(crate) folder_picker: Option<(bool, Receiver<Option<PathBuf>>)>,
    /// Fonts being built on a worker.
    pub(crate) fonts_rx: Option<Receiver<egui::FontDefinitions>>,
    /// Pending folder expansion of dropped files (play now?, result channel).
    pub(crate) drop_rx: Option<(bool, Receiver<Vec<String>>)>,
    /// Running online lyrics lookup (see `app/lyric_fetch.rs`).
    pub(crate) fetch_job: Option<lyric_fetch::FetchJob>,
    /// Tracks already looked up automatically this session.
    pub(crate) fetch_tried: std::collections::HashSet<String>,
    /// Running external lyric-timing job (see `app/lyric_align.rs`).
    pub(crate) align_job: Option<lyric_align::AlignJob>,
    /// Settings "Check" for the timing command.
    pub(crate) align_check: Option<Receiver<Result<String, lyric_align::AlignError>>>,
    pub(crate) exclude_input: String,
    /// Last (message, time) toast, to drop identical repeats.
    pub(crate) last_toast: Option<(String, Instant)>,
    pub(crate) window: Option<lrc_player_application::dto::WindowStateDto>,
    /// Rows of the current detail page (album / artist / playlist), rebuilt on change.
    pub(crate) detail_rows: pages::DetailRows,
}

pub struct GuiApp {
    // ---- Audio engine ----
    /// The one playback engine (None only if its thread could not be spawned / disabled).
    pub(crate) engine: Option<AudioEngine>,
    /// Snapshot taken at the start of the current frame.
    pub(crate) engine_snapshot: Option<EngineSnapshot>,
    /// Output device names for the device combo (refreshed on demand).
    pub(crate) device_names: Vec<String>,
    /// The engine could not open any audio output (no device). Shown as a quiet indicator;
    /// cleared when `DeviceChanged` reports a device.
    pub(crate) no_output_device: bool,
    /// Configured output device; `None` follows the system default.
    pub(crate) preferred_output_device: Option<String>,
    /// Device the engine currently renders to (from `DeviceChanged`).
    pub(crate) active_output_device: Option<String>,
    /// Settings file contents (saved on change when persistence is on).
    pub(crate) config: Config,

    // ---- Transport ----
    pub(crate) play_path: String,
    pub(crate) playback_state: PlaybackState,
    pub(crate) navigator: QueueNavigator,
    pub(crate) current_queue_index: Option<usize>,
    /// Track handed to `AudioEngine::set_next` (queue index, path) for gapless playback.
    pub(crate) pending_next: Option<(usize, String)>,
    /// Consecutive load errors (guards against skipping forever through broken files).
    pub(crate) consecutive_errors: usize,
    /// The current track reported an error; skip it once the engine reports Stopped.
    pub(crate) pending_error_skip: bool,
    /// 0..=1 perceptual volume (UI value, persisted).
    pub(crate) volume: f32,
    pub(crate) muted: bool,
    pub(crate) eq: EqSettings,
    /// Parametric EQ (mirrors `config.parametric_eq`).
    pub(crate) peq: ParametricEq,
    /// Last `ExclusiveUnavailable` reason while exclusive mode is requested (shown in
    /// Settings; one toast per distinct reason).
    pub(crate) exclusive_unavailable: Option<String>,
    /// A debounced `config.json` write is pending (see `GuiApp::save_config_soon`).
    pub(crate) config_save_due: Option<Instant>,

    // ---- Seeking ----
    pub(crate) pending_seek_ms: Option<u64>,
    pub(crate) seek_last_change_at: Option<Instant>,
    /// Target of the last seek sent to the engine, shown until the engine reports it.
    pub(crate) seek_display_hold: Option<(u64, Instant)>,

    pub(crate) play_position_ms: u64,
    pub(crate) play_duration_ms: Option<u64>,
    pub(crate) play_title: Option<String>,
    pub(crate) play_artist: Option<String>,
    pub(crate) play_album: Option<String>,
    /// Background probe + lyrics discovery for the current track.
    pub(crate) track_info: track_info::TrackInfoLoader,
    /// EBU R128 measurement of untagged files -> engine loudness hints (`app/loudness.rs`).
    pub(crate) loudness: loudness::LoudnessAnalyzer,

    pub(crate) play_artwork_handle: Option<egui::TextureHandle>,
    pub(crate) play_backdrop_handle: Option<egui::TextureHandle>,
    /// Track whose artwork was requested from the worker (also caches "none").
    pub(crate) play_artwork_requested_for: Option<String>,
    /// Cover image file for the OS media overlay (cover file or extracted embedded art).
    pub(crate) play_cover_path: Option<PathBuf>,

    // ---- Lyrics ----
    pub(crate) lyrics_candidates: Vec<LyricsCandidate>,
    pub(crate) selected_lyrics_index: Option<usize>,
    /// Timeline of the loaded lyrics; owns the `[offset:]` tag + user offset (applied once).
    pub(crate) loaded_timeline: Option<LyricsTimeline>,
    /// Bumped when `loaded_timeline` is replaced (lyrics view galley cache key).
    pub(crate) lyrics_revision: u64,
    pub(crate) lyrics_offset_ms: i64,
    /// Track whose lyrics discovery (worker) has finished.
    pub(crate) lyrics_discovered_for: String,
    /// Lyric text shift of the current track.
    pub(crate) lyrics_shift: lrc_player_application::dto::LyricsShiftDto,
    /// Per-track lyric text shifts (persisted).
    pub(crate) lyrics_shifts: HashMap<String, lrc_player_application::dto::LyricsShiftDto>,
    /// Per-track user lyrics offsets (persisted).
    pub(crate) lyrics_offsets: HashMap<String, i64>,
    /// `.lrc` files attached by drag & drop (track -> lyrics file, persisted).
    pub(crate) lyrics_overrides: HashMap<String, String>,
    pub(crate) spectrum: spectrum::FftAnalyzer,

    // ---- Library / scan ----
    pub(crate) scan_targets: Vec<String>,
    pub(crate) scan_lrc_targets: Vec<String>,
    pub(crate) scan_excludes: Vec<String>,
    pub(crate) last_scan_status: Option<String>,
    pub(crate) scan_counts: (Option<u64>, Option<u64>, Option<u64>),
    pub(crate) scanning: bool,
    /// Persisted for older builds (no longer shown).
    pub(crate) favorite_directories: Vec<String>,
    pub(crate) artist_view_mode: ArtistViewMode,

    pub(crate) playlists: Vec<PlaylistDto>,
    /// Paths in the favorites list (rebuilt whenever the playlists are saved).
    pub(crate) favorites: std::collections::HashSet<String>,
    pub(crate) queue: Vec<String>,
    pub(crate) queue_rows: shell::QueueRowsCache,

    // ---- Data layer ----
    /// One long-lived SQLite connection for UI reads/writes.
    pub(crate) meta: Option<SharedMetaCache>,
    pub(crate) library: library_index::LibraryIndex,
    /// Background artwork decoding / thumbnail cache.
    pub(crate) artwork: ArtworkLoader,

    // ---- UI ----
    pub(crate) ui: UiShell,
    pub(crate) media: media_controls::MediaControlsBridge,

    // ---- Persistence ----
    /// False in tests / screenshots: nothing is written to the data directory.
    pub(crate) persist: bool,
    state_saver: Option<StateSaver>,
    state_dirty: bool,
    state_last_submit: Option<Instant>,
}

impl GuiApp {
    /// Translator for the configured language.
    pub(crate) fn tr(&self) -> Tr {
        Tr::new(match self.config.language {
            lrc_player_application::config::Language::Ja => Lang::Ja,
            _ => Lang::En,
        })
    }

    /// Currently shown page.
    pub fn page(&self) -> Page {
        self.ui.page
    }

    /// Repaint policy: event-driven with a low-rate tick only while audio plays.
    /// Media overlay sync, state saving and the next wake-up (after logic and after ui).
    fn after_frame(&mut self, ctx: &egui::Context) {
        if self.media.is_active() {
            let view = self.playback_view();
            self.media.sync(&view, self.play_cover_path.as_deref());
        }
        self.maybe_submit_state();
        self.flush_config_if_due(false);
        self.schedule_repaint(ctx);
    }

    fn schedule_repaint(&self, ctx: &egui::Context) {
        let minimized = ctx.input(|i| {
            let vp = i.viewport();
            vp.minimized.unwrap_or(false) || vp.occluded.unwrap_or(false)
        });
        if minimized {
            // Only `logic` runs. Engine events, workers and media keys wake it; a pending seek
            // (media keys) needs its debounce, and while playing a slow tick keeps the OS media
            // overlay position and the saved state current.
            if self.pending_seek_ms.is_some() {
                ctx.request_repaint_after(seek::SEEK_DEBOUNCE);
            } else if self.playback_state == PlaybackState::Playing {
                ctx.request_repaint_after(REPAINT_BACKGROUND_PLAYING);
            }
            return;
        }
        if self.pending_seek_ms.is_some() {
            ctx.request_repaint_after(seek::SEEK_DEBOUNCE);
            return;
        }
        if self.ui.folder_picker.is_some() || self.ui.fonts_rx.is_some() || self.ui.drop_rx.is_some() || self.ui.autoeq_picker.is_some() {
            // Results also wake the UI; this is only a safety net.
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if let Some(due) = self.config_save_due {
            ctx.request_repaint_after(due.saturating_duration_since(Instant::now()));
        }
        match self.playback_state {
            PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => {
                // The signal path (Settings → 音質, transport popover) and the transport time
                // are refreshed by the ~4 fps tick below; nothing extra is scheduled for them.
                let time_based_visible = self.ui.page == Page::NowPlaying
                    || self.ui.right_panel == Some(RightPanel::Lyrics);
                if time_based_visible {
                    ctx.request_repaint_after(REPAINT_TIME_BASED);
                } else {
                    ctx.request_repaint_after(REPAINT_IDLE_PLAYING);
                }
            }
            PlaybackState::Paused | PlaybackState::Stopped => {}
        }
    }

    /// Hands the state to the background saver when it changed (rate limited).
    fn maybe_submit_state(&mut self) {
        let now = Instant::now();
        if self.playback_state == PlaybackState::Playing
            && self
                .state_last_submit
                .is_none_or(|t| now.duration_since(t) >= POSITION_PERSIST_INTERVAL)
        {
            self.state_dirty = true;
        }
        if !self.state_dirty {
            return;
        }
        if let Some(last) = self.state_last_submit {
            if now.duration_since(last) < STATE_SUBMIT_INTERVAL {
                return;
            }
        }
        self.submit_state_now();
    }
}

impl App for GuiApp {
    /// Everything that is not drawing. eframe calls this before every `ui`, and also while the
    /// window is minimized / hidden (then `ui` is skipped) whenever a repaint was requested
    /// (engine events, workers and media keys request one). So playback keeps advancing,
    /// the gapless successor is queued and the OS media overlay stays current in the
    /// background.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll_fonts(ctx);
        crate::presentation::ui::theme::sync(ctx, self.ui.theme, self.ui.accent);
        self.poll_scan_status();
        self.poll_engine();
        self.poll_track_info();
        self.poll_library();
        self.poll_loudness();
        self.poll_folder_picker();
        self.poll_autoeq_picker();
        self.poll_lyric_alignment(ctx);
        self.poll_lyrics_fetch(ctx);
        self.poll_dropped();
        self.artwork.poll(ctx);
        self.ensure_play_artwork();
        self.media.ensure_attached(frame, ctx);
        self.poll_media_controls();
        window_scale::install(frame);
        self.track_window_geometry(ctx);
        self.after_frame(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.draw_shell(ui);
        // Again after drawing, so actions from this frame's UI are reflected right away.
        self.after_frame(&ctx);
        self.artwork.end_frame();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }
}

impl Drop for GuiApp {
    fn drop(&mut self) {
        // `on_exit` normally ran already; this covers other teardown paths.
        self.shutdown();
    }
}
