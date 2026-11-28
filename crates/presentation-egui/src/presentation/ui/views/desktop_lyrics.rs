//! Content of the desktop lyrics window (a small always-on-top window, see
//! `app/desktop_lyrics.rs`): the current line large with the karaoke fill of the main lyrics
//! view (`lyrics_view::paint_karaoke`), the next line smaller and dimmer, an optional title
//! row, and controls that fade in while the pointer is over the window.
//!
//! With album art on, a square cover sits left of the text (also in the no-lyrics and
//! instrumental states); it is dropped when the window is too narrow ([`artwork_rect`]).
//! The hover bar's opacity button swaps the buttons for a background opacity slider in place
//! (the text stays visible below it, so the effect shows while dragging).
//!
//! The widget is pure: it draws into the rect it is given and returns
//! [`DesktopLyricsAction`]s. Window commands (drag, resize) are actions too, so the app maps
//! them to viewport commands and tests can render the widget inside an ordinary window.

use eframe::egui::{
    self, epaint::text::TextWrapping, pos2, text::LayoutJob, vec2, Align, Color32, CursorIcon, FontId, Galley, Rect,
    Sense, Stroke, Ui, UiBuilder, Vec2,
};
use std::sync::Arc;

pub use eframe::egui::viewport::ResizeDirection;

use crate::presentation::ui::{
    i18n::Tr,
    icons::{icon_button_ex, paint_icon, Icon, IconButtonStyle},
    text::{self, TextLang},
    theme,
    views::lyrics_view::{line_text, paint_karaoke},
    widgets::{cover_uv, slim_slider, SlimSliderStyle, Tex},
};

/// The current lyric line as the overlay draws it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayLine<'a> {
    pub text: &'a str,
    /// The line has (real or estimated) karaoke timing.
    pub karaoke: bool,
    /// Highlighted chars (fractional), as in `LyricPosition::highlighted_chars`.
    pub highlighted_chars: f32,
}

/// What the window shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesktopLyricsContent<'a> {
    /// Nothing is loaded.
    NoTrack,
    /// A track without lyrics: title and artist instead.
    NoLyrics,
    /// The lyrics say the track is instrumental.
    Instrumental,
    Lyrics {
        /// `None` during an interlude (blank line) or before the first line (`intro`).
        current: Option<OverlayLine<'a>>,
        /// Before the first line: the title is shown dimmed in place of a line.
        intro: bool,
        next: Option<&'a str>,
        /// Changes whenever the current line changes (fades the new line in).
        line_key: u64,
        /// Dominant language of the song (per-line CJK font choice, see `ui::text`).
        lang: TextLang,
    },
}

pub struct DesktopLyricsProps<'a> {
    pub content: DesktopLyricsContent<'a>,
    pub title: &'a str,
    pub artist: &'a str,
    /// Point size of the current line.
    pub font_size: f32,
    /// Background opacity `0..=1`.
    pub opacity: f32,
    pub show_next_line: bool,
    pub show_title: bool,
    /// Click-through: no controls, a lock hint instead.
    pub locked: bool,
    pub playing: bool,
    pub has_track: bool,
    /// Show the hover controls regardless of the pointer (screenshots / tests).
    pub force_controls: bool,
    /// The current track's album art (`None`: a placeholder tile).
    pub artwork: Option<Tex>,
    /// The art column is on (setting); still dropped when nothing is loaded or the window is
    /// too narrow.
    pub show_artwork: bool,
    pub tr: Tr,
}

/// User intents from the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesktopLyricsAction {
    Previous,
    PlayPause,
    Next,
    FontSmaller,
    FontLarger,
    Lock,
    /// Bring the main window to the front.
    OpenMain,
    Close,
    /// New background opacity `0..=1` from the hover bar's slider (sent while dragging).
    SetOpacity(f32),
    /// Move the window with the pointer (`ViewportCommand::StartDrag`).
    StartDrag,
    /// Resize the window from an edge (`ViewportCommand::BeginResize`).
    BeginResize(ResizeDirection),
}

/// Animation state kept between frames.
#[derive(Debug, Default, Clone)]
pub struct DesktopLyricsViewState {
    line_key: Option<u64>,
    line_since: f64,
    locked_since: Option<f64>,
    /// The opacity slider replaces the buttons of the hover bar.
    opacity_open: bool,
    /// The slider is being dragged (last frame).
    opacity_dragging: bool,
    /// Slider value the props do not reflect yet (the main window applies it a pass later).
    opacity_local: Option<f32>,
    /// When the pointer left the window while the slider was open.
    pointer_left_at: Option<f64>,
}

impl DesktopLyricsViewState {
    /// Opens / closes the opacity slider (screenshots and tests).
    pub fn set_opacity_slider_open(&mut self, open: bool) {
        self.opacity_open = open;
    }

    pub fn opacity_slider_open(&self) -> bool {
        self.opacity_open
    }
}

