//! アプリケーション状態の永続化と構築を担当
//!
//! - State persistence goes through a debounced background [`StateSaver`]: hot paths only set
//!   a dirty flag ([`GuiApp::save_state`]); the frame loop hands a snapshot to the saver at
//!   most once per second, and `on_exit` flushes synchronously.
//! - Construction of the app (`new` / `with_options`): fonts (built on a worker), theme,
//!   audio engine, shared DB handle, library index.
//! - Window geometry (restored by `apps/gui` through [`initial_window`]).

use std::path::PathBuf;
use std::time::Instant;

use eframe::{egui, CreationContext};
use lrc_player_application::config::{Config, Theme};
use lrc_player_application::dto::{legacy_playback_mode, PlaylistDto, StateDto, WindowStateDto};
use lrc_player_application::infrastructure::audio::engine::AudioEngine;
use lrc_player_application::infrastructure::persistence::{
    MetaCache, SharedMetaCache, StateFile, StateSaver,
};
use lrc_player_application::usecases::queue_navigation::QueueNavigator;
use lrc_player_domain::model::PlaybackState;

use crate::presentation::app::playback::{create_engine, eq_from_dto, eq_to_dto};
use crate::presentation::app::{
    library_index::LibraryIndex, loudness, media_controls::MediaControlsBridge, playlists, scan_status,
    spectrum::FftAnalyzer, track_info, ArtworkLoader, GuiApp, UiShell,
};
use crate::presentation::ui::views::album_grid::{ALBUM_CARD_DEFAULT, ALBUM_CARD_MAX, ALBUM_CARD_MIN};
use crate::presentation::ui::{
    shell::SidebarState, views::{Column, NowPlayingState}, widgets::Toasts, Accent, Page, RightPanel, ThemeMode,
};

/// How a [`GuiApp`] is built. [`AppOptions::default`] is the real application.
#[derive(Debug, Clone)]
pub struct AppOptions {
    /// Read and write `state.json` / `config.json` / `playlists.json` next to the executable.
    pub persist: bool,
    /// Start the audio engine.
    pub engine: bool,
    /// Library database (default: the shared database next to the executable).
    pub meta_path: Option<PathBuf>,
    /// Use this state instead of loading `state.json`.
    pub state: Option<StateDto>,
    /// Use this config instead of loading `config.json`.
    pub config: Option<Config>,
    /// Use these playlists instead of loading `playlists.json`.
    pub playlists: Option<Vec<PlaylistDto>>,
    /// Build system fonts on a worker thread.
    pub load_fonts: bool,
    /// Register OS media controls (media keys / overlay).
    pub media_controls: bool,
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            persist: true,
            engine: true,
            meta_path: None,
            state: None,
            config: None,
            playlists: None,
            load_fonts: true,
            media_controls: true,
        }
    }
}

pub(crate) fn theme_mode(theme: Theme) -> ThemeMode {
    match theme {
        Theme::System => ThemeMode::System,
        Theme::Light => ThemeMode::Light,
        Theme::Dark => ThemeMode::Dark,
    }
}

pub(crate) fn config_theme(mode: ThemeMode) -> Theme {
    match mode {
        ThemeMode::System => Theme::System,
        ThemeMode::Light => Theme::Light,
        ThemeMode::Dark => Theme::Dark,
    }
}

fn page_key(page: Page) -> &'static str {
    match page {
        Page::NowPlaying => "now_playing",
        Page::Library => "library",
        Page::Albums => "albums",
        Page::Artists => "artists",
        Page::Playlists => "playlists",
        Page::Folders => "folders",
        Page::Queue => "queue",
        Page::Settings => "settings",
    }
}

/// Page from the persisted `last_tab` (also understands the old tab labels).
pub(crate) fn page_from_key(key: &str) -> Option<Page> {
    Some(match key {
        "now_playing" | "Play" => Page::NowPlaying,
        "library" | "Library" | "Queue" => Page::Library,
        "albums" | "Albums" => Page::Albums,
        "artists" | "Artists" => Page::Artists,
        "playlists" | "Playlists" => Page::Playlists,
        "folders" | "Scan" => Page::Folders,
        "queue" => Page::Queue,
        "settings" | "Settings" => Page::Settings,
        _ => return None,
    })
}

