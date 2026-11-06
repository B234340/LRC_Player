//! GUIアプリケーションのメイン構造体と App トレイト実装を提供するモジュール。
//!
//! Playback is driven by one long-lived [`AudioEngine`] (see `app/playback.rs`). The public
//! transport/volume/queue methods the UI should call are documented there
//! (`GuiApp::toggle_play_pause`, `seek_to`, `set_volume`, ...).
//!
//! Repaint policy (see [`GuiApp::schedule_repaint`]): the UI is event driven. Engine events
//! wake it through the engine waker; while audio is playing it repaints at ~30 fps only when
//! time-based content is visible (Play tab: lyrics / FFT / position), otherwise at ~4 fps;
//! nothing is scheduled while paused, stopped or minimized.

mod artwork;
mod artwork_loader;
mod fonts;
mod library_cache;
mod lyrics;
mod playback;
pub mod scan_status;
mod seek;
mod state;
mod track_info;
mod track_meta;
mod view_model;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use eframe::{App, egui};
use lrc_player_application::config::{Language, LogLevel, ResamplerSetting, Theme};
use lrc_player_application::dto::PlaylistDto;
use lrc_player_application::infrastructure::audio::engine::{AudioEngine, EngineSnapshot, EqSettings};
use lrc_player_application::infrastructure::library_snapshot::LibrarySnapshotCache;
use lrc_player_application::infrastructure::lyrics::{Lyrics, LyricsCandidate, LyricsTimeline};
use lrc_player_application::infrastructure::persistence::{SharedMetaCache, StateSaver};
use lrc_player_application::localization::translate;
use lrc_player_application::usecases::queue_navigation::QueueNavigator;
use lrc_player_application::view_model::{AppVm, QueueVm};
use lrc_player_domain::model::PlaybackState;

pub use lrc_player_application::config::ArtistViewMode;
pub use lrc_player_application::dto::RepeatMode;
pub use playback::PlaybackView;

pub use artwork_loader::ArtworkMode;
pub(crate) use artwork_loader::ArtworkLoader;
pub(crate) use lyrics::overlay_time_ms as lyric_clock_ms;
pub(crate) use state::apply_theme;
pub(crate) use track_meta::TrackMetaCache;

use crate::presentation::components::fft_overlay::FftAnalyzer;
use crate::presentation::components::lrc_overlay::LrcLayoutCache;
use crate::presentation::side_panel::output;
use crate::presentation::tabs;

/// Repaint interval while playing and time-based content (lyrics / FFT / Play tab) is visible.
const REPAINT_TIME_BASED: Duration = Duration::from_millis(33);
/// Repaint interval while playing on other tabs (enough for a time label).
const REPAINT_IDLE_PLAYING: Duration = Duration::from_millis(250);
/// How long a hot-path state change may stay unsaved before it is handed to the saver.
const STATE_SUBMIT_INTERVAL: Duration = Duration::from_millis(1000);
/// While playing, the position is persisted this often so a crash resumes nearby.
const POSITION_PERSIST_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Library,
    Play,
    Playlist,
    Album,
    Artist,
    Queue,
    Scan,
    Settings,
}

impl Tab {
    fn label(&self) -> &'static str {
        match self {
            Tab::Library => "Library",
            Tab::Play => "Play",
            Tab::Playlist => "Playlists",
            Tab::Album => "Albums",
            Tab::Artist => "Artists",
            Tab::Queue => "Queue",
            Tab::Scan => "Scan",
            Tab::Settings => "Settings",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "Library" => Some(Tab::Library),
            "Play" => Some(Tab::Play),
            "Playlists" => Some(Tab::Playlist),
            "Albums" => Some(Tab::Album),
            "Artists" => Some(Tab::Artist),
            "Queue" => Some(Tab::Queue),
            "Scan" => Some(Tab::Scan),
            "Settings" => Some(Tab::Settings),
            _ => None,
        }
    }
}

pub struct GuiApp {
    pub app_vm: AppVm,

    active_tab: Tab,