/// Fade-in time of a new line.
const LINE_FADE_S: f64 = 0.18;
/// How long the "locked" hint stays after locking.
const LOCK_HINT_S: f64 = 4.0;
const PAD_X: f32 = 18.0;
const PAD_Y: f32 = 10.0;
const CORNER: f32 = 14.0;
/// Resize grip width along the window edges.
const EDGE: f32 = 6.0;
/// Height of the controls row.
const CONTROLS_H: f32 = 30.0;
/// The next line's size relative to the current one.
const NEXT_RATIO: f32 = 0.6;
/// A long current line is shrunk down to this fraction of the font size before it wraps.
const MIN_FIT: f32 = 0.62;
/// Album art: margin to the window edges, gap to the text, size limits, corner radius.
const ART_PAD: f32 = 10.0;
const ART_GAP: f32 = 14.0;
const ART_MIN: f32 = 40.0;
const ART_MAX: f32 = 160.0;
const ART_CORNER: f32 = 6.0;
/// Smallest art square drawn (a window lower than this plus margins shows none).
const ART_MIN_SIDE: f32 = 16.0;
/// The opacity slider closes this long after the pointer left the window.
const OPACITY_GRACE_S: f64 = 1.5;
/// Below this window width the hover bar drops A−/A+.
const COMPACT_W: f32 = 330.0;
/// Hover bar buttons: icon size, padding, spacing; separator width.
const BAR_ICON: f32 = 15.0;
const BAR_PAD: f32 = 6.5;
const BAR_SPACING: f32 = 2.0;
const BAR_SEP: f32 = 9.0;

/// Where the album art goes in a window `full`: a square left of the text, as tall as the
/// window minus margins (at most [`ART_MAX`]), vertically centered. Shown whenever the setting
/// is on (whatever the width); `None` only when the window is too low to draw one.
pub fn artwork_rect(full: Rect) -> Option<Rect> {
    let side = (full.height() - 2.0 * ART_PAD).min(ART_MAX);
    (side.is_finite() && side >= ART_MIN_SIDE).then(|| Rect::from_min_size(pos2(full.left() + ART_PAD, full.center().y - side / 2.0), Vec2::splat(side)))
}

/// The art while the hover bar is up: when the bar reaches over it, it shrinks into the space
/// below the bar (bottom-left anchored), like the text moves below the bar. `None`: no room.
pub(crate) fn artwork_under_bar(art: Rect, bar: Rect, full: Rect) -> Option<Rect> {
    let pill = bar.expand2(vec2(8.0, 1.0));
    if pill.left() >= art.right() + 4.0 {
        return Some(art);
    }
    let side = art.height().min(full.bottom() - ART_PAD - (pill.bottom() + 4.0));
    (side >= ART_MIN * 0.75).then(|| Rect::from_min_size(pos2(art.left(), art.bottom() - side), Vec2::splat(side)))
}

/// The text column: right of the art, else the window minus its padding.
fn text_rect(full: Rect, art: Option<Rect>) -> Rect {
    let inner = full.shrink2(vec2(PAD_X, PAD_Y));
    match art {
        Some(a) => Rect::from_min_max(pos2(a.right() + ART_GAP, inner.top()), inner.max),
        None => inner,
    }
}

/// The hover bar: (row rect, compact). Centered over the text column, kept inside the window
/// (over the art when the column is narrower than the bar).
pub(crate) fn controls_row(full: Rect, text: Rect) -> (Rect, bool) {
    let compact = full.width() < COMPACT_W;
    let side = BAR_ICON + BAR_PAD * 2.0;
    // prev play next | (A− A+) opacity | lock open close
    let n = if compact { 7.0 } else { 9.0 };
    let groups = if compact { 1.0 } else { 2.0 };
    let row_w = n * side + (n - 1.0) * BAR_SPACING + groups * BAR_SEP;
    let half = row_w / 2.0 + 8.0; // pill margin
    let lo = full.left() + 4.0 + half;
    let hi = full.right() - 4.0 - half;
    let cx = if lo <= hi { text.center().x.clamp(lo, hi) } else { full.center().x };
    (Rect::from_center_size(pos2(cx, full.top() + 6.0 + CONTROLS_H / 2.0), vec2(row_w, CONTROLS_H)), compact)
}

/// One text row of the window.
struct Row {
    galley: Arc<Galley>,
    color: Color32,
    /// Karaoke highlighted chars (`None`: plain text in `color`).
    karaoke: Option<f32>,
    /// Fades / slides in when the line changes.
    fades: bool,
    /// Extra alpha (the title row gives way to the hover controls).
    alpha: f32,
}

/// Colors of the window for a theme.
#[derive(Clone, Copy, Debug)]
struct Colors {
    bg: Color32,
    text: Color32,
    sung: Color32,
    next: Color32,
    halo: Color32,
}

fn colors(pal: &theme::Palette) -> Colors {
    Colors {
        bg: if pal.dark { theme::blend(pal.bg, Color32::BLACK, 0.25) } else { pal.surface },
        text: pal.text,
        sung: pal.accent_fg,
        next: theme::with_alpha(pal.text_muted, 0.92),
        halo: if pal.dark { Color32::BLACK } else { Color32::WHITE },
    }
}

/// One line laid out centered, at most `max_rows` rows, cut with "…" beyond.
fn layout(ui: &Ui, text: &str, font: FontId, wrap: f32, max_rows: usize) -> Arc<Galley> {
    let mut job = LayoutJob::simple(text.to_owned(), font, Color32::PLACEHOLDER, wrap);
    job.halign = Align::Center;
    job.wrap = TextWrapping { max_width: wrap, max_rows, break_anywhere: false, overflow_character: Some('…') };
    ui.fonts_mut(|f| f.layout_job(job))
}