/// Optional library columns and their persisted keys.
fn column_key(c: Column) -> Option<&'static str> {
    match c {
        Column::Year => Some("year"),
        Column::Genre => Some("genre"),
        _ => None,
    }
}

fn column_from_key(key: &str) -> Option<Column> {
    match key {
        "year" => Some(Column::Year),
        "genre" => Some(Column::Genre),
        _ => None,
    }
}

fn right_panel_key(panel: Option<RightPanel>) -> Option<String> {
    panel.map(|p| match p {
        RightPanel::Queue => "queue".to_string(),
        RightPanel::Lyrics => "lyrics".to_string(),
    })
}

fn right_panel_from_key(key: Option<&str>) -> Option<RightPanel> {
    match key? {
        "queue" => Some(RightPanel::Queue),
        "lyrics" => Some(RightPanel::Lyrics),
        _ => None,
    }
}

/// Saved window geometry for the native options (`apps/gui`): `None` on first start.
pub fn initial_window() -> Option<WindowStateDto> {
    let state = StateFile::open().and_then(|f| f.load()).ok()?;
    let [min_w, min_h] = crate::presentation::ui::shell::MIN_WINDOW_SIZE;
    state.window.filter(|w| w.width >= 200.0 && w.height >= 150.0).map(|mut w| {
        // Never restore below the minimum size (older versions allowed smaller windows).
        w.width = w.width.max(min_w);
        w.height = w.height.max(min_h);
        w
    })
}

impl GuiApp {
    /// The real application (persistent state, audio engine, system fonts).
    pub fn new(cc: &CreationContext<'_>) -> Self {
        super::startup_cloak::cloak(cc);
        let mut app = Self::with_options(&cc.egui_ctx, AppOptions::default());
        // The binary does not create the window maximized on Windows (winit would show it
        // before it is cloaked); maximize it now, while it is still hidden.
        if cfg!(windows) && app.ui.window.as_ref().is_some_and(|w| w.maximized) {
            cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            super::startup_cloak::expect_maximized();
        }
        app.wait_for_fonts(&cc.egui_ctx);
        app
    }

    /// Installs the system fonts before the first frame. eframe keeps the window hidden until
    /// that frame is painted, so waiting here avoids a first frame drawn with the built-in
    /// fonts (no Japanese glyphs) that is then redrawn: a visible flash at startup. The fonts
    /// were being built on the worker while the rest of the app started; if that takes
    /// unusually long, the window opens anyway and they are installed when ready.
    fn wait_for_fonts(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.ui.fonts_rx else { return };
        let started = std::time::Instant::now();
        match rx.recv_timeout(std::time::Duration::from_secs(3)) {
            Ok(defs) => {
                crate::presentation::ui::fonts::install_definitions(ctx, defs);
                self.ui.fonts_rx = None;
                tracing::debug!("fonts ready after {:?} at startup", started.elapsed());
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => self.ui.fonts_rx = None,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                tracing::warn!("fonts not ready after 3 s; opening the window without them");
            }
        }
    }

