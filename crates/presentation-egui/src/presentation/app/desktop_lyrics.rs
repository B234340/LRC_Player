//! Desktop lyrics: a second native window (egui viewport) that floats above other apps and
//! shows the current lyric line with the karaoke fill, the next line and hover controls
//! (`ui/views/desktop_lyrics.rs` draws it).
//!
//! # Viewport type
//!
//! A **deferred** viewport (`Context::show_viewport_deferred`). An immediate viewport is drawn
//! inside its parent's frame, so every overlay frame would also run the whole main window UI,
//! and the overlay could only repaint as often as the main window does (≈1 Hz while minimized,
//! see `REPAINT_BACKGROUND_PLAYING`). A deferred viewport has its own repaint schedule: eframe
//! calls its callback whenever *it* needs a frame, while the main window sleeps.
//!
//! The viewport is (re-)registered from `GuiApp::after_frame`, which runs in `App::logic` —
//! and eframe runs `logic` for every main-window pass even while the main window is minimized
//! or hidden (only `App::ui` is skipped then). egui removes a child viewport that its parent
//! did not show during a pass, so registering it from `ui` would close the overlay whenever
//! the main window is minimized. (With the overlay visible, eframe also runs `App::ui` for a
//! minimized main window — `is_viewport_or_descendant_visible` — without painting it.)
//!
//! # Data flow
//!
//! The callback must be `Send + Sync + 'static`, so it reads a snapshot ([`Shared`]) that the
//! main window refreshes on each of its passes: title, the lyrics timeline (`Arc`, with the
//! user offset and text shift already applied, exactly what the main lyrics view uses),
//! settings, and the last playback position with the time it was taken. Between main-window
//! passes the overlay extrapolates the position with the wall clock, so it animates at the
//! display rate while the main window ticks slowly (or is minimized). Clicks are queued in
//! [`Shared::actions`] and the main window is woken to apply them.
//!
//! Repaints: at the display rate only while playing a karaoke line; otherwise at the next
//! line change (capped at 250 ms); nothing while paused or hidden (eframe does not run the
//! callback of a minimized / occluded viewport).
//!
//! # Lock (click-through) and how to unlock
//!
//! Locking sets `ViewportCommand::MousePassthrough` (via the viewport builder): the window
//! ignores the mouse entirely, so it cannot unlock itself. Unlock from the main window:
//! - the transport bar's desktop lyrics button: while locked a click unlocks (it shows a lock
//!   badge); its right-click menu has show / lock / settings;
//! - Settings → 歌詞 → デスクトップ歌詞 → ロック;
//! - Ctrl+Alt+L (lock / unlock) and Ctrl+Shift+L (show / hide) while the main window or the
//!   overlay has keyboard focus.
//!
//! # Platform notes
//!
//! - Windows: the window is created hidden and shown after its first two frames (no white
//!   flash), without taking focus, without a taskbar button (`with_taskbar(false)` =
//!   `WS_EX_TOOLWINDOW`). The main window's WndProc subclass (`window_scale`) and DWM cloak
//!   (`startup_cloak`) use the root window handle only (`eframe::Frame` / `CreationContext`),
//!   so they never touch this window. winit implements click-through with
//!   `WS_EX_TRANSPARENT | WS_EX_LAYERED`; a layered window stays invisible until
//!   `SetLayeredWindowAttributes` is called, which [`win::keep_layered_visible`] does.
//! - Transparency needs an alpha-capable GL config: Windows (WGL, alpha 8 by default) and
//!   Wayland have it; macOS / X11 need the root viewport to request transparency (see
//!   `apps/gui`). The overlay draws everything itself, so an opaque fallback only loses the
//!   see-through corners.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use eframe::egui::{self, pos2, vec2, Pos2, Rect, Vec2, ViewportBuilder, ViewportClass, ViewportCommand, ViewportId};
use lrc_player_application::config::{DesktopLyricsSetting, DESKTOP_LYRICS_FONT_RANGE};
use lrc_player_application::dto::WindowStateDto;
use lrc_player_application::infrastructure::lyrics::LyricsTimeline;
use lrc_player_domain::model::PlaybackState;

use crate::presentation::app::GuiApp;
use crate::presentation::ui::{
    text::{self, TextLang},
    views::desktop_lyrics::{
        desktop_lyrics_view, DesktopLyricsAction, DesktopLyricsContent, DesktopLyricsProps, DesktopLyricsViewState, OverlayLine,
    },
    Accent, ThemeMode, Tr, UiAction,
};

/// Title of the native window (not localized: Windows finds the window by it).
const WINDOW_TITLE: &str = "LRC Player — Desktop Lyrics";
pub(crate) const DEFAULT_SIZE: Vec2 = Vec2::new(640.0, 144.0);
pub(crate) const MIN_SIZE: Vec2 = Vec2::new(260.0, 72.0);
/// Font size step of the −/+ buttons.
const FONT_STEP: f32 = 2.0;
/// A fresh position sample this far behind the extrapolated one is held (no jitter).
const BACKSTEP_HOLD_MS: u64 = 250;
/// Longest wait between repaints while playing without a karaoke line.
const MAX_IDLE: Duration = Duration::from_millis(250);
/// A position sample that differs from the extrapolation by more than this is a jump (seek).
const JUMP_MS: u64 = 400;

pub(crate) fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("lrc_player_desktop_lyrics")
}

// ---------------------------------------------------------------------------------------
// Pure helpers (unit tested)
// ---------------------------------------------------------------------------------------

