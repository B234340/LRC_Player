//! Content of the desktop lyrics window (a small always-on-top window, see
//! `app/desktop_lyrics.rs`): the current line large with the karaoke fill of the main lyrics
//! view (`lyrics_view::paint_karaoke`), the next line smaller and dimmer, an optional title
//! row, and controls that fade in while the pointer is over the window.
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
    let show_controls = !props.locked && (pointer_in || props.force_controls);
    let t = ui.ctx().animate_bool_with_time(id.with("hover"), show_controls, 0.15);

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
    // While hovered the window gets at least a faint background, so its bounds are visible.
    let bg_alpha = props.opacity.clamp(0.0, 1.0).max(0.45 * t);
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

    // ---- text ----
    let inner = full.shrink2(vec2(PAD_X, PAD_Y));
    let wrap = inner.width().max(40.0);
    let size = props.font_size.clamp(10.0, 120.0);
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
        controls(ui, props, full, t, &col, &mut actions);
    }
    actions
}

fn controls(ui: &mut Ui, props: &DesktopLyricsProps<'_>, full: Rect, t: f32, col: &Colors, actions: &mut Vec<DesktopLyricsAction>) {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let compact = full.width() < 330.0;
    let (icon, pad, spacing) = (15.0, 6.5, 2.0);
    let side = icon + pad * 2.0;
    let n = if compact { 6.0 } else { 8.0 };
    let groups = if compact { 1.0 } else { 2.0 };
    let sep = 9.0;
    let row_w = n * side + (n - 1.0) * spacing + groups * sep;
    let row = Rect::from_center_size(pos2(full.center().x, full.top() + 6.0 + CONTROLS_H / 2.0), vec2(row_w, CONTROLS_H));
    let pill = row.expand2(vec2(8.0, 1.0));
    let painter = ui.painter_at(full);
    painter.rect_filled(pill, CONTROLS_H / 2.0 + 1.0, theme::with_alpha(col.bg, 0.94 * t));

    let mut cui = ui.new_child(UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(Align::Center)));
    cui.multiply_opacity(t);
    cui.spacing_mut().item_spacing.x = spacing;
    let style = IconButtonStyle { color: Some(col.text), hover_fill: Some(pal.hover), padding: Some(pad), ..Default::default() };
    let enabled = IconButtonStyle { enabled: props.has_track, ..style };
    let mut button = |ui: &mut Ui, icon_kind: Icon, st: IconButtonStyle, tip: &str, action: DesktopLyricsAction| {
        if icon_button_ex(ui, icon_kind, icon, st, tip).clicked() {
            actions.push(action);
        }
    };
    let separator = |ui: &mut Ui| {
        let (r, _) = ui.allocate_exact_size(vec2(sep - spacing, CONTROLS_H), Sense::hover());
        ui.painter().vline(r.center().x, (r.center().y - 7.0)..=(r.center().y + 7.0), Stroke::new(1.0, theme::with_alpha(pal.border_strong, t)));
    };
    button(&mut cui, Icon::Previous, enabled, tr.t("transport.previous"), DesktopLyricsAction::Previous);
    let (play_icon, play_tip) = if props.playing { (Icon::Pause, tr.t("transport.pause")) } else { (Icon::Play, tr.t("transport.play")) };
    button(&mut cui, play_icon, style, play_tip, DesktopLyricsAction::PlayPause);
    button(&mut cui, Icon::Next, enabled, tr.t("transport.next"), DesktopLyricsAction::Next);
    separator(&mut cui);
    if !compact {
        button(&mut cui, Icon::TextSmaller, style, tr.t("lyrics.font_smaller"), DesktopLyricsAction::FontSmaller);
        button(&mut cui, Icon::TextLarger, style, tr.t("lyrics.font_larger"), DesktopLyricsAction::FontLarger);
        separator(&mut cui);
    }
    button(&mut cui, Icon::Lock, style, tr.t("desktop_lyrics.lock"), DesktopLyricsAction::Lock);
    button(&mut cui, Icon::OpenWindow, style, tr.t("desktop_lyrics.open_main"), DesktopLyricsAction::OpenMain);
    button(&mut cui, Icon::Close, style, tr.t("desktop_lyrics.close"), DesktopLyricsAction::Close);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Context, Event, PointerButton, RawInput};

    fn props(content: DesktopLyricsContent<'_>) -> DesktopLyricsProps<'_> {
        DesktopLyricsProps {
            content,
            title: "夜に駆ける",
            artist: "YOASOBI",
            font_size: 32.0,
            opacity: 0.7,
            show_next_line: true,
            show_title: false,
            locked: false,
            playing: true,
            has_track: true,
            force_controls: false,
            tr: Tr::JA,
        }
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
        let line = OverlayLine { text: "沈むように溶けてゆくように", karaoke: true, highlighted_chars: 4.0 };
        let p = props(DesktopLyricsContent::Lyrics { current: Some(line), intro: false, next: Some("次"), line_key: 1, lang: TextLang::Ja });
        // Hover: the controls fade in (animation needs a few frames).
        for _ in 0..20 {
            run(&ctx, vec![Event::PointerMoved(pos2(320.0, 70.0))], &p, &mut st);
        }
        // The close button is the right-most control of the row centered at the top.
        let side = 15.0 + 13.0;
        let row_w = 8.0 * side + 7.0 * 2.0 + 2.0 * 9.0;
        let close = pos2(320.0 + row_w / 2.0 - side / 2.0, 6.0 + CONTROLS_H / 2.0);
        run(&ctx, vec![Event::PointerMoved(close)], &p, &mut st);
        let mut got = Vec::new();
        for pressed in [true, false] {
            got.extend(run(&ctx, vec![Event::PointerButton { pos: close, button: PointerButton::Primary, pressed, modifiers: Default::default() }], &p, &mut st));
        }
        assert_eq!(got, vec![DesktopLyricsAction::Close]);
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
