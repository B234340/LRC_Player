//! GUIアプリケーションのメイン構造体と App トレイト実装を提供するモジュール。

mod artwork;
mod fonts;
mod library_cache;
mod lyrics;
mod playback;
mod seek;
mod state;
mod view_model;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use eframe::{App, egui};
use lrc_player_application::config::{Language, LogLevel, Theme};
use lrc_player_application::dto::PlaylistDto;
use lrc_player_application::infrastructure::lyrics::Lyrics;
use lrc_player_application::infrastructure::persistence::MetaCache;
use lrc_player_application::localization::translate;
use lrc_player_application::view_model::{AppVm, QueueVm};
use lrc_player_domain::model::{PlaybackMode, PlaybackState};

pub use lrc_player_application::config::ArtistViewMode;

use crate::presentation::side_panel::output;
use crate::presentation::tabs;

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

    pub(crate) device_names: Vec<String>,
    pub(crate) selected: Option<usize>,
    pub(crate) device_filter: String,
    active_output_device_name: Option<String>,
    last_seen_default_output_device_name: Option<String>,
    last_default_device_poll_at: Option<Instant>,

    pub(crate) play_path: String,
    pub(crate) playback_state: PlaybackState,
    pub(crate) playback_mode: PlaybackMode,
    pub(crate) current_queue_index: Option<usize>,
    pub(crate) playback_start_instant: Option<Instant>,
    playback_start_ms: u64,
    pub(crate) pending_play_request: bool,
    pub(crate) playback_state_before_seek: Option<PlaybackState>,

    playback_status_receiver: Option<Receiver<String>>,
    playback_cancel_sender: Option<Sender<()>>,
    playback_thread_handle: Option<std::thread::JoinHandle<()>>,

    pub(crate) pending_seek_ms: Option<u64>,
    pub(crate) seek_last_change_at: Option<Instant>,
    seek_interacting: bool,
    seek_long_press_start: Option<Instant>,
    seek_key_hold_direction: i8,
    seek_key_hold_started_at: Option<Instant>,
    seek_key_last_step_at: Option<Instant>,

    pub(crate) queue_scroll_to_playing: bool,

    pub(crate) artist_view_mode: ArtistViewMode,

    pub(crate) play_position_ms: u64,
    pub(crate) play_duration_ms: Option<u64>,
    pub(crate) play_title: Option<String>,
    pub(crate) play_artist: Option<String>,
    pub(crate) play_album: Option<String>,

    pub(crate) play_artwork_handle: Option<egui::TextureHandle>,
    pub(crate) play_artwork_source: Option<PathBuf>,
    pub(crate) play_artwork_blur_handle: Option<egui::TextureHandle>,
    pub(crate) play_artwork_blur_source: Option<PathBuf>,

    pub(crate) lrc_overlay_enabled: bool,
    pub(crate) fft_overlay_enabled: bool,
    pub(crate) lyrics_files: Vec<PathBuf>,
    pub(crate) selected_lyrics_index: Option<usize>,
    pub(crate) loaded_lyrics: Option<Lyrics>,
    pub(crate) lyrics_offset_ms: i64,

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

    pub(crate) taps: usize,

    pub(crate) side_panel_expanded: bool,
    pub(crate) theme: Theme,
    pub(crate) log_level: LogLevel,
    pub(crate) language: Language,

    pub(crate) last_scan_status: Option<String>,

    album_meta_cache: Option<MetaCache>,
    album_artwork_path_cache: HashMap<String, Option<PathBuf>>,
    album_artwork_texture_cache: HashMap<String, egui::TextureHandle>,
    album_artwork_pending: VecDeque<String>,
    album_artwork_queued: HashSet<String>,
    album_artwork_album_titles: Vec<String>,

    pub(crate) track_details_dialog: Option<TrackDetailsDialog>,
}

#[derive(Debug, Clone)]
pub(crate) struct TrackDetailsDialog {
    pub(crate) path: String,
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) album: String,
}

impl App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.active_tab == Tab::Scan
            || self.last_scan_status.as_deref() == Some("Requested scan")
        {
            self.refresh_scan_status();
        }

        self.poll_playback_status();
        self.monitor_default_output_device_hotplug();

        if matches!(
            self.playback_state,
            PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking
        ) || self.pending_seek_ms.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(16));
        }

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
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
                            egui::SelectableLabel::new(self.active_tab == tab, tab.label()),
                        )
                        .clicked()
                    {
                        self.active_tab = tab;
                        if tab == Tab::Queue {
                            self.queue_scroll_to_playing = true;
                        }
                        self.update_app_vm();
                    }
                }
            });
        });

        egui::SidePanel::left("side_panel")
            .resizable(false)
            .min_width(if self.side_panel_expanded { 240.0 } else { 32.0 })
            .default_width(if self.side_panel_expanded { 240.0 } else { 32.0 })
            .show(ctx, |ui| {
                output::ui(ui, self);
            });

        egui::CentralPanel::default().show(ctx, |ui| match self.active_tab {
            Tab::Library => tabs::library_tab::ui(ui, self),
            Tab::Play => tabs::play_tab::ui(ui, self),
            Tab::Playlist => tabs::playlist_tab::ui(ui, self),
            Tab::Album => tabs::album_tab::ui(ui, self),
            Tab::Artist => tabs::artist_tab::ui(ui, self),
            Tab::Queue => tabs::queue_tab::ui(ui, self),
            Tab::Scan => tabs::scan_tab::ui(ui, self),
            Tab::Settings => tabs::settings_tab::ui(ui, self),
        });
    }
}