/// The current line: shrunk (down to [`MIN_FIT`]) to fit one row, wrapped to two rows beyond.
fn layout_current(ui: &Ui, text: &str, lang: TextLang, size: f32, wrap: f32) -> Arc<Galley> {
    let family = text::family_in_context(text, lang, true);
    let natural = ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), FontId::new(size, family.clone()), Color32::PLACEHOLDER));
    let fit = if natural.size().x > wrap { (wrap / natural.size().x).max(MIN_FIT) } else { 1.0 };
    layout(ui, text, FontId::new((size * fit).floor().max(10.0), family), wrap, 2)
}

/// Paint `galley` with a soft outline in `halo` (legibility over a transparent background).
fn paint_halo(painter: &egui::Painter, pos: egui::Pos2, galley: &Arc<Galley>, halo: Color32, strength: f32) {
    if strength <= 0.02 {
        return;
    }
    let c = theme::with_alpha(halo, 0.5 * strength);
    for (dx, dy) in [(-1.2, 0.0), (1.2, 0.0), (0.0, -1.2), (0.0, 1.2), (-0.9, -0.9), (0.9, -0.9), (-0.9, 0.9), (0.9, 0.9)] {
        painter.galley(pos + vec2(dx, dy), galley.clone(), c);
    }
}

/// Edge zones that resize the window (corners first so they win).
fn resize_zones(r: Rect) -> [(Rect, ResizeDirection, CursorIcon); 8] {
    let e = EDGE;
    let c = EDGE * 2.0;
    [
        (Rect::from_min_size(r.min, Vec2::splat(c)), ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest),
        (Rect::from_min_size(pos2(r.right() - c, r.top()), Vec2::splat(c)), ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast),
        (Rect::from_min_size(pos2(r.left(), r.bottom() - c), Vec2::splat(c)), ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest),
        (Rect::from_min_size(r.max - Vec2::splat(c), Vec2::splat(c)), ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast),
        (Rect::from_min_max(r.min, pos2(r.right(), r.top() + e)), ResizeDirection::North, CursorIcon::ResizeNorth),
        (Rect::from_min_max(pos2(r.left(), r.bottom() - e), r.max), ResizeDirection::South, CursorIcon::ResizeSouth),
        (Rect::from_min_max(r.min, pos2(r.left() + e, r.bottom())), ResizeDirection::West, CursorIcon::ResizeWest),
        (Rect::from_min_max(pos2(r.right() - e, r.top()), r.max), ResizeDirection::East, CursorIcon::ResizeEast),
    ]
}