/// Playback position as last reported by the main window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PositionSample {
    pub position_ms: u64,
    pub at: Instant,
    /// Playing (the position advances with the wall clock).
    pub advancing: bool,
    pub duration_ms: Option<u64>,
}

impl PositionSample {
    pub(crate) fn at_time(&self, now: Instant) -> u64 {
        if !self.advancing {
            return self.position_ms;
        }
        let p = self.position_ms.saturating_add(now.saturating_duration_since(self.at).as_millis() as u64);
        match self.duration_ms {
            Some(d) if d > 0 => p.min(d),
            _ => p,
        }
    }
}

/// Position to display: a small step back (a fresh sample slightly behind the extrapolation)
/// holds the previous value, so the karaoke wipe never jitters backwards; real jumps pass.
pub(crate) fn smooth_position(prev_shown: Option<u64>, target: u64) -> u64 {
    match prev_shown {
        Some(prev) if target < prev && prev - target <= BACKSTEP_HOLD_MS => prev,
        _ => target,
    }
}

/// Lines the overlay shows at a position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OverlayLines {
    /// Current (non-blank) line; `None` before the first line or during an interlude.
    pub current: Option<usize>,
    pub highlighted_chars: f32,
    /// Next non-blank line.
    pub next: Option<usize>,
    /// Before the first line.
    pub intro: bool,
}

/// Same timing as the main lyrics view (`LyricsTimeline::position_at`, offsets included).
pub(crate) fn overlay_lines(timeline: &LyricsTimeline, position_ms: u64) -> OverlayLines {
    let lines = timeline.lines();
    let next_after = |from: usize| (from..lines.len()).find(|&i| !lines[i].is_blank());
    match timeline.position_at(position_ms) {
        None => OverlayLines { current: None, highlighted_chars: 0.0, next: next_after(0), intro: true },
        Some(p) if lines[p.line_index].is_blank() => {
            OverlayLines { current: None, highlighted_chars: 0.0, next: next_after(p.line_index + 1), intro: false }
        }
        Some(p) => OverlayLines {
            current: Some(p.line_index),
            highlighted_chars: p.highlighted_chars,
            next: next_after(p.line_index + 1),
            intro: false,
        },
    }
}

/// Milliseconds of playback until the next line starts (for the repaint schedule).
pub(crate) fn ms_to_next_line(timeline: &LyricsTimeline, position_ms: u64) -> Option<u64> {
    let t = timeline.lyric_time_ms(position_ms);
    timeline.lines().iter().find(|l| l.start_ms as i64 > t).map(|l| (l.start_ms as i64 - t) as u64)
}

/// When the overlay should paint again: `Some(ZERO)` = every display frame.
pub(crate) fn next_repaint(playing: bool, karaoke_line: bool, to_next_line: Option<u64>) -> Option<Duration> {
    if !playing {
        return None;
    }
    if karaoke_line {
        return Some(Duration::ZERO);
    }
    Some(to_next_line.map_or(MAX_IDLE, |ms| Duration::from_millis(ms + 5).min(MAX_IDLE)))
}

/// Lyrics that only say "instrumental" (LRCLIB, NetEase-style placeholders).
pub(crate) fn is_instrumental(timeline: &LyricsTimeline) -> bool {
    let texts: Vec<&str> = timeline.lines().iter().filter(|l| !l.is_blank()).map(|l| l.text.as_str()).collect();
    !texts.is_empty() && texts.len() <= 2 && texts.iter().all(|t| is_instrumental_marker(t))
}

fn is_instrumental_marker(text: &str) -> bool {
    let s = text.trim().trim_matches(|c: char| "()[]（）【】「」『』♪~〜 *-".contains(c)).to_lowercase();
    s == "instrumental" || s == "インスト" || s.contains("インストゥルメンタル") || s.contains("纯音乐") || s.contains("純音樂") || s.contains("純音楽")
}

/// `rect` moved (and if needed shrunk) to lie inside `area`.
pub(crate) fn clamp_rect(rect: Rect, area: Rect) -> Rect {
    if !area.is_finite() || area.width() < 1.0 || area.height() < 1.0 {
        return rect;
    }
    let size = rect.size().max(MIN_SIZE).min(area.size());
    let x = rect.min.x.clamp(area.min.x, area.max.x - size.x);
    let y = rect.min.y.clamp(area.min.y, area.max.y - size.y);
    Rect::from_min_size(pos2(x, y), size)
}

/// Where the window opens: the saved geometry (clamped into `area`, the work area of the
/// monitor nearest to it when known), else centered above the bottom of the main window.
/// Returns (position, inner size); `None` lets the OS place the window.
pub(crate) fn initial_geometry(saved: Option<WindowStateDto>, area: Option<Rect>, main: Option<Rect>) -> (Option<Pos2>, Vec2) {
    let plausible = |v: f32| v.is_finite() && (-16_000.0..16_000.0).contains(&v);
    let size = saved
        .filter(|w| w.width.is_finite() && w.height.is_finite())
        .map(|w| vec2(w.width, w.height))
        .unwrap_or(DEFAULT_SIZE)
        .clamp(MIN_SIZE, vec2(4000.0, 1200.0));
    let pos = match saved.and_then(|w| Some(pos2(w.x.filter(|v| plausible(*v))?, w.y.filter(|v| plausible(*v))?))) {
        Some(p) => Some(p),
        None => main.map(|m| pos2(m.center().x - size.x / 2.0, (m.bottom() - size.y - 110.0).max(m.top()))),
    };
    match (pos, area) {
        (Some(p), Some(area)) => {
            let r = clamp_rect(Rect::from_min_size(p, size), area);
            (Some(r.min), r.size())
        }
        (p, _) => (p, size),
    }
}