    /// Builds the app with explicit options (tests and screenshots use temp data).
    pub fn with_options(ctx: &egui::Context, opts: AppOptions) -> Self {
        let ctx = ctx.clone();
        let cfg = match opts.config.clone() {
            Some(cfg) => cfg,
            None if opts.persist => Config::load(),
            None => Config::default(),
        };
        let state = match opts.state.clone() {
            Some(state) => state,
            None if opts.persist => StateFile::open().and_then(|s| s.load()).unwrap_or_default(),
            None => StateDto::default(),
        };
        let theme = theme_mode(cfg.theme);
        let accent = Accent::from_key(&cfg.accent).unwrap_or_default();
        crate::presentation::ui::theme::apply(&ctx, theme, accent);

        let fonts_rx = opts.load_fonts.then(|| {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let wake = ctx.clone();
            let _ = std::thread::Builder::new().name("font-loader".into()).spawn(move || {
                let extra: Vec<PathBuf> = if cfg!(target_os = "linux") {
                    vec![PathBuf::from("/mnt/c/Windows/Fonts")]
                } else {
                    Vec::new()
                };
                let dirs: Vec<&std::path::Path> = extra.iter().map(PathBuf::as_path).collect();
                let (defs, report) = crate::presentation::ui::fonts::build_definitions(&dirs);
                tracing::info!("fonts: {report:?}");
                let _ = tx.send(defs);
                wake.request_repaint();
            });
            rx
        });

        let playlists = match opts.playlists.clone() {
            Some(p) => playlists::normalize_playlists(p),
            None if opts.persist => playlists::load_playlists(),
            None => playlists::normalize_playlists(Vec::new()),
        };
        let queue = state.queue_tracks.clone();
        let selected_queue_index = state.selected_queue_index.filter(|&i| i < queue.len());
        let (shuffle, repeat) = state.shuffle_repeat();
        let mut navigator = QueueNavigator::new(false, repeat);
        navigator.set_shuffle(shuffle, selected_queue_index, queue.len());

        let volume = state.volume.clamp(0.0, 1.0);
        let eq = eq_from_dto(&state.eq);
        let peq = super::audio_quality::peq_from_config(&cfg.parametric_eq);
        let engine = if opts.engine { create_engine(&ctx, &cfg, volume, state.muted, eq) } else { None };
        let device_names = if opts.engine { AudioEngine::output_device_names() } else { Vec::new() };

        let meta = match &opts.meta_path {
            Some(path) => SharedMetaCache::open(path),
            None => MetaCache::shared(),
        };
        let meta = match meta {
            Ok(meta) => Some(meta),
            Err(err) => {
                tracing::warn!("opening the library database failed: {err:#}");
                None
            }
        };
        let state_saver = if opts.persist {
            match StateSaver::open_default() {
                Ok(saver) => Some(saver),
                Err(err) => {
                    tracing::warn!("state saver unavailable, saving synchronously: {err:#}");
                    None
                }
            }
        } else {
            None
        };
        scan_status::set_repaint_context(ctx.clone());

        let play_path = state.last_play_path.clone().unwrap_or_default();
        let lyrics_offset_ms = state.lyrics_offsets.get(&play_path).copied().unwrap_or(0);
        let lyrics_shift = state.lyrics_shifts.get(&play_path).copied().unwrap_or_default();
        let page = state.last_tab.as_deref().and_then(page_from_key).unwrap_or(Page::Library);
        let mut library = LibraryIndex::new(ctx.clone(), opts.persist);
        if let Some(meta) = &meta {
            library.reload(meta.path());
        }

        let artwork = ArtworkLoader::new(ctx.clone(), meta.as_ref().map(|m| m.path().to_path_buf()));
        let mut gui = GuiApp {
            engine,
            engine_snapshot: None,
            device_names,
            no_output_device: false,
            preferred_output_device: cfg.preferred_output_device.clone(),
            active_output_device: None,

            play_path,
            playback_state: PlaybackState::Stopped,
            navigator,
            current_queue_index: selected_queue_index,
            pending_next: None,
            consecutive_errors: 0,
            pending_error_skip: false,
            volume,
            muted: state.muted,
            eq,
            peq,
            exclusive_unavailable: None,
            config_save_due: None,

            pending_seek_ms: None,
            seek_last_change_at: None,
            seek_display_hold: None,

            play_position_ms: state.last_play_position_ms.unwrap_or(0),
            play_duration_ms: None,
            play_title: None,
            play_artist: None,
            play_album: None,
            track_info: track_info::TrackInfoLoader::new(ctx.clone()),
            loudness: loudness::LoudnessAnalyzer::new(ctx.clone(), opts.persist.then(loudness::default_cache_path).flatten()),

            play_artwork_handle: None,
            play_backdrop_handle: None,
            play_artwork_requested_for: None,
            play_cover_path: None,

            lyrics_candidates: Vec::new(),
            selected_lyrics_index: None,
            loaded_timeline: None,
            lyrics_revision: 1,
            lyrics_offset_ms,
            lyrics_discovered_for: String::new(),
            lyrics_shift,
            lyrics_shifts: state.lyrics_shifts.clone(),
            lyrics_offsets: state.lyrics_offsets.clone(),
            lyrics_overrides: state.lyrics_overrides.clone(),
            spectrum: FftAnalyzer::default(),

            scan_targets: state.scan_targets.clone(),
            scan_lrc_targets: state.scan_lrc_targets.clone(),
            scan_excludes: state.scan_excludes.clone(),
            last_scan_status: state.last_scan_status.clone(),
            scan_counts: (state.scan_scanned, state.scan_parsed, state.scan_registered),
            scanning: false,
            favorite_directories: state.favorite_directories.clone(),
            artist_view_mode: state.artist_view_mode,

            favorites: playlists::favorite_set(&playlists),
            playlists,
            queue,
            queue_rows: Default::default(),

            meta,
            library,
            artwork,

            ui: UiShell {
                page,
                album_detail: None,
                artist_detail: None,
                playlist_detail: None,
                right_panel: right_panel_from_key(state.right_panel.as_deref()),
                sidebar: SidebarState { collapsed: state.sidebar_collapsed },
                eq_open: false,
                eq_tab: Default::default(),
                eq_curve: None,
                signal_open: false,
                scroll_to_sound: 0,
                autoeq_picker: None,
                toasts: Toasts::new(),
                library_table: Default::default(),
                library_query: String::new(),
                library_columns: state.library_columns.iter().filter_map(|k| column_from_key(k)).collect(),
                history: Vec::new(),
                album_card_size: state.album_card_size.filter(|s| s.is_finite()).unwrap_or(ALBUM_CARD_DEFAULT).clamp(ALBUM_CARD_MIN, ALBUM_CARD_MAX),
                detail_table: Default::default(),
                detail_query: String::new(),
                artist_query: String::new(),
                focus_search: false,
                now: NowPlayingState::default(),
                side_lyrics: Default::default(),
                theme,
                accent,
                modal: None,
                details: None,
                folder_picker: None,
                fonts_rx,
                drop_rx: None,
                align_job: None,
                align_check: None,
                fetch_job: None,
                fetch_tried: Default::default(),
                exclude_input: String::new(),
                last_toast: None,
                window: state.window,
                detail_rows: Default::default(),
            },
            media: MediaControlsBridge::new(opts.media_controls),
            config: cfg,

            persist: opts.persist,
            state_saver,
            state_dirty: false,
            state_last_submit: None,
        };

        if !gui.play_path.is_empty() {
            // Duration / tags / lyrics for the restored track, off the UI thread.
            gui.request_track_info();
        }
        gui
    }