    // ---- Audio engine ----
    /// The one playback engine (None only if its thread could not be spawned).
    pub(crate) engine: Option<AudioEngine>,
    /// Snapshot taken at the start of the current frame.
    pub(crate) engine_snapshot: Option<EngineSnapshot>,
    /// Output device names for the device combo (refreshed on demand).
    pub(crate) device_names: Vec<String>,
    /// Configured output device; `None` follows the system default.
    pub(crate) preferred_output_device: Option<String>,
    pub(crate) device_filter: String,
    /// Device the engine currently renders to (from `DeviceChanged`).
    pub(crate) active_output_device: Option<String>,
    pub(crate) resampler: ResamplerSetting,
    pub(crate) taps: usize,

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

    // ---- Seeking ----
    pub(crate) pending_seek_ms: Option<u64>,
    pub(crate) seek_last_change_at: Option<Instant>,
    /// Target of the last seek sent to the engine, shown until the engine reports it.
    pub(crate) seek_display_hold: Option<(u64, Instant)>,
    seek_interacting: bool,
    seek_key_hold_direction: i8,
    seek_key_hold_started_at: Option<Instant>,
    seek_key_last_step_at: Option<Instant>,
    seek_key_hold_steps: u32,

    pub(crate) queue_scroll_to_playing: bool,
    /// One-shot scroll request for a virtualized track list: (id_source, row index).
    pub(crate) track_list_scroll_request: Option<(String, usize)>,

    pub(crate) artist_view_mode: ArtistViewMode,

    pub(crate) play_position_ms: u64,
    pub(crate) play_duration_ms: Option<u64>,
    pub(crate) play_title: Option<String>,
    pub(crate) play_artist: Option<String>,
    pub(crate) play_album: Option<String>,
    /// Background probe + lyrics discovery for the current track.
    pub(crate) track_info: track_info::TrackInfoLoader,

    pub(crate) play_artwork_handle: Option<egui::TextureHandle>,
    pub(crate) play_artwork_blur_handle: Option<egui::TextureHandle>,
    /// Track whose Play-tab artwork was requested from the worker (also caches "none").
    pub(crate) play_artwork_requested_for: Option<String>,

    pub(crate) lrc_overlay_enabled: bool,
    pub(crate) fft_overlay_enabled: bool,
    pub(crate) lyrics_files: Vec<std::path::PathBuf>,
    pub(crate) lyrics_candidates: Vec<LyricsCandidate>,
    pub(crate) selected_lyrics_index: Option<usize>,
    pub(crate) loaded_lyrics: Option<Lyrics>,
    /// Timeline of the loaded lyrics; owns the `[offset:]` tag + user offset (applied once).
    pub(crate) loaded_timeline: Option<LyricsTimeline>,
    pub(crate) lyrics_offset_ms: i64,
    /// Per-track user lyrics offsets (persisted).
    pub(crate) lyrics_offsets: HashMap<String, i64>,
    pub(crate) lrc_layout: LrcLayoutCache,
    pub(crate) fft: FftAnalyzer,

    pub(crate) play_status: Option<String>,

    pub(crate) scan_target_input: String,
    pub(crate) scan_lrc_target_input: String,
    pub(crate) scan_exclude_input: String,
    pub(crate) scan_targets: Vec<String>,
    pub(crate) scan_lrc_targets: Vec<String>,
    pub(crate) scan_excludes: Vec<String>,

    pub(crate) selected_album: Option<String>,
    pub(crate) selected_artist: Option<String>,
    pub(crate) selected_directory: Option<String>,
    pub(crate) favorite_directories: Vec<String>,
    pub(crate) favorite_directory_input: String,
    pub(crate) library_expanded_directories: HashSet<String>,

    pub(crate) playlists: Vec<PlaylistDto>,
    /// Paths in the favorite playlist (playlists[0]) for O(1) per-row lookups.
    pub(crate) favorite_set: HashSet<String>,
    pub(crate) new_playlist_name: String,
    pub(crate) selected_playlist: Option<usize>,
    pub(crate) playlist_track_input: String,

    pub(crate) queue: QueueVm,
    pub(crate) queue_item_input: String,
    pub(crate) selected_queue_index: Option<usize>,
    pub(crate) selected_queue_indices: Vec<usize>,
    pub(crate) queue_selection_anchor: Option<usize>,
    pub(crate) queue_handle_hold_index: Option<usize>,
    pub(crate) queue_handle_hold_started_at: Option<Instant>,
    pub(crate) queue_drag_source_index: Option<usize>,