fn window_state(r: Rect) -> WindowStateDto {
    WindowStateDto { width: r.width().round(), height: r.height().round(), x: Some(r.min.x.round()), y: Some(r.min.y.round()), maximized: false }
}

// ---------------------------------------------------------------------------------------
// State shared with the viewport callback
// ---------------------------------------------------------------------------------------

/// What the main window hands to the overlay.
#[derive(Clone)]
pub(crate) struct Feed {
    pub has_track: bool,
    pub title: String,
    pub artist: String,
    pub timeline: Option<Arc<LyricsTimeline>>,
    /// Identity of `timeline` (line keys for the fade-in).
    pub revision: u64,
    pub karaoke: bool,
    pub lang: TextLang,
    pub instrumental: bool,
    pub sample: PositionSample,
    /// Playing / buffering (play-pause icon).
    pub playing: bool,
    pub settings: DesktopLyricsSetting,
    pub tr: Tr,
}

impl Default for Feed {
    fn default() -> Self {
        Self {
            has_track: false,
            title: String::new(),
            artist: String::new(),
            timeline: None,
            revision: 0,
            karaoke: true,
            lang: TextLang::Latin,
            instrumental: false,
            sample: PositionSample { position_ms: 0, at: Instant::now(), advancing: false, duration_ms: None },
            playing: false,
            settings: DesktopLyricsSetting::default(),
            tr: Tr::default(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Shared {
    pub feed: Feed,
    /// Clicks waiting for the main window.
    pub actions: Vec<DesktopLyricsAction>,
    /// Outer position + inner size reported by the window (points).
    pub geometry: Option<Rect>,
    view: DesktopLyricsViewState,
    shown_pos: Option<u64>,
    /// Frames painted since the window was (re)created.
    frames: u32,
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a change of requires a repaint of the overlay.
#[derive(Clone, PartialEq)]
struct FeedKey {
    has_track: bool,
    title: String,
    artist: String,
    revision: u64,
    offset_ms: i64,
    karaoke: bool,
    playing: bool,
    advancing: bool,
    paused_at: Option<u64>,
    settings: DesktopLyricsSetting,
    lang: crate::presentation::ui::Lang,
    theme: (ThemeMode, Accent),
}

/// Desktop lyrics state owned by the main window (`UiShell`).
pub(crate) struct DesktopLyrics {
    shared: Arc<Mutex<Shared>>,
    /// Persisted geometry (state.json).
    window: Option<WindowStateDto>,
    /// Builder geometry while the viewport exists (kept fixed so egui never moves the window).
    open: Option<(Option<Pos2>, Vec2)>,
    key: Option<FeedKey>,
    /// (lyrics revision, total offset, karaoke) the shared timeline was built for.
    timeline_key: Option<(u64, i64, bool)>,
}

impl DesktopLyrics {
    pub(crate) fn new(window: Option<WindowStateDto>) -> Self {
        Self { shared: Arc::default(), window, open: None, key: None, timeline_key: None }
    }

    /// Geometry to persist (the window's latest report wins).
    pub(crate) fn window_for_save(&self) -> Option<WindowStateDto> {
        lock(&self.shared).geometry.map(window_state).or(self.window)
    }

    /// The window exists (tests).
    #[cfg(test)]
    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }
}

// ---------------------------------------------------------------------------------------
// Main window side
// ---------------------------------------------------------------------------------------

impl GuiApp {
    /// Show / hide the desktop lyrics window.
    pub(crate) fn toggle_desktop_lyrics(&mut self, ctx: &egui::Context) {
        self.config.desktop_lyrics.enabled = !self.config.desktop_lyrics.enabled;
        self.save_config();
        ctx.request_repaint();
    }

    pub(crate) fn set_desktop_lyrics_locked(&mut self, ctx: &egui::Context, locked: bool) {
        let s = &mut self.config.desktop_lyrics;
        s.locked = locked;
        if locked {
            s.enabled = true;
        }
        self.save_config();
        ctx.request_repaint();
    }

    /// Forgets the saved geometry and moves an open window back above the main window.
    pub(crate) fn reset_desktop_lyrics_position(&mut self, ctx: &egui::Context) {
        let dl = &mut self.ui.desktop_lyrics;
        dl.window = None;
        lock(&dl.shared).geometry = None;
        let (area, main) = monitor_hints(ctx, None);
        let (pos, size) = initial_geometry(None, area, main);
        if dl.open.is_some() {
            if let Some(p) = pos {
                ctx.send_viewport_cmd_to(viewport_id(), ViewportCommand::OuterPosition(p));
            }
            ctx.send_viewport_cmd_to(viewport_id(), ViewportCommand::InnerSize(size));
        }
        self.save_state();
    }

    /// Applies queued overlay clicks, refreshes the overlay's snapshot and keeps its viewport
    /// alive. Called from `after_frame` (so also while the main window is minimized).
    pub(crate) fn sync_desktop_lyrics(&mut self, ctx: &egui::Context) {
        self.desktop_lyrics_shortcuts(ctx);
        let queued = std::mem::take(&mut lock(&self.ui.desktop_lyrics.shared).actions);
        for action in queued {
            self.apply_desktop_lyrics_action(ctx, action);
        }

        // Persist geometry reported by the window.
        let reported = lock(&self.ui.desktop_lyrics.shared).geometry.map(window_state);
        if reported.is_some() && reported != self.ui.desktop_lyrics.window {
            self.ui.desktop_lyrics.window = reported;
            self.save_state();
        }

        let settings = self.config.desktop_lyrics.sanitized();
        if !settings.enabled {
            if self.ui.desktop_lyrics.open.take().is_some() {
                // Not shown this pass: egui closes the window at the end of the next one.
                self.ui.desktop_lyrics.key = None;
                ctx.request_repaint();
            }
            return;
        }

        self.update_desktop_lyrics_feed(ctx, settings);

        let dl = &mut self.ui.desktop_lyrics;
        if dl.open.is_none() {
            let (area, main) = monitor_hints(ctx, dl.window);
            dl.open = Some(initial_geometry(dl.window, area, main));
            let mut sh = lock(&dl.shared);
            sh.frames = 0;
            sh.view = DesktopLyricsViewState::default();
            sh.shown_pos = None;
        }
        let (pos, size) = dl.open.unwrap_or((None, DEFAULT_SIZE));
        let mut builder = ViewportBuilder::default()
            .with_title(WINDOW_TITLE)
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_resizable(true)
            .with_taskbar(false)
            .with_active(false)
            .with_has_shadow(false)
            .with_inner_size(size)
            .with_min_inner_size(MIN_SIZE)
            .with_mouse_passthrough(settings.locked)
            // Shown after its first frames on Windows (see the module docs).
            .with_visible(!cfg!(windows));
        if let Some(p) = pos {
            builder = builder.with_position(p);
        }
        let shared = dl.shared.clone();
        ctx.show_viewport_deferred(viewport_id(), builder, move |ui, class| overlay_ui(ui, class, &shared));
    }

    fn update_desktop_lyrics_feed(&mut self, ctx: &egui::Context, settings: DesktopLyricsSetting) {
        let karaoke = self.config.karaoke;
        let offset = self.loaded_timeline.as_ref().map_or(0, |t| t.total_offset_ms());
        let tkey = (self.lyrics_revision, offset, karaoke);
        let rebuild = self.ui.desktop_lyrics.timeline_key != Some(tkey);
        let timeline = rebuild.then(|| self.loaded_timeline.clone().filter(|t| !t.is_empty()).map(Arc::new));
        let now = Instant::now();
        let sample = PositionSample {
            position_ms: self.play_position_ms,
            at: now,
            advancing: self.playback_state == PlaybackState::Playing,
            duration_ms: self.play_duration_ms,
        };
        let key = FeedKey {
            has_track: !self.play_path.is_empty(),
            title: self.current_title(),
            artist: self.play_artist.clone().unwrap_or_default(),
            revision: self.lyrics_revision,
            offset_ms: offset,
            karaoke,
            playing: self.is_playing(),
            advancing: sample.advancing,
            paused_at: (!sample.advancing).then_some(sample.position_ms),
            settings,
            lang: self.tr().lang,
            theme: (self.ui.theme, self.ui.accent),
        };
        let tr = self.tr();
        let dl = &mut self.ui.desktop_lyrics;
        let mut sh = lock(&dl.shared);
        let feed = &mut sh.feed;
        // Keep the older sample while the engine reports the same position: its time stamp is
        // the accurate one (this runs after `logic` and again after `ui`).
        let old = feed.sample;
        let same = old.advancing == sample.advancing && old.position_ms == sample.position_ms && old.duration_ms == sample.duration_ms;
        let jump = old.at_time(now).abs_diff(sample.position_ms) > JUMP_MS;
        if !same {
            feed.sample = sample;
        }
        if let Some(t) = timeline {
            feed.lang = t.as_ref().map_or(TextLang::Latin, |t| text::dominant_lang(t.lines().iter().map(|l| l.text.as_str())));
            feed.instrumental = t.as_deref().is_some_and(is_instrumental);
            feed.timeline = t;
            dl.timeline_key = Some(tkey);
        }
        feed.has_track = key.has_track;
        feed.title.clone_from(&key.title);
        feed.artist.clone_from(&key.artist);
        feed.revision = self.lyrics_revision;
        feed.karaoke = karaoke;
        feed.playing = key.playing;
        feed.settings = settings;
        feed.tr = tr;
        if jump || rebuild {
            sh.shown_pos = None;
        }
        drop(sh);
        if jump || dl.key.as_ref() != Some(&key) {
            dl.key = Some(key);
            ctx.request_repaint_of(viewport_id());
        }
    }

    fn apply_desktop_lyrics_action(&mut self, ctx: &egui::Context, action: DesktopLyricsAction) {
        let s = &mut self.config.desktop_lyrics;
        match action {
            DesktopLyricsAction::Previous => self.dispatch(ctx, UiAction::Previous),
            DesktopLyricsAction::PlayPause => self.dispatch(ctx, UiAction::PlayPause),
            DesktopLyricsAction::Next => self.dispatch(ctx, UiAction::Next),
            DesktopLyricsAction::FontSmaller | DesktopLyricsAction::FontLarger => {
                let step = if action == DesktopLyricsAction::FontLarger { FONT_STEP } else { -FONT_STEP };
                s.font_size = (s.font_size + step).clamp(*DESKTOP_LYRICS_FONT_RANGE.start(), *DESKTOP_LYRICS_FONT_RANGE.end());
                self.save_config_soon();
            }
            DesktopLyricsAction::Lock => {
                let locked = !s.locked;
                self.set_desktop_lyrics_locked(ctx, locked);
            }
            DesktopLyricsAction::Close => {
                s.enabled = false;
                self.save_config();
            }
            DesktopLyricsAction::OpenMain => {
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
            // Handled by the window itself.
            DesktopLyricsAction::StartDrag | DesktopLyricsAction::BeginResize(_) => {}
        }
    }

    /// Ctrl+Shift+L (show / hide) and Ctrl+Alt+L (lock / unlock) in the main window. Runs
    /// before the global shortcuts, which would read Ctrl+Shift+L as Ctrl+L (lyrics panel).
    fn desktop_lyrics_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (toggle, lock_key) = ctx.input_mut(consume_shortcuts);
        if toggle {
            self.toggle_desktop_lyrics(ctx);
        }
        if lock_key {
            let locked = !self.config.desktop_lyrics.locked;
            self.set_desktop_lyrics_locked(ctx, locked);
        }
    }
}

/// (show / hide, lock / unlock) pressed this pass.
fn consume_shortcuts(i: &mut egui::InputState) -> (bool, bool) {
    use egui::{Key, Modifiers};
    let toggle = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::L);
    let lock = i.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::L);
    (toggle, lock)
}

/// Work area of the monitor nearest to the saved window (Windows) and the main window's
/// rect, for placing the overlay.
fn monitor_hints(ctx: &egui::Context, saved: Option<WindowStateDto>) -> (Option<Rect>, Option<Rect>) {
    let (main, ppp) = ctx.input(|i| {
        let vp = i.viewport();
        (vp.outer_rect.filter(|_| !vp.minimized.unwrap_or(false)), vp.native_pixels_per_point.unwrap_or(1.0))
    });
    let probe = saved
        .and_then(|w| Some(Rect::from_min_size(pos2(w.x?, w.y?), vec2(w.width, w.height))))
        .or(main);
    (probe.and_then(|r| win::work_area(r, ppp)), main)
}

// ---------------------------------------------------------------------------------------
// Overlay side (runs in the overlay viewport's own passes)
// ---------------------------------------------------------------------------------------

fn overlay_ui(ui: &mut egui::Ui, class: ViewportClass, shared: &Arc<Mutex<Shared>>) {
    let ctx = ui.ctx().clone();
    let native = class != ViewportClass::EmbeddedWindow;
    let mut sh = lock(shared);
    let mut wake_main = false;

    if native && ctx.input(|i| i.viewport().close_requested()) {
        // Alt+F4 on the overlay: hide it (the setting is turned off by the main window).
        sh.actions.push(DesktopLyricsAction::Close);
        wake_main = true;
    }
    if !ctx.egui_wants_keyboard_input() {
        let (toggle, lock_key) = ctx.input_mut(consume_shortcuts);
        if toggle {
            sh.actions.push(DesktopLyricsAction::Close);
            wake_main = true;
        }
        if lock_key {
            sh.actions.push(DesktopLyricsAction::Lock);
            wake_main = true;
        }
    }

    let now = Instant::now();
    let feed = sh.feed.clone();
    let settings = feed.settings;
    let position = smooth_position(sh.shown_pos, feed.sample.at_time(now));
    sh.shown_pos = Some(position);

    let mut karaoke_line = false;
    let mut to_next = None;
    let content = match (&feed.timeline, feed.has_track) {
        (_, false) => DesktopLyricsContent::NoTrack,
        (None, true) => DesktopLyricsContent::NoLyrics,
        (Some(_), true) if feed.instrumental => DesktopLyricsContent::Instrumental,
        (Some(t), true) => {
            let l = overlay_lines(t, position);
            let lines = t.lines();
            let current = l.current.map(|i| OverlayLine {
                text: &lines[i].text,
                karaoke: feed.karaoke && lines[i].is_karaoke(),
                highlighted_chars: l.highlighted_chars,
            });
            karaoke_line = current.is_some_and(|c| c.karaoke && c.highlighted_chars < lines[l.current.unwrap_or(0)].char_count as f32);
            to_next = ms_to_next_line(t, position);
            let slot = match (l.current, l.intro) {
                (Some(i), _) => i as u64,
                (None, true) => u64::from(u32::MAX),
                (None, false) => u64::from(u32::MAX) - 1 - l.next.unwrap_or(0) as u64,
            };
            DesktopLyricsContent::Lyrics {
                current,
                intro: l.intro,
                next: l.next.map(|i| lines[i].text.as_str()),
                line_key: feed.revision.wrapping_shl(32) ^ slot,
                lang: feed.lang,
            }
        }
    };
    let props = DesktopLyricsProps {
        content,
        title: &feed.title,
        artist: &feed.artist,
        font_size: settings.font_size,
        opacity: settings.opacity,
        show_next_line: settings.show_next_line,
        show_title: settings.show_title,
        locked: settings.locked && native,
        playing: feed.playing,
        has_track: feed.has_track,
        force_controls: false,
        tr: feed.tr,
    };
    let mut view = std::mem::take(&mut sh.view);
    drop(sh);
    let actions = desktop_lyrics_view(ui, &props, &mut view);
    let mut sh = lock(shared);
    sh.view = view;
    for action in actions {
        match action {
            DesktopLyricsAction::StartDrag if native => ctx.send_viewport_cmd(ViewportCommand::StartDrag),
            DesktopLyricsAction::BeginResize(dir) if native => ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir)),
            DesktopLyricsAction::StartDrag | DesktopLyricsAction::BeginResize(_) => {}
            other => {
                sh.actions.push(other);
                wake_main = true;
            }
        }
    }