    /// Builds the persisted state from the in-memory app state.
    pub(crate) fn build_state_dto(&self) -> StateDto {
        let shuffle = self.navigator.shuffle();
        let repeat = self.navigator.repeat();
        StateDto {
            last_tab: Some(page_key(self.ui.page).to_string()),
            last_play_path: (!self.play_path.is_empty()).then(|| self.play_path.clone()),
            last_play_position_ms: Some(self.play_position_ms),
            last_scan_status: self.last_scan_status.clone(),
            scan_scanned: self.scan_counts.0,
            scan_parsed: self.scan_counts.1,
            scan_registered: self.scan_counts.2,
            scan_targets: self.scan_targets.clone(),
            scan_lrc_targets: self.scan_lrc_targets.clone(),
            scan_excludes: self.scan_excludes.clone(),
            queue_tracks: self.queue.clone(),
            selected_queue_index: self.current_queue_index,
            queue_playback_mode: legacy_playback_mode(shuffle, repeat),
            artist_view_mode: self.artist_view_mode,
            lrc_overlay_enabled: true,
            fft_overlay_enabled: self.config.show_spectrum,
            side_panel_expanded: !self.ui.sidebar.collapsed,
            favorite_directories: self.favorite_directories.clone(),
            volume: self.volume,
            muted: self.muted,
            eq: eq_to_dto(&self.eq),
            shuffle: Some(shuffle),
            repeat: Some(repeat),
            lyrics_offsets: self.lyrics_offsets.clone(),
            lyrics_shifts: self.lyrics_shifts.clone(),
            lyrics_overrides: self.lyrics_overrides.clone(),
            right_panel: right_panel_key(self.ui.right_panel),
            sidebar_collapsed: self.ui.sidebar.collapsed,
            window: self.ui.window,
            library_columns: self.ui.library_columns.iter().filter_map(|c| column_key(*c)).map(str::to_string).collect(),
            album_card_size: Some(self.ui.album_card_size),
        }
    }