/// Draw the desktop lyrics content filling `ui.max_rect()`.
pub fn desktop_lyrics_view(ui: &mut Ui, props: &DesktopLyricsProps<'_>, state: &mut DesktopLyricsViewState) -> Vec<DesktopLyricsAction> {
    let pal = theme::palette(ui);
    let col = colors(&pal);
    let tr = props.tr;
    let mut actions = Vec::new();
    let full = ui.max_rect();
    let id = ui.id().with("desktop_lyrics");
    let now = ui.input(|i| i.time);

    // ---- hover (geometric: buttons and their tooltips never flip it) ----
    let pointer_in = !props.locked && ui.ctx().pointer_hover_pos().is_some_and(|p| full.contains(p));
    update_opacity_slider(ui, props, state, pointer_in, now);
    // The open slider keeps the bar up (a drag may leave the window).
    let show_controls = !props.locked && (pointer_in || props.force_controls || state.opacity_open);
    let t = ui.ctx().animate_bool_with_time(id.with("hover"), show_controls, 0.15);
    let opacity = state.opacity_local.unwrap_or(props.opacity).clamp(0.0, 1.0);

    // ---- window interaction (under the controls, which are added later) ----
    if !props.locked {
        let bg = ui.interact(full, id.with("bg"), Sense::click_and_drag());
        if bg.drag_started_by(egui::PointerButton::Primary) {
            actions.push(DesktopLyricsAction::StartDrag);
        }
        if bg.double_clicked() {
            actions.push(DesktopLyricsAction::OpenMain);
        }
        for (i, (r, dir, cursor)) in resize_zones(full).into_iter().enumerate() {
            let resp = ui.interact(r, id.with(("edge", i)), Sense::drag()).on_hover_cursor(cursor);
            if resp.drag_started_by(egui::PointerButton::Primary) {
                actions.push(DesktopLyricsAction::BeginResize(dir));
            }
        }
    }

    // ---- background ----
    let painter = ui.painter_at(full);
    // While hovered the window gets at least a faint background, so its bounds are visible —
    // except while the opacity slider is open: then it shows exactly the chosen value.
    let bg_alpha = if state.opacity_open { opacity } else { opacity.max(0.45 * t) };
    if bg_alpha > 0.0 {
        painter.rect_filled(full, CORNER, theme::with_alpha(col.bg, bg_alpha));
    }
    if t > 0.0 {
        painter.rect_stroke(full.shrink(0.5), CORNER, Stroke::new(1.0, theme::with_alpha(pal.border_strong, t)), egui::StrokeKind::Inside);
    }
    // Outline strength: strong over a see-through background, none over an opaque one.
    let halo = (1.0 - bg_alpha / 0.6).clamp(0.0, 1.0);

    // ---- line change fade ----
    let line_key = match props.content {
        DesktopLyricsContent::Lyrics { line_key, .. } => line_key,
        _ => u64::MAX,
    };
    if state.line_key != Some(line_key) {
        // No fade for the very first frame (window just opened).
        state.line_since = if state.line_key.is_some() { now } else { now - LINE_FADE_S };
        state.line_key = Some(line_key);
    }
    let fade = (((now - state.line_since) / LINE_FADE_S) as f32).clamp(0.0, 1.0);
    if fade < 1.0 {
        ui.ctx().request_repaint();
    }

    // ---- album art ----
    let size = props.font_size.clamp(10.0, 120.0);
    let art = (props.show_artwork && props.has_track && props.content != DesktopLyricsContent::NoTrack)
        .then(|| artwork_rect(full))
        .flatten();
    // ---- text ----
    let inner = text_rect(full, art);
    if let Some(r) = art {
        // Out of the hover bar's way (the column keeps its place).
        let (bar, _) = controls_row(full, inner);
        let (shown, a) = match artwork_under_bar(r, bar, full) {
            Some(h) => (lerp_rect(r, h, t), 1.0),
            // No room below the bar (tiny window): the art stays; the bar draws over it.
            None => (r, 1.0),
        };
        if a > 0.01 {
            paint_art(ui, shown, props.artwork, &col, pal.dark, a);
        }
    }
    let wrap = inner.width().max(40.0);
    let next_size = (size * NEXT_RATIO).max(12.0);
    let title_line = if props.artist.trim().is_empty() {
        props.title.to_owned()
    } else {
        format!("{} — {}", props.title, props.artist)
    };

    let mut rows: Vec<Row> = Vec::new();
    let show_title_row = props.show_title && props.has_track && !matches!(props.content, DesktopLyricsContent::NoLyrics | DesktopLyricsContent::Instrumental);
    if show_title_row {
        let g = layout(ui, &title_line, text::font_id(&title_line, 12.5, false), wrap, 1);
        rows.push(Row { galley: g, color: col.next, karaoke: None, fades: false, alpha: 1.0 - t });
    }
    match props.content {
        DesktopLyricsContent::NoTrack => {
            let s = tr.t("desktop_lyrics.no_track");
            rows.push(Row { galley: layout(ui, s, text::font_id(s, next_size, false), wrap, 1), color: col.next, karaoke: None, fades: false, alpha: 1.0 });
        }
        DesktopLyricsContent::NoLyrics | DesktopLyricsContent::Instrumental => {
            let instrumental = props.content == DesktopLyricsContent::Instrumental;
            let first = if instrumental { format!("♪ {}", props.title) } else { props.title.to_owned() };
            let g = layout(ui, &first, text::font_id(&first, (size * 0.72).max(13.0), true), wrap, 1);
            rows.push(Row { galley: g, color: col.text, karaoke: None, fades: false, alpha: 1.0 });
            let second = match (instrumental, props.artist.trim().is_empty()) {
                (true, _) => tr.t("desktop_lyrics.instrumental").to_owned(),
                (false, true) => tr.t("desktop_lyrics.no_lyrics").to_owned(),
                (false, false) => format!("{} · {}", props.artist, tr.t("desktop_lyrics.no_lyrics")),
            };
            let g = layout(ui, &second, text::font_id(&second, next_size.min(16.0), false), wrap, 1);
            rows.push(Row { galley: g, color: col.next, karaoke: None, fades: false, alpha: 1.0 });
        }
        DesktopLyricsContent::Lyrics { current, intro, next, lang, .. } => {
            match current {
                Some(line) => {
                    let shown = line_text(line.text);
                    let g = layout_current(ui, &shown, lang, size, wrap);
                    let chars = line.karaoke.then_some(line.highlighted_chars);
                    rows.push(Row { galley: g, color: col.text, karaoke: chars, fades: true, alpha: 1.0 });
                }
                None if intro && props.has_track => {
                    let g = layout(ui, &title_line, text::font_id(&title_line, (size * 0.72).max(13.0), true), wrap, 1);
                    rows.push(Row { galley: g, color: col.next, karaoke: None, fades: true, alpha: 1.0 });
                }
                None => {
                    let g = layout(ui, "♪", FontId::proportional(size), wrap, 1);
                    rows.push(Row { galley: g, color: col.next, karaoke: None, fades: true, alpha: 1.0 });
                }
            }
            if props.show_next_line {
                if let Some(n) = next.filter(|n| !n.trim().is_empty()) {
                    let g = layout(ui, n.trim_end(), FontId::new(next_size, text::family_in_context(n, lang, false)), wrap, 1);
                    rows.push(Row { galley: g, color: col.next, karaoke: None, fades: true, alpha: 1.0 });
                }
            }
        }
    }

    let gap = (size * 0.18).clamp(4.0, 12.0);
    // While the controls are shown, the text moves below them: the title row gives way to the
    // controls, and in a short window the next line too.
    let next_row = match props.content {
        DesktopLyricsContent::Lyrics { next: Some(n), .. } if props.show_next_line && !n.trim().is_empty() => Some(rows.len() - 1),
        _ => None,
    };
    let title_row = show_title_row.then_some(0);
    let heights = |collapse_next: bool| -> Vec<f32> {
        rows.iter()
            .enumerate()
            .map(|(i, r)| {
                let hidden = Some(i) == title_row || (collapse_next && Some(i) == next_row);
                (r.galley.size().y + gap) * if hidden { 1.0 - t } else { 1.0 }
            })
            .collect()
    };
    let area_top = inner.top() + (full.top() + 8.0 + CONTROLS_H - inner.top()).max(0.0) * t;
    let area = Rect::from_min_max(pos2(inner.left(), area_top), inner.max);
    let full_heights = heights(false);
    let collapse_next = t > 0.0 && full_heights.iter().sum::<f32>() - gap > area.height() + 0.5;
    let hs = if collapse_next { heights(true) } else { full_heights };
    let block_h = (hs.iter().sum::<f32>() - gap).max(0.0);
    let mut y = if block_h <= area.height() { area.center().y - block_h / 2.0 } else { area.top() };
    for (i, Row { galley, color, karaoke: chars, fades, alpha: extra }) in rows.iter().enumerate() {
        let collapsing = collapse_next && Some(i) == next_row;
        let a = if *fades { fade * extra } else { *extra } * if collapsing { 1.0 - t } else { 1.0 };
        let slide = if *fades { (1.0 - fade) * 6.0 } else { 0.0 };
        if a > 0.01 {
            // Galleys are centered (`halign`): their rect spans -w/2..w/2 around the anchor.
            let pos = pos2(inner.center().x, y + slide);
            paint_halo(&painter, pos, galley, col.halo, halo * a);
            match chars {
                Some(c) => paint_karaoke(&painter, pos, galley, *c, col.text, col.sung, a),
                None => {
                    painter.galley(pos, galley.clone(), theme::with_alpha(*color, a));
                }
            }
        }
        y += hs[i];
    }

    // ---- lock hint ----
    if props.locked {
        let since = *state.locked_since.get_or_insert(now);
        let age = now - since;
        let hint_a = if age < LOCK_HINT_S { (((LOCK_HINT_S - age) / 0.4) as f32).clamp(0.0, 1.0) } else { 0.0 };
        if hint_a > 0.0 {
            ui.ctx().request_repaint();
            let s = tr.t("desktop_lyrics.locked_hint");
            let g = layout(ui, s, FontId::proportional(12.0), (full.width() - 24.0).max(40.0), 2);
            let r = Rect::from_center_size(pos2(full.center().x, full.top() + 8.0 + g.size().y / 2.0 + 4.0), g.size() + vec2(20.0, 8.0));
            painter.rect_filled(r, 8.0, theme::with_alpha(col.bg, 0.92 * hint_a));
            painter.galley(pos2(r.center().x, r.top() + 4.0), g, theme::with_alpha(col.text, hint_a));
        } else if age < LOCK_HINT_S + 1.0 {
            ui.ctx().request_repaint();
        }
        // A small padlock stays in the corner while locked.
        let c = pos2(full.right() - 16.0, full.top() + 16.0);
        painter.circle_filled(c, 10.0, theme::with_alpha(col.bg, 0.55));
        paint_icon(&painter, Rect::from_center_size(c, Vec2::splat(13.0)), Icon::Lock, theme::with_alpha(col.text, 0.6));
    } else {
        state.locked_since = None;
    }

    // ---- controls ----
    if t > 0.0 && !props.locked {
        let (row, compact) = controls_row(full, inner);
        controls(ui, props, state, opacity, full, row, compact, t, &col, &mut actions);
    }
    actions
}