    if native {
        let (outer, inner) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
        if let (Some(outer), Some(inner)) = (outer, inner) {
            let geometry = Rect::from_min_size(outer.min, inner.size());
            if inner.width() >= MIN_SIZE.x - 1.0 && sh.geometry != Some(geometry) {
                sh.geometry = Some(geometry);
                // Persisted by the main window on its next pass (soon, not every drag frame).
                ctx.request_repaint_after_for(Duration::from_secs(1), ViewportId::ROOT);
            }
        }
        sh.frames = sh.frames.saturating_add(1);
        if cfg!(windows) && sh.frames == 2 {
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        }
        if sh.frames < 3 {
            ctx.request_repaint();
        }
        win::keep_layered_visible(WINDOW_TITLE, settings.locked);
    }
    drop(sh);
    if wake_main {
        ctx.request_repaint_of(ViewportId::ROOT);
    }
    match next_repaint(feed.playing && feed.sample.advancing, karaoke_line, to_next) {
        Some(d) if d.is_zero() => ctx.request_repaint(),
        Some(d) => ctx.request_repaint_after(d),
        None => {}
    }
}

// ---------------------------------------------------------------------------------------
// Windows helpers
// ---------------------------------------------------------------------------------------

#[cfg(windows)]
mod win {
    use std::sync::atomic::{AtomicBool, Ordering};