    /// Marks the state as changed. Cheap: the frame loop hands it to the background saver
    /// (at most once per second); nothing is serialized or written here.
    pub(crate) fn save_state(&mut self) {
        self.state_dirty = true;
    }

    /// Serializes the state now and queues it on the background saver (no disk I/O here).
    pub(crate) fn submit_state_now(&mut self) {
        self.state_dirty = false;
        self.state_last_submit = Some(Instant::now());
        if !self.persist {
            return;
        }
        let state = self.build_state_dto();
        match &self.state_saver {
            Some(saver) => {
                if let Err(err) = saver.save(&state) {
                    tracing::warn!("state save failed: {err:#}");
                }
            }
            None => {
                if let Ok(store) = StateFile::open() {
                    let _ = store.save(&state);
                }
            }
        }
    }

    /// Submits the current state and blocks until it is on disk (exit path).
    pub(crate) fn flush_state(&mut self) {
        self.submit_state_now();
        if let Some(saver) = &self.state_saver {
            if let Err(err) = saver.flush() {
                tracing::warn!("state flush failed: {err:#}");
            }
        }
    }

    /// Writes `config.json` (no-op without persistence).
    pub(crate) fn save_config(&mut self) {
        if !self.persist {
            return;
        }
        if let Err(err) = self.config.save() {
            tracing::warn!("saving config failed: {err:#}");
            let tr = self.tr();
            self.toast_error(tr.f("toast.config_save_failed", &[&format!("{err:#}")]));
        }
    }

    /// Installs the fonts built on the worker (once).
    pub(crate) fn poll_fonts(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.ui.fonts_rx else { return };
        match rx.try_recv() {
            Ok(defs) => {
                crate::presentation::ui::fonts::install_definitions(ctx, defs);
                self.ui.fonts_rx = None;
                // The bold family becomes active next pass; `theme::sync` re-applies then.
                ctx.request_repaint();
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => self.ui.fonts_rx = None,
            Err(crossbeam_channel::TryRecvError::Empty) => {}
        }
    }

    /// Remembers the window geometry (persisted with the state when it changes).
    pub(crate) fn track_window_geometry(&mut self, ctx: &egui::Context) {
        let (inner, outer, maximized, minimized) = ctx.input(|i| {
            let vp = i.viewport();
            (vp.inner_rect, vp.outer_rect, vp.maximized.unwrap_or(false), vp.minimized.unwrap_or(false))
        });
        if minimized {
            return;
        }
        // Wayland reports no window position, so `inner_rect` is unknown there; the size
        // still comes from the content area.
        let inner = inner.unwrap_or_else(|| ctx.content_rect());
        if inner.width() < 50.0 || inner.height() < 50.0 {
            return;
        }
        let prev = self.ui.window;
        let mut next = WindowStateDto {
            width: inner.width().round(),
            height: inner.height().round(),
            x: outer.map(|r| r.min.x.round()),
            y: outer.map(|r| r.min.y.round()),
            maximized,
        };
        if maximized {
            // Keep the restored (non-maximized) size/position for the next start.
            if let Some(p) = prev {
                next.width = p.width;
                next.height = p.height;
                next.x = p.x;
                next.y = p.y;
            }
        }
        if prev != Some(next) {
            self.ui.window = Some(next);
            self.save_state();
        }
    }
}
