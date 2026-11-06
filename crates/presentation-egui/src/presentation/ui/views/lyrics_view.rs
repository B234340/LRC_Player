//! Synced lyrics: current line large and bright, neighbors smaller and dimmer, smooth
//! (eased) scrolling, karaoke left-to-right fill, click-to-seek, wheel to detach
//! auto-follow, offset and font-size controls.
//!
//! Galleys are laid out once per (lyrics revision, width, font size, pixels-per-point) and
//! cached; font sizes are never animated per frame.

use std::sync::Arc;

use eframe::egui::{
    self, pos2, text::LayoutJob, vec2, Align, Color32, CursorIcon, FontId, Galley, Rect,
    Sense, Stroke, Ui, UiBuilder, Vec2,
};

use crate::presentation::ui::{
    actions::UiAction,
    format::format_offset,
    i18n::Tr,
    icons::{icon_button_ex, paint_icon, Icon, IconButtonStyle},
    text, theme,
    widgets::slider::wheel_notches,
};

/// One lyric line (borrowed from the app's timeline).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LyricLine<'a> {
    pub text: &'a str,
    /// Lyric time (before offsets).
    pub start_ms: u64,
    /// The line has char/word timings (karaoke fill applies).
    pub karaoke: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct LyricsProps<'a> {
    pub lines: &'a [LyricLine<'a>],
    /// Identity of the loaded lyrics (bump on reload / offset-independent).
    pub revision: u64,
    /// Index of the active line.
    pub current: Option<usize>,
    /// Number of highlighted chars of the current line (fractional for smooth wipes).
    /// Use `LyricPosition::highlighted_chars`; ignored for non-karaoke lines.
    pub highlighted_chars: f32,
    /// Total user offset, displayed in the controls.
    pub offset_ms: i64,
    /// Size of the active line in points (others use ~72 %).
    pub font_size: f32,
    pub align: Align,
    /// White-on-artwork colors (Now Playing) instead of theme colors.
    pub on_backdrop: bool,
    pub show_controls: bool,
    /// Enable the "Save to file" button.
    pub can_save_offset: bool,
    pub tr: Tr,
}

struct LineGalleys {
    small: Arc<Galley>,
    big: Arc<Galley>,
}

#[derive(Default)]
pub struct LyricsViewState {
    follow: bool,
    initialized: bool,
    /// Eased scroll position (content y placed at the focus line).
    scroll: f32,
    /// Target when detached.
    manual: f32,
    last_time: Option<f64>,
    last_current: Option<usize>,
    /// (revision, lines, wrap, size, ppp, align, `text::epoch()`): a line's font family is a
    /// pure function of its text and the epoch.
    cache_key: Option<(u64, usize, u32, u32, u32, u8, u64)>,
    cache: Vec<LineGalleys>,
    /// Re-requested every frame to detect font-atlas rebuilds (see `lyrics_view`).
    probe: Option<Arc<Galley>>,
}

impl std::fmt::Debug for LyricsViewState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LyricsViewState")
            .field("follow", &self.follow)
            .field("scroll", &self.scroll)
            .finish_non_exhaustive()
    }
}

impl LyricsViewState {
    /// Whether the view is following playback.
    pub fn is_following(&self) -> bool {
        self.follow || !self.initialized
    }

    /// Re-attach to the current line.
    pub fn follow_current(&mut self) {
        self.follow = true;
    }
}

const SMALL_RATIO: f32 = 0.72;
const CONTROLS_H: f32 = 40.0;

fn line_text(t: &str) -> String {
    if t.trim().is_empty() {
        "♪".to_owned()
    } else {
        t.trim_end().to_owned()
    }
}

/// Each line is drawn entirely in the font of its own language, decided per line
/// (`text::lang_in_context`; `context` is the song's dominant language, so an ambiguous
/// all-kanji line in a Chinese song stays Chinese), so a line never mixes glyph forms.
fn layout_line(ui: &Ui, text: &str, context: text::TextLang, size: f32, wrap: f32, align: Align) -> Arc<Galley> {
    let shown = line_text(text);
    let font = FontId::new(size, text::family_in_context(&shown, context, true));
    let mut job = LayoutJob::simple(shown, font, Color32::PLACEHOLDER, wrap);
    job.halign = align;
    // A little extra line spacing for wrapped rows.
    if let Some(s) = job.sections.first_mut() {
        s.format.line_height = Some(size * 1.32);
    }
    ui.fonts_mut(|f| f.layout_job(job))
}