    use eframe::egui::{pos2, Rect};
    use winapi::shared::windef::{HWND, RECT};
    use winapi::um::winuser::{
        FindWindowW, GetMonitorInfoW, GetWindowLongPtrW, GetWindowThreadProcessId, MonitorFromRect, SetLayeredWindowAttributes,
        GWL_EXSTYLE, LWA_ALPHA, MONITORINFO, MONITOR_DEFAULTTONEAREST, WS_EX_LAYERED,
    };

    /// `SetLayeredWindowAttributes` was called since the window became layered.
    static LAYERED_FIXED: AtomicBool = AtomicBool::new(false);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Our overlay window (by title, owned by this process).
    fn overlay_hwnd(title: &str) -> Option<HWND> {
        let title = wide(title);
        // SAFETY: plain window lookup with a NUL-terminated title.
        let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0u32;
        // SAFETY: `hwnd` came from FindWindowW; `pid` is a valid out pointer.
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        (pid == std::process::id()).then_some(hwnd)
    }

    /// winit makes a click-through window `WS_EX_LAYERED`; such a window is not drawn until
    /// `SetLayeredWindowAttributes` has been called. Called every overlay frame (cheap once
    /// done): opaque layer alpha, so the per-pixel alpha of the GL surface still applies.
    pub(super) fn keep_layered_visible(title: &str, locked: bool) {
        if !locked {
            LAYERED_FIXED.store(false, Ordering::Relaxed);
            return;
        }
        if LAYERED_FIXED.load(Ordering::Relaxed) {
            return;
        }
        let Some(hwnd) = overlay_hwnd(title) else { return };
        // SAFETY: style query / layered attributes on our own live window.
        let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        if ex & WS_EX_LAYERED != 0 {
            unsafe { SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA) };
            LAYERED_FIXED.store(true, Ordering::Relaxed);
        }
    }

    /// Work area (taskbar excluded) of the monitor nearest to `near` (points), in points.
    pub(super) fn work_area(near: Rect, ppp: f32) -> Option<Rect> {
        let ppp = if ppp.is_finite() && ppp > 0.1 { ppp } else { 1.0 };
        let r = RECT {
            left: (near.min.x * ppp) as i32,
            top: (near.min.y * ppp) as i32,
            right: (near.max.x * ppp) as i32,
            bottom: (near.max.y * ppp) as i32,
        };
        // SAFETY: MonitorFromRect with DEFAULTTONEAREST always returns a monitor; the info
        // struct has its size set.
        unsafe {
            let mon = MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST);
            if mon.is_null() {
                return None;
            }
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(mon, &mut info) == 0 {
                return None;
            }
            let w = info.rcWork;
            Some(Rect::from_min_max(
                pos2(w.left as f32 / ppp, w.top as f32 / ppp),
                pos2(w.right as f32 / ppp, w.bottom as f32 / ppp),
            ))
        }
    }
}