/// Opens / closes bookkeeping of the opacity slider: closed while locked, after
/// [`OPACITY_GRACE_S`] with the pointer outside the window (and no drag), or on Escape; the
/// local value is dropped once the props caught up.
fn update_opacity_slider(ui: &Ui, props: &DesktopLyricsProps<'_>, state: &mut DesktopLyricsViewState, pointer_in: bool, now: f64) {
    if props.locked || (state.opacity_open && ui.input(|i| i.key_pressed(egui::Key::Escape))) {
        state.opacity_open = false;
    }
    if state.opacity_open {
        if pointer_in || state.opacity_dragging || props.force_controls {
            state.pointer_left_at = None;
        } else {
            let left = *state.pointer_left_at.get_or_insert(now);
            let remaining = OPACITY_GRACE_S - (now - left);
            if remaining <= 0.0 {
                state.opacity_open = false;
            } else {
                // One wake-up to close it (no animation runs meanwhile).
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(remaining + 0.02));
            }
        }
    }
    if !state.opacity_open {
        state.pointer_left_at = None;
        state.opacity_dragging = false;
    }
    if let Some(v) = state.opacity_local {
        let caught_up = (v - props.opacity).abs() < 0.005;
        if (caught_up && !state.opacity_dragging) || !state.opacity_open {
            state.opacity_local = None;
        }
    }
}

/// The album art (or a quiet placeholder tile) with a hairline edge, so a white cover keeps
/// its shape on the light panel and a black one on the dark panel.
fn paint_art(ui: &Ui, rect: Rect, tex: Option<Tex>, col: &Colors, dark: bool, alpha: f32) {
    let painter = ui.painter();
    match tex {
        Some(t) => {
            egui::Image::new((t.id, rect.size()))
                .uv(cover_uv(t.size, rect.size()))
                .corner_radius(ART_CORNER)
                .tint(Color32::WHITE.gamma_multiply(alpha))
                .paint_at(ui, rect);
        }
        None => {
            painter.rect_filled(rect, ART_CORNER, theme::with_alpha(col.bg, 0.5 * alpha));
            painter.rect_filled(rect, ART_CORNER, theme::with_alpha(col.text, 0.07 * alpha));
            let icon = (rect.width() * 0.36).clamp(14.0, 48.0);
            paint_icon(painter, Rect::from_center_size(rect.center(), Vec2::splat(icon)), Icon::Note, theme::with_alpha(col.text, 0.32 * alpha));
        }
    }
    let edge = if dark { Color32::from_white_alpha(26) } else { Color32::from_black_alpha(24) };
    painter.rect_stroke(rect, ART_CORNER, Stroke::new(1.0, edge.gamma_multiply(alpha)), egui::StrokeKind::Inside);
}

fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    Rect::from_min_max(a.min.lerp(b.min, t), a.max.lerp(b.max, t))
}

#[allow(clippy::too_many_arguments)]
fn controls(
    ui: &mut Ui,
    props: &DesktopLyricsProps<'_>,
    state: &mut DesktopLyricsViewState,
    opacity: f32,
    full: Rect,
    row: Rect,
    compact: bool,
    t: f32,
    col: &Colors,
    actions: &mut Vec<DesktopLyricsAction>,
) {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let (icon, pad, spacing, sep) = (BAR_ICON, BAR_PAD, BAR_SPACING, BAR_SEP);
    let pill = row.expand2(vec2(8.0, 1.0));
    let painter = ui.painter_at(full);
    painter.rect_filled(pill, CONTROLS_H / 2.0 + 1.0, theme::with_alpha(col.bg, 0.94 * t));

    let mut cui = ui.new_child(UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(Align::Center)));
    cui.multiply_opacity(t);
    cui.spacing_mut().item_spacing.x = spacing;
    let style = IconButtonStyle { color: Some(col.text), hover_fill: Some(pal.hover), padding: Some(pad), ..Default::default() };
    let enabled = IconButtonStyle { enabled: props.has_track, ..style };

    if state.opacity_open {
        opacity_slider_row(&mut cui, props, state, opacity, row, style, col, actions);
        return;
    }

    let mut toggle_opacity = false;
    let mut button = |ui: &mut Ui, icon_kind: Icon, st: IconButtonStyle, tip: &str, action: Option<DesktopLyricsAction>| {
        if icon_button_ex(ui, icon_kind, icon, st, tip).clicked() {
            match action {
                Some(a) => actions.push(a),
                None => toggle_opacity = true,
            }
        }
    };
    let separator = |ui: &mut Ui| {
        let (r, _) = ui.allocate_exact_size(vec2(sep - spacing, CONTROLS_H), Sense::hover());
        ui.painter().vline(r.center().x, (r.center().y - 7.0)..=(r.center().y + 7.0), Stroke::new(1.0, theme::with_alpha(pal.border_strong, t)));
    };
    button(&mut cui, Icon::Previous, enabled, tr.t("transport.previous"), Some(DesktopLyricsAction::Previous));
    let (play_icon, play_tip) = if props.playing { (Icon::Pause, tr.t("transport.pause")) } else { (Icon::Play, tr.t("transport.play")) };
    button(&mut cui, play_icon, style, play_tip, Some(DesktopLyricsAction::PlayPause));
    button(&mut cui, Icon::Next, enabled, tr.t("transport.next"), Some(DesktopLyricsAction::Next));
    separator(&mut cui);
    if !compact {
        button(&mut cui, Icon::TextSmaller, style, tr.t("lyrics.font_smaller"), Some(DesktopLyricsAction::FontSmaller));
        button(&mut cui, Icon::TextLarger, style, tr.t("lyrics.font_larger"), Some(DesktopLyricsAction::FontLarger));
    }
    button(&mut cui, Icon::Opacity, style, tr.t("desktop_lyrics.opacity"), None);
    if !compact {
        separator(&mut cui);
    }
    button(&mut cui, Icon::Lock, style, tr.t("desktop_lyrics.lock"), Some(DesktopLyricsAction::Lock));
    button(&mut cui, Icon::OpenWindow, style, tr.t("desktop_lyrics.open_main"), Some(DesktopLyricsAction::OpenMain));
    button(&mut cui, Icon::Close, style, tr.t("desktop_lyrics.close"), Some(DesktopLyricsAction::Close));
    if toggle_opacity {
        state.opacity_open = true;
        state.pointer_left_at = None;
    }
}