    pub(crate) side_panel_expanded: bool,
    pub(crate) theme: Theme,
    pub(crate) log_level: LogLevel,
    pub(crate) language: Language,

    pub(crate) last_scan_status: Option<String>,
    pub(crate) scan_counts: (Option<u64>, Option<u64>, Option<u64>),

    // ---- Data layer ----
    /// One long-lived SQLite connection for UI reads (`try_with` never blocks).
    pub(crate) meta: Option<SharedMetaCache>,
    pub(crate) library_snapshot: LibrarySnapshotCache,
    /// Display metadata for track rows, filled by batched queries.
    pub(crate) track_meta: TrackMetaCache,
    /// Background artwork decoding / thumbnail cache (list rows, album grid, Play tab).
    pub(crate) artwork: ArtworkLoader,

    // ---- Persistence ----
    state_saver: Option<StateSaver>,
    state_dirty: bool,
    state_last_submit: Option<Instant>,

    pub(crate) track_details_dialog: Option<TrackDetailsDialog>,
}

#[derive(Debug, Clone)]
pub(crate) struct TrackDetailsDialog {
    pub(crate) path: String,
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) album: String,
}

impl GuiApp {
    /// Currently shown tab.
    pub fn active_tab(&self) -> Tab {
        self.active_tab
    }

    /// Repaint policy: event-driven with a low-rate tick only while audio plays.
    fn schedule_repaint(&self, ctx: &egui::Context) {
        let minimized = ctx.input(|i| {
            let vp = i.viewport();
            vp.minimized.unwrap_or(false) || vp.occluded.unwrap_or(false)
        });
        if minimized {
            // Engine events still wake the UI through the waker.
            return;
        }
        if self.pending_seek_ms.is_some() {
            ctx.request_repaint_after(seek::SEEK_DEBOUNCE);
            return;
        }
        match self.playback_state {
            PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => {
                let time_based_visible = self.active_tab == Tab::Play;
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.poll_scan_status();
        self.poll_engine();
        self.poll_track_info();
        self.artwork.poll(&ctx);
        self.handle_global_keyboard_shortcuts(&ctx);

        egui::Panel::top("top_panel").show(ui, |ui| {
            let tab_size = ui.spacing().interact_size * 2.0;
            ui.heading(translate("title", self.language));
            ui.horizontal(|ui| {
                for tab in [
                    Tab::Library,
                    Tab::Play,
                    Tab::Playlist,
                    Tab::Album,
                    Tab::Artist,
                    Tab::Queue,
                    Tab::Scan,
                    Tab::Settings,
                ] {
                    if ui
                        .add_sized(
                            tab_size,
                            egui::Button::selectable(self.active_tab == tab, tab.label()),
                        )
                        .clicked()
                        && self.active_tab != tab
                    {
                        self.active_tab = tab;
                        if tab == Tab::Queue {
                            self.queue_scroll_to_playing = true;
                        }
                        // Cheap: library data comes from LibrarySnapshotCache.
                        self.update_app_vm();
                    }
                }
            });
        });

        egui::Panel::left("side_panel")
            .resizable(false)
            .min_size(if self.side_panel_expanded { 240.0 } else { 32.0 })
            .default_size(if self.side_panel_expanded { 240.0 } else { 32.0 })
            .show(ui, |ui| {
                output::ui(ui, self);
            });

        egui::CentralPanel::default().show(ui, |ui| match self.active_tab {
            Tab::Library => tabs::library_tab::ui(ui, self),
            Tab::Play => tabs::play_tab::ui(ui, self),
            Tab::Playlist => tabs::playlist_tab::ui(ui, self),
            Tab::Album => tabs::album_tab::ui(ui, self),
            Tab::Artist => tabs::artist_tab::ui(ui, self),
            Tab::Queue => tabs::queue_tab::ui(ui, self),
            Tab::Scan => tabs::scan_tab::ui(ui, self),
            Tab::Settings => tabs::settings_tab::ui(ui, self),
        });

        self.artwork.end_frame();
        self.maybe_submit_state();
        self.schedule_repaint(&ctx);
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