#[cfg(not(windows))]
mod win {
    use eframe::egui::Rect;

    pub(super) fn keep_layered_visible(_title: &str, _locked: bool) {}

    /// Monitor positions are not available here; the saved position is only sanity-checked.
    pub(super) fn work_area(_near: Rect, _ppp: f32) -> Option<Rect> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LRC: &str = "[00:10.00]一行目\n\
[00:14.00]<00:14.00>二<00:15.00>行<00:16.00>目<00:17.00>\n\
[00:18.00]\n\
[00:22.00]四行目\n";

    #[test]
    fn line_selection_matches_the_timeline() {
        let t = LyricsTimeline::parse(LRC);
        let l = overlay_lines(&t, 5_000);
        assert_eq!((l.current, l.next, l.intro), (None, Some(0), true), "intro shows the first line next");
        let l = overlay_lines(&t, 11_000);
        assert_eq!((l.current, l.next), (Some(0), Some(1)));
        // Karaoke progress mid-line, same as the main view.
        let l = overlay_lines(&t, 15_500);
        assert_eq!(l.current, Some(1));
        assert!((l.highlighted_chars - 1.5).abs() < 0.01, "{}", l.highlighted_chars);
        assert_eq!(l.highlighted_chars, t.position_at(15_500).unwrap().highlighted_chars);
        // Blank line = interlude; the next non-blank line is announced.
        let l = overlay_lines(&t, 19_000);
        assert_eq!((l.current, l.next, l.intro), (None, Some(3), false));
        let l = overlay_lines(&t, 60_000);
        assert_eq!((l.current, l.next), (Some(3), None));
    }