/// The hover bar while adjusting the background: back button, icon, slider, value.
#[allow(clippy::too_many_arguments)]
fn opacity_slider_row(
    ui: &mut Ui,
    props: &DesktopLyricsProps<'_>,
    state: &mut DesktopLyricsViewState,
    opacity: f32,
    row: Rect,
    style: IconButtonStyle,
    col: &Colors,
    actions: &mut Vec<DesktopLyricsAction>,
) {
    let pal = theme::palette(ui);
    let tr = props.tr;
    if icon_button_ex(ui, Icon::ChevronLeft, BAR_ICON, style, tr.t("desktop_lyrics.opacity_back")).clicked() {
        state.opacity_open = false;
    }
    let (r, _) = ui.allocate_exact_size(vec2(18.0, CONTROLS_H), Sense::hover());
    paint_icon(ui.painter(), Rect::from_center_size(r.center(), Vec2::splat(14.0)), Icon::Opacity, theme::with_alpha(col.text, 0.8));
    let pct = format!("{:.0}%", opacity * 100.0);
    let label = ui.fonts_mut(|f| f.layout_no_wrap("100%".to_owned(), FontId::proportional(12.5), col.text));
    let label_w = label.size().x + 6.0;
    let used = ui.min_rect().width() + ui.spacing().item_spacing.x * 2.0;
    let width = (row.width() - used - label_w).max(60.0);
    let out = slim_slider(ui, ui.id().with("desktop_lyrics_opacity"), opacity, width, SlimSliderStyle::themed(&pal));
    let out_resp = out.response.on_hover_text(tr.t("desktop_lyrics.opacity"));
    state.opacity_dragging = out_resp.dragged();
    if let Some(v) = out.dragging.or(out.committed) {
        // Whole percents: a drag sends a value only when it changes by one.
        let v = (v * 100.0).round() / 100.0;
        if (v - opacity).abs() >= 0.005 || out.committed.is_some() {
            state.opacity_local = Some(v);
            actions.push(DesktopLyricsAction::SetOpacity(v));
        }
    }
    let (lr, _) = ui.allocate_exact_size(vec2(label_w, CONTROLS_H), Sense::hover());
    let g = ui.fonts_mut(|f| f.layout_no_wrap(pct, FontId::proportional(12.5), col.text));
    ui.painter().galley(pos2(lr.right() - g.size().x, lr.center().y - g.size().y / 2.0), g, col.text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Context, Event, PointerButton, RawInput};

    fn props(content: DesktopLyricsContent<'_>) -> DesktopLyricsProps<'_> {
        DesktopLyricsProps {
            content,
            title: "夜の灯台",
            artist: "SORAMICHI",
            font_size: 32.0,
            opacity: 0.7,
            show_next_line: true,
            show_title: false,
            locked: false,
            playing: true,
            has_track: true,
            force_controls: false,
            artwork: None,
            show_artwork: true,
            tr: Tr::JA,
        }
    }

    const W: Rect = Rect { min: pos2(0.0, 0.0), max: pos2(640.0, 140.0) };
    const SIDE: f32 = BAR_ICON + BAR_PAD * 2.0;

    fn click(ctx: &Context, at: egui::Pos2, p: &DesktopLyricsProps<'_>, st: &mut DesktopLyricsViewState) -> Vec<DesktopLyricsAction> {
        run(ctx, vec![Event::PointerMoved(at)], p, st);
        let mut got = Vec::new();
        for pressed in [true, false] {
            got.extend(run(ctx, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }], p, st));
        }
        got
    }

    /// The hover bar of the test window (art on, 32 pt).
    fn bar() -> Rect {
        controls_row(W, text_rect(W, artwork_rect(W))).0
    }

    fn run(ctx: &Context, events: Vec<Event>, p: &DesktopLyricsProps<'_>, st: &mut DesktopLyricsViewState) -> Vec<DesktopLyricsAction> {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 140.0))),
            events,
            ..Default::default()
        };
        let mut out = Vec::new();
        let _ = ctx.run_ui(input, |ui| out = desktop_lyrics_view(ui, p, st));
        out
    }

    #[test]
    fn controls_appear_on_hover_and_emit_actions() {
        let ctx = Context::default();
        let mut st = DesktopLyricsViewState::default();
        let line = OverlayLine { text: "最終電車を見送ってから歩く", karaoke: true, highlighted_chars: 4.0 };
        let p = props(DesktopLyricsContent::Lyrics { current: Some(line), intro: false, next: Some("次"), line_key: 1, lang: TextLang::Ja });
        // Hover: the controls fade in (animation needs a few frames).
        for _ in 0..20 {
            run(&ctx, vec![Event::PointerMoved(pos2(320.0, 70.0))], &p, &mut st);
        }
        // The close button is the right-most control of the row at the top.
        let row = bar();
        assert_eq!(row.width(), 9.0 * SIDE + 8.0 * BAR_SPACING + 2.0 * BAR_SEP, "9 buttons, 2 separators");
        let close = pos2(row.right() - SIDE / 2.0, row.center().y);
        assert_eq!(click(&ctx, close, &p, &mut st), vec![DesktopLyricsAction::Close]);
    }

    #[test]
    fn art_goes_left_of_the_text_at_any_width() {
        let r = artwork_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 144.0))).expect("default size shows the art");
        assert_eq!(r, Rect::from_min_size(pos2(ART_PAD, ART_PAD), Vec2::splat(124.0)), "square, window height minus margins");
        let text = text_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 144.0)), Some(r));
        assert_eq!((text.left(), text.right()), (r.right() + ART_GAP, 640.0 - PAD_X));
        // Shown whatever the width (the setting decides), including the minimum window size.
        for size in [vec2(260.0, 72.0), vec2(300.0, 110.0), vec2(390.0, 144.0)] {
            assert!(artwork_rect(Rect::from_min_size(pos2(0.0, 0.0), size)).is_some(), "{size:?}");
        }
        // A tall window caps the art and centers it vertically.
        let tall = Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 400.0));
        let r = artwork_rect(tall).unwrap();
        assert_eq!((r.width(), r.center().y, r.left()), (ART_MAX, tall.center().y, 100.0 + ART_PAD));
        // Only a window too low to draw one shows none.
        assert_eq!(artwork_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 30.0))), None);
    }

    #[test]
    fn hover_bar_fits_the_minimum_window_and_sits_over_the_text() {
        let min = Rect::from_min_size(pos2(0.0, 0.0), crate::presentation::app::desktop_lyrics::MIN_SIZE);
        let (row, compact) = controls_row(min, text_rect(min, artwork_rect(min)));
        assert!(compact);
        assert_eq!(row.width(), 7.0 * SIDE + 6.0 * BAR_SPACING + BAR_SEP, "opacity button included");
        assert!(min.shrink(4.0).contains_rect(row.expand2(vec2(8.0, 0.0))), "{row:?} in {min:?}");
        // With the art, the bar is centered over the text column.
        let text = text_rect(W, artwork_rect(W));
        assert_eq!(bar().center().x, text.center().x);
        assert_eq!(artwork_under_bar(artwork_rect(W).unwrap(), bar(), W), artwork_rect(W), "no overlap: stays");
        // A column narrower than the bar: the bar stays inside the window and reaches over the
        // art, which shrinks below it.
        let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(430.0, 144.0));
        let art = artwork_rect(narrow).expect("art at 430");
        let (row, _) = controls_row(narrow, text_rect(narrow, Some(art)));
        assert!(narrow.shrink(4.0).contains_rect(row.expand2(vec2(8.0, 0.0))));
        assert!(row.left() - 8.0 < art.right(), "overlaps");
        let under = artwork_under_bar(art, row, narrow).unwrap();
        assert!(under.top() > row.bottom() && under.bottom() == art.bottom() && under.width() == under.height(), "{under:?}");
    }

    #[test]
    fn opacity_slider_opens_in_the_bar_and_stays_while_open() {
        let ctx = Context::default();
        let mut st = DesktopLyricsViewState::default();
        let p = props(DesktopLyricsContent::NoLyrics);
        for _ in 0..20 {
            run(&ctx, vec![Event::PointerMoved(pos2(320.0, 90.0))], &p, &mut st);
        }
        // prev play next | A− A+ [opacity] | …
        let row = bar();
        let button = pos2(row.left() + 5.0 * (SIDE + BAR_SPACING) + BAR_SEP + SIDE / 2.0, row.center().y);
        assert!(click(&ctx, button, &p, &mut st).is_empty(), "handled inside the overlay");
        assert!(st.opacity_slider_open());
        run(&ctx, vec![], &p, &mut st);
        // Left end of the slider (after the back button and the icon): 0 %.
        let slider_left = pos2(row.left() + SIDE + 18.0 + 2.0 * BAR_SPACING + 2.0, row.center().y);
        let got = click(&ctx, slider_left, &p, &mut st);
        assert_eq!(got.last(), Some(&DesktopLyricsAction::SetOpacity(0.0)), "{got:?}");
        assert_eq!(st.opacity_local, Some(0.0), "the background follows before the main window answers");
        // Pointer out of the window: still open for a moment, then closed.
        run(&ctx, vec![Event::PointerGone], &p, &mut st);
        for _ in 0..10 {
            run(&ctx, vec![], &p, &mut st);
        }
        assert!(st.opacity_slider_open(), "no fade-out right after leaving");
        for _ in 0..120 {
            run(&ctx, vec![], &p, &mut st);
        }
        assert!(!st.opacity_slider_open(), "closed after the grace period");
        assert_eq!(st.opacity_local, None, "back to the configured value");
        // The back button closes it too; locking closes it.
        st.set_opacity_slider_open(true);
        for _ in 0..3 {
            run(&ctx, vec![Event::PointerMoved(pos2(320.0, 90.0))], &p, &mut st);
        }
        click(&ctx, pos2(row.left() + SIDE / 2.0, row.center().y), &p, &mut st);
        assert!(!st.opacity_slider_open());
        st.set_opacity_slider_open(true);
        let locked = DesktopLyricsProps { locked: true, ..props(DesktopLyricsContent::NoLyrics) };
        run(&ctx, vec![], &locked, &mut st);
        assert!(!st.opacity_slider_open());
    }

    #[test]
    fn locked_window_has_no_controls() {
        let ctx = Context::default();
        let mut st = DesktopLyricsViewState::default();
        let mut p = props(DesktopLyricsContent::NoLyrics);
        p.locked = true;
        p.force_controls = true;
        let mut got = Vec::new();
        for _ in 0..10 {
            got.extend(run(&ctx, vec![Event::PointerMoved(pos2(320.0, 20.0))], &p, &mut st));
        }
        let click = pos2(320.0, 21.0);
        for pressed in [true, false] {
            got.extend(run(&ctx, vec![Event::PointerButton { pos: click, button: PointerButton::Primary, pressed, modifiers: Default::default() }], &p, &mut st));
        }
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn dragging_the_background_moves_the_window() {
        let ctx = Context::default();
        let mut st = DesktopLyricsViewState::default();
        let p = props(DesktopLyricsContent::NoTrack);
        let start = pos2(100.0, 110.0);
        run(&ctx, vec![Event::PointerMoved(start)], &p, &mut st);
        let mut got = run(&ctx, vec![Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Default::default() }], &p, &mut st);
        for dx in [4.0, 10.0, 20.0] {
            got.extend(run(&ctx, vec![Event::PointerMoved(start + vec2(dx, 0.0))], &p, &mut st));
        }
        assert!(got.contains(&DesktopLyricsAction::StartDrag), "{got:?}");
        // An edge resizes instead.
        let mut st = DesktopLyricsViewState::default();
        let edge = pos2(639.0, 70.0);
        run(&ctx, vec![Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: false, modifiers: Default::default() }], &p, &mut st);
        run(&ctx, vec![Event::PointerMoved(edge)], &p, &mut st);
        let mut got = run(&ctx, vec![Event::PointerButton { pos: edge, button: PointerButton::Primary, pressed: true, modifiers: Default::default() }], &p, &mut st);
        for dx in [-4.0, -10.0, -20.0] {
            got.extend(run(&ctx, vec![Event::PointerMoved(edge + vec2(dx, 0.0))], &p, &mut st));
        }
        assert!(got.contains(&DesktopLyricsAction::BeginResize(ResizeDirection::East)), "{got:?}");
    }
}