/// Draw the lyrics view filling the available rect.
pub fn lyrics_view(ui: &mut Ui, props: &LyricsProps<'_>, state: &mut LyricsViewState) -> Vec<UiAction> {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let mut actions = Vec::new();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let id = ui.id().with("lyrics_view");
    if !state.initialized {
        state.follow = true;
        state.initialized = true;
    }

    let controls_h = if props.show_controls { CONTROLS_H } else { 0.0 };
    let area = Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - controls_h));
    let pad_x = 20.0;
    let wrap = (area.width() - pad_x * 2.0).max(60.0);
    let big_size = props.font_size.clamp(12.0, 64.0);
    let small_size = (big_size * SMALL_RATIO).round();

    let (bright, dim, hover_c) = if props.on_backdrop {
        (Color32::WHITE, Color32::from_white_alpha(92), Color32::from_white_alpha(170))
    } else {
        (pal.text, theme::with_alpha(pal.text_muted, 0.62), pal.text)
    };

    // ---- galley cache ----
    let align_code = match props.align {
        Align::Min => 0,
        Align::Center => 1,
        Align::Max => 2,
    };
    let key = (
        props.revision,
        props.lines.len(),
        wrap.round() as u32,
        big_size.to_bits(),
        ui.ctx().pixels_per_point().to_bits(),
        align_code,
        text::epoch(),
    );
    // Cached galleys hold UVs into the font atlas. egui rebuilds the atlas (and clears its own
    // galley cache) when fonts change or the atlas fills up; detect that with a tiny probe
    // galley requested every frame: egui hands back the same Arc while its cache is intact.
    let probe = layout_line(ui, "♪", text::TextLang::Latin, small_size, wrap, props.align);
    if state.probe.as_ref().is_none_or(|p| !Arc::ptr_eq(p, &probe)) {
        state.cache_key = None;
    }
    state.probe = Some(probe);
    if state.cache_key != Some(key) {
        let context = text::dominant_lang(props.lines.iter().map(|l| l.text));
        state.cache = props
            .lines
            .iter()
            .map(|l| LineGalleys {
                small: layout_line(ui, l.text, context, small_size, wrap, props.align),
                big: layout_line(ui, l.text, context, big_size, wrap, props.align),
            })
            .collect();
        state.cache_key = Some(key);
    }

    // ---- line positions (content coordinates) ----
    let gap = small_size * 0.85;
    let current = props.current.filter(|&c| c < state.cache.len());
    let mut tops = Vec::with_capacity(state.cache.len());
    let mut y = 0.0;
    for (i, g) in state.cache.iter().enumerate() {
        tops.push(y);
        let h = if Some(i) == current { g.big.size().y } else { g.small.size().y };
        y += h + gap;
    }
    let content_h = y;
    let focus_y = area.top() + area.height() * 0.40;
    let target_follow = match current {
        Some(c) => tops[c] + state.cache[c].big.size().y / 2.0,
        None => 0.0,
    };

    // ---- wheel detaches auto-follow ----
    let hovered = ui.rect_contains_pointer(area);
    if hovered {
        let notches = wheel_notches(ui);
        if notches != 0.0 {
            if state.follow {
                state.manual = state.scroll;
            }
            state.follow = false;
            state.manual = (state.manual - notches * 60.0).clamp(0.0, content_h.max(0.0));
        }
    }
    if current != state.last_current && state.last_current.is_none() {
        // first line appears: jump without animation
        state.scroll = target_follow;
    }
    state.last_current = current;
    let target = if state.follow { target_follow } else { state.manual };

    // eased scrolling (critically damped-ish exponential approach)
    let now = ui.input(|i| i.time);
    let dt = state.last_time.map(|t| (now - t) as f32).unwrap_or(0.0).clamp(0.0, 0.1);
    state.last_time = Some(now);
    let k = 1.0 - (-dt * 9.0).exp();
    state.scroll += (target - state.scroll) * k;
    if (target - state.scroll).abs() < 0.3 {
        state.scroll = target;
    } else {
        ui.ctx().request_repaint();
    }

    // ---- paint lines ----
    // Lines live in `lines_rect` (a little above the controls' separator) and fade out by their
    // far edge, so no line is ever cut hard at the top or by the controls bar.
    let lines_rect = Rect::from_min_max(area.min, pos2(area.right(), area.bottom() - if props.show_controls { 6.0 } else { 0.0 }));
    let painter = ui.painter_at(lines_rect);
    let fade = 48.0f32.min(lines_rect.height() * 0.2).max(1.0);
    for (i, g) in state.cache.iter().enumerate() {
        let is_cur = Some(i) == current;
        let galley = if is_cur { &g.big } else { &g.small };
        let top = focus_y - state.scroll + tops[i];
        let anchor_x = match props.align {
            Align::Min => area.left() + pad_x,
            Align::Center => area.center().x,
            Align::Max => area.right() - pad_x,
        };
        let pos = pos2(anchor_x, top);
        let rect = galley.rect.translate(pos.to_vec2());
        // edge fade: fully transparent once the line's far edge reaches the boundary
        let edge = (rect.top() - lines_rect.top()).min(lines_rect.bottom() - rect.bottom());
        let edge_a = (edge / fade).clamp(0.0, 1.0);
        if edge_a <= 0.0 {
            continue;
        }

        let hit = rect.expand2(vec2(10.0, gap * 0.4));
        let resp = ui
            .interact(hit, id.with(("line", i)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if resp.hovered() && !is_cur {
            painter.rect_filled(hit, 8.0, if props.on_backdrop { Color32::from_white_alpha(14) } else { theme::with_alpha(pal.hover, 0.7) });
        }
        if resp.clicked() {
            actions.push(UiAction::SeekToLyricLine { index: i, start_ms: props.lines[i].start_ms });
            state.follow = true;
        }

        let base = if is_cur { bright } else if resp.hovered() { hover_c } else { dim };
        if is_cur && props.lines[i].karaoke {
            paint_karaoke(&painter, pos, galley, props.highlighted_chars, dim, bright, edge_a);
        } else {
            painter.galley(pos, galley.clone(), theme::with_alpha(base, edge_a));
        }
    }

    // ---- back to current ----
    if !state.follow {
        let label = tr.t("lyrics.back_to_current");
        let g = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(12.5), Color32::PLACEHOLDER);
        let size = vec2(g.size().x + 44.0, 32.0);
        let r = Rect::from_center_size(pos2(area.center().x, area.bottom() - 28.0), size);
        let resp = ui
            .interact(r, id.with("back"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let fill = if resp.hovered() { pal.accent_hover } else { pal.accent };
        ui.painter().add(egui::epaint::Shadow { offset: [0, 4], blur: 14, spread: 0, color: pal.shadow }.as_shape(r, 16));
        ui.painter().rect_filled(r, 16.0, fill);
        paint_icon(ui.painter(), Rect::from_center_size(pos2(r.left() + 18.0, r.center().y), Vec2::splat(14.0)), Icon::ArrowDownToLine, pal.accent_text);
        ui.painter().galley(pos2(r.left() + 32.0, r.center().y - g.size().y / 2.0), g, pal.accent_text);
        if resp.clicked() {
            state.follow = true;
        }
    }

    // ---- controls ----
    if props.show_controls {
        let bar = Rect::from_min_max(pos2(full.left() + 8.0, full.bottom() - controls_h), full.max - vec2(8.0, 0.0));
        controls(ui, props, bar, &mut actions);
    }
    actions
}

/// Paint `galley` dimmed, then bright up to `chars` (fractional), row by row, with a soft
/// leading edge.
fn paint_karaoke(painter: &egui::Painter, pos: egui::Pos2, galley: &Arc<Galley>, chars: f32, dim: Color32, bright: Color32, alpha: f32) {
    painter.galley(pos, galley.clone(), theme::with_alpha(dim, alpha));
    let mut remaining = chars.max(0.0);
    for placed in &galley.rows {
        if remaining <= 0.0 {
            break;
        }
        let glyphs = &placed.row.glyphs;
        let row_rect = placed.rect().translate(pos.to_vec2());
        let n = glyphs.len() as f32;
        let fill_x = if remaining >= n {
            row_rect.right()
        } else {
            let k = remaining.floor() as usize;
            let frac = remaining - k as f32;
            let gl = &glyphs[k];
            pos.x + placed.pos.x + gl.pos.x + gl.advance_width * frac
        };
        let full_row = remaining >= n;
        remaining -= n + if placed.ends_with_newline { 1.0 } else { 0.0 };
        let clip_y = row_rect.y_range();
        let soft = 10.0;
        let solid_right = if full_row { fill_x } else { fill_x - soft };
        if solid_right > row_rect.left() {
            let clip = Rect::from_x_y_ranges(row_rect.left() - 2.0..=solid_right, clip_y);
            painter.with_clip_rect(clip.intersect(painter.clip_rect())).galley(pos, galley.clone(), theme::with_alpha(bright, alpha));
        }
        if !full_row {
            // soft edge in 4 slices
            for s in 0..4 {
                let x0 = (fill_x - soft + soft * s as f32 / 4.0).max(row_rect.left() - 2.0);
                let x1 = fill_x - soft + soft * (s + 1) as f32 / 4.0;
                if x1 <= x0 {
                    continue;
                }
                let a = 1.0 - (s as f32 + 0.5) / 4.0;
                let clip = Rect::from_x_y_ranges(x0..=x1, clip_y);
                let c = theme::blend(dim, bright, a);
                painter.with_clip_rect(clip.intersect(painter.clip_rect())).galley(pos, galley.clone(), theme::with_alpha(c, alpha));
            }
        }
    }
}

fn controls(ui: &mut Ui, props: &LyricsProps<'_>, bar: Rect, actions: &mut Vec<UiAction>) {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let (fg, hover) = if props.on_backdrop {
        (Color32::from_white_alpha(200), Color32::from_white_alpha(28))
    } else {
        (pal.text_muted, pal.hover)
    };
    let style = IconButtonStyle { color: Some(fg), hover_fill: Some(hover), padding: Some(7.0), ..Default::default() };
    let shift = ui.input(|i| i.modifiers.shift);
    let step = if shift { 500 } else { 100 };
    let mut cui = ui.new_child(UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(Align::Center)));
    cui.spacing_mut().item_spacing.x = 2.0;
    // offset group
    if icon_button_ex(&mut cui, Icon::Minus, 13.0, style, tr.t("lyrics.offset_minus")).clicked() {
        actions.push(UiAction::LyricOffsetDelta(-step));
    }
    let label = format_offset(props.offset_ms);
    let g = cui.painter().layout_no_wrap(label, FontId::proportional(12.0), Color32::PLACEHOLDER);
    let (r, resp) = cui.allocate_exact_size(vec2(g.size().x.max(58.0) + 8.0, 26.0), Sense::hover());
    let text_c = if props.offset_ms != 0 { if props.on_backdrop { Color32::WHITE } else { pal.accent_fg } } else { fg };
    cui.painter().galley(r.center() - g.size() / 2.0, g, text_c);
    resp.on_hover_text(format!("{} — {}", tr.t("lyrics.offset"), tr.t("lyrics.shift_hint")));
    if icon_button_ex(&mut cui, Icon::Plus, 13.0, style, tr.t("lyrics.offset_plus")).clicked() {
        actions.push(UiAction::LyricOffsetDelta(step));
    }
    let enabled = props.offset_ms != 0;
    if icon_button_ex(&mut cui, Icon::Undo, 14.0, IconButtonStyle { enabled, ..style }, tr.t("lyrics.offset_reset")).clicked() {
        actions.push(UiAction::LyricOffsetReset);
    }
    if icon_button_ex(&mut cui, Icon::Save, 14.0, IconButtonStyle { enabled: props.can_save_offset && enabled, ..style }, tr.t("lyrics.offset_save")).clicked() {
        actions.push(UiAction::SaveLyricOffset);
    }
    // font size group, right-aligned
    cui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
        if icon_button_ex(ui, Icon::TextLarger, 16.0, style, tr.t("lyrics.font_larger")).clicked() {
            actions.push(UiAction::SetLyricFontSize((props.font_size + 2.0).min(56.0)));
        }
        if icon_button_ex(ui, Icon::TextSmaller, 16.0, style, tr.t("lyrics.font_smaller")).clicked() {
            actions.push(UiAction::SetLyricFontSize((props.font_size - 2.0).max(14.0)));
        }
    });
    let sep_c = if props.on_backdrop { Color32::from_white_alpha(30) } else { pal.border };
    ui.painter().hline(bar.x_range(), bar.top() + 0.5, Stroke::new(1.0, sep_c));
}