    #[test]
    fn user_offset_moves_the_lines() {
        let mut t = LyricsTimeline::parse(LRC);
        t.set_user_offset_ms(2_000); // lyrics earlier
        assert_eq!(overlay_lines(&t, 8_500).current, Some(0));
        assert_eq!(ms_to_next_line(&t, 8_500), Some(3_500));
        t.set_user_offset_ms(-2_000);
        assert_eq!(overlay_lines(&t, 11_000).current, None);
    }

    #[test]
    fn position_extrapolates_and_never_jitters_back() {
        let t0 = Instant::now();
        let s = PositionSample { position_ms: 1_000, at: t0, advancing: true, duration_ms: Some(1_400) };
        assert_eq!(s.at_time(t0 + Duration::from_millis(250)), 1_250);
        assert_eq!(s.at_time(t0 + Duration::from_secs(5)), 1_400, "clamped to the duration");
        let paused = PositionSample { advancing: false, ..s };
        assert_eq!(paused.at_time(t0 + Duration::from_secs(5)), 1_000);
        assert_eq!(smooth_position(Some(5_100), 5_000), 5_100, "small step back holds");
        assert_eq!(smooth_position(Some(5_100), 5_200), 5_200);
        assert_eq!(smooth_position(Some(60_000), 1_000), 1_000, "seek back passes");
        assert_eq!(smooth_position(None, 7), 7);
    }

    #[test]
    fn repaints_only_while_needed() {
        assert_eq!(next_repaint(false, true, Some(10)), None, "paused: idle");
        assert_eq!(next_repaint(true, true, Some(10_000)), Some(Duration::ZERO), "karaoke: display rate");
        assert_eq!(next_repaint(true, false, Some(40)), Some(Duration::from_millis(45)));
        assert_eq!(next_repaint(true, false, Some(10_000)), Some(MAX_IDLE));
        assert_eq!(next_repaint(true, false, None), Some(MAX_IDLE));
    }

    #[test]
    fn instrumental_placeholders() {
        assert!(is_instrumental(&LyricsTimeline::parse("[00:00.00][Instrumental]\n")));
        assert!(is_instrumental(&LyricsTimeline::parse("[00:00.00]♪ インストゥルメンタル ♪\n")));
        assert!(is_instrumental(&LyricsTimeline::parse("[00:00.00]此歌曲为没有填词的纯音乐，请您欣赏\n")));
        assert!(!is_instrumental(&LyricsTimeline::parse(LRC)));
        assert!(!is_instrumental(&LyricsTimeline::parse("[00:00.00]Instrumental break\n[00:05.00]歌詞\n")));
    }

    #[test]
    fn geometry_is_clamped_on_screen() {
        let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(1920.0, 1040.0));
        // Off to the right (monitor removed): pulled back fully inside.
        let r = clamp_rect(Rect::from_min_size(pos2(2500.0, 900.0), vec2(640.0, 132.0)), area);
        assert_eq!(r, Rect::from_min_size(pos2(1280.0, 900.0), vec2(640.0, 132.0)));
        // Larger than the screen: shrunk to it.
        let r = clamp_rect(Rect::from_min_size(pos2(-50.0, -50.0), vec2(4000.0, 2000.0)), area);
        assert_eq!(r, area);
        // Below the minimum size: grown.
        let r = clamp_rect(Rect::from_min_size(pos2(10.0, 10.0), vec2(20.0, 20.0)), area);
        assert_eq!(r.size(), MIN_SIZE);
        // Secondary monitor left of the primary (negative coordinates) stays put.
        let left = Rect::from_min_max(pos2(-1920.0, 0.0), pos2(0.0, 1080.0));
        let r = Rect::from_min_size(pos2(-1500.0, 300.0), vec2(640.0, 132.0));
        assert_eq!(clamp_rect(r, left), r);
    }

    #[test]
    fn initial_geometry_from_state() {
        let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(1920.0, 1040.0));
        let saved = WindowStateDto { width: 700.0, height: 140.0, x: Some(3000.0), y: Some(100.0), maximized: false };
        let (pos, size) = initial_geometry(Some(saved), Some(area), None);
        assert_eq!((pos, size), (Some(pos2(1220.0, 100.0)), vec2(700.0, 140.0)));
        // Nothing saved: above the bottom of the main window, horizontally centered.
        let main = Rect::from_min_size(pos2(100.0, 100.0), vec2(1280.0, 820.0));
        let (pos, size) = initial_geometry(None, None, Some(main));
        assert_eq!(size, DEFAULT_SIZE);
        assert_eq!(pos, Some(pos2(740.0 - DEFAULT_SIZE.x / 2.0, 920.0 - DEFAULT_SIZE.y - 110.0)));
        // Garbage position (old / corrupted state): the OS places the window.
        let bad = WindowStateDto { width: f32::NAN, height: 10.0, x: Some(f32::INFINITY), y: Some(5.0), maximized: false };
        assert_eq!(initial_geometry(Some(bad), None, None), (None, DEFAULT_SIZE));
    }

    fn pass(ctx: &egui::Context, app: &mut GuiApp, minimized: bool) -> egui::FullOutput {
        let mut input = egui::RawInput::default();
        input.viewports.insert(ViewportId::ROOT, egui::ViewportInfo { minimized: Some(minimized), ..Default::default() });
        let mut frame = eframe::Frame::_new_kittest();
        // eframe runs only `logic` for a minimized main window.
        ctx.run_ui(input, |ui| eframe::App::logic(app, ui.ctx(), &mut frame))
    }

    #[test]
    fn viewport_is_kept_alive_from_logic_and_closed_when_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false); // as on desktop eframe
        let mut app = super::super::tests::test_app(&ctx, dir.path(), false);
        let out = pass(&ctx, &mut app, true);
        assert!(!out.viewport_output.contains_key(&viewport_id()), "off by default");

        app.config.desktop_lyrics.enabled = true;
        app.config.desktop_lyrics.locked = true;
        for _ in 0..3 {
            let out = pass(&ctx, &mut app, true);
            let vp = out.viewport_output.get(&viewport_id()).expect("shown while the main window is minimized");
            assert!(vp.class == ViewportClass::Deferred, "deferred viewport");
            assert_eq!(vp.parent, ViewportId::ROOT);
            assert_eq!(vp.builder.mouse_passthrough, Some(true), "locked = click-through");
            assert_eq!(vp.builder.window_level, Some(egui::WindowLevel::AlwaysOnTop));
            assert_eq!((vp.builder.decorations, vp.builder.transparent, vp.builder.taskbar), (Some(false), Some(true), Some(false)));
        }
        assert!(app.ui.desktop_lyrics.is_open());

        app.set_desktop_lyrics_locked(&ctx, false);
        let out = pass(&ctx, &mut app, false);
        assert_eq!(out.viewport_output[&viewport_id()].builder.mouse_passthrough, Some(false));

        app.config.desktop_lyrics.enabled = false;
        pass(&ctx, &mut app, false);
        let out = pass(&ctx, &mut app, false);
        assert!(!out.viewport_output.contains_key(&viewport_id()), "closed once no longer shown");
        assert!(!app.ui.desktop_lyrics.is_open());
    }

    #[test]
    fn overlay_clicks_and_feed_go_through_the_main_window() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = super::super::tests::test_app(&ctx, dir.path(), false);
        app.config.desktop_lyrics.enabled = true;
        app.play_path = "/music/a.flac".into();
        app.play_title = Some("夜に駆ける".into());
        let mut t = LyricsTimeline::parse(LRC);
        t.set_user_offset_ms(1_500);
        app.loaded_timeline = Some(t);
        app.lyrics_revision += 1;
        app.play_position_ms = 9_000;
        pass(&ctx, &mut app, false);
        {
            let sh = lock(&app.ui.desktop_lyrics.shared);
            let feed = &sh.feed;
            assert!(feed.has_track && feed.title == "夜に駆ける");
            let tl = feed.timeline.as_ref().expect("timeline shared");
            assert_eq!(tl.total_offset_ms(), 1_500, "the user offset of the main view applies");
            assert_eq!(overlay_lines(tl, feed.sample.position_ms).current, Some(0));
            assert_eq!(feed.lang, TextLang::Ja);
        }
        // An offset change (same revision) reaches the overlay too.
        app.set_lyrics_offset(0);
        pass(&ctx, &mut app, false);
        assert_eq!(lock(&app.ui.desktop_lyrics.shared).feed.timeline.as_ref().unwrap().total_offset_ms(), 0);

        let size = app.config.desktop_lyrics.font_size;
        lock(&app.ui.desktop_lyrics.shared).actions.extend([DesktopLyricsAction::FontLarger, DesktopLyricsAction::Lock]);
        pass(&ctx, &mut app, false);
        assert_eq!(app.config.desktop_lyrics.font_size, size + FONT_STEP);
        assert!(app.config.desktop_lyrics.locked);
        lock(&app.ui.desktop_lyrics.shared).actions.push(DesktopLyricsAction::Close);
        pass(&ctx, &mut app, false);
        assert!(!app.config.desktop_lyrics.enabled);

        // Geometry reported by the window is persisted with the state.
        lock(&app.ui.desktop_lyrics.shared).geometry = Some(Rect::from_min_size(pos2(40.4, 900.0), vec2(500.0, 120.0)));
        let w = app.build_state_dto().desktop_lyrics_window.unwrap();
        assert_eq!((w.x, w.y, w.width, w.height), (Some(40.0), Some(900.0), 500.0, 120.0));
    }

    #[test]
    fn shortcuts_are_consumed_before_the_lyrics_panel_shortcut() {
        use egui::{Event, Key, Modifiers, RawInput};
        let ctx = egui::Context::default();
        let press = |modifiers| Event::Key { key: Key::L, physical_key: None, pressed: true, repeat: false, modifiers };
        let input = RawInput { events: vec![press(Modifiers::COMMAND | Modifiers::SHIFT)], ..Default::default() };
        let mut got = (false, false);
        let mut panel = false;
        let _ = ctx.run_ui(input, |ui| {
            got = ui.ctx().input_mut(consume_shortcuts);
            panel = !crate::presentation::ui::shortcuts::global_shortcuts(ui.ctx(), 0.5).is_empty();
        });
        assert_eq!(got, (true, false));
        assert!(!panel, "Ctrl+Shift+L must not also toggle the lyrics panel");
    }
}
