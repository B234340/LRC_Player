//! Now Playing page: large artwork over a blurred, darkened artwork backdrop, track info and
//! the synced lyrics column. The artwork scales with the space the page gets (two columns
//! when the lyrics keep ≥ 360 px, stacked otherwise) and opens a full-window view on click.

use eframe::egui::{
    self, pos2, vec2, Align, Color32, ColorImage, CursorIcon, FontId, Key, Layout, Modifiers, Order,
    Rect, Sense, Ui, UiBuilder, Vec2,
};

use super::lyrics_view::{lyrics_view, LyricsProps, LyricsViewState};
use crate::presentation::ui::{
    actions::UiAction,
    i18n::Tr,
    icons::{paint_icon, Icon},
    theme,
    widgets::{
        artwork::{gradient_rounded_rect, gradient_rounded_rect_with, paint_shadow, seed_hue},
        cover_uv, paint_artwork, Tex,
    },
};

pub struct NowPlayingProps<'a> {
    pub has_track: bool,
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub artwork: Option<Tex>,
    /// Texture made with [`make_backdrop`] (upload with `TextureOptions::LINEAR`).
    pub backdrop: Option<Tex>,
    /// Placeholder / fallback-background seed (album name).
    pub seed: &'a str,
    /// `None` when the track has no lyrics.
    pub lyrics: Option<LyricsProps<'a>>,
    pub tr: Tr,
}

#[derive(Debug, Default)]
pub struct NowPlayingState {
    pub lyrics: LyricsViewState,
    /// Space to keep free under the artwork column (set by the page, e.g. for the spectrum).
    pub bottom_reserve: f32,
    /// Layout of the last frame (`None` without a track); the page places extras with it.
    pub layout: Option<NowPlayingLayout>,
    /// Largest artwork side shown in the last frame, in physical pixels (cover or the
    /// enlarged view); the page asks the loader for a sharper texture when it exceeds 1024.
    pub wanted_art_px: f32,
    /// The enlarged artwork view is open.
    pub overlay_open: bool,
    /// Bumped when the overlay is closed without animation (page left, track changed).
    overlay_gen: u32,
    last_pass: u64,
}

impl NowPlayingState {
    fn close_overlay(&mut self) {
        self.overlay_open = false;
        self.overlay_gen = self.overlay_gen.wrapping_add(1);
    }
}

/// Build the backdrop image: downscale to ≤ `48` px, blur, boost saturation, darken.
/// Pure CPU, allocation-light (~1 ms for a 1000² cover); run it on a worker thread once
/// per track and upload with `TextureOptions::LINEAR` so the GPU upscale stays smooth.
pub fn make_backdrop(image: &ColorImage) -> ColorImage {
    const TARGET: usize = 48;
    let [w, h] = image.size;
    if w == 0 || h == 0 {
        return ColorImage::new([1, 1], vec![Color32::from_gray(20)]);
    }
    let scale = (w.max(h) as f32 / TARGET as f32).max(1.0);
    let tw = ((w as f32 / scale).round() as usize).max(1);
    let th = ((h as f32 / scale).round() as usize).max(1);
    // area-average downscale
    let mut buf = vec![[0.0f32; 3]; tw * th];
    for ty in 0..th {
        let y0 = ty * h / th;
        let y1 = ((ty + 1) * h / th).max(y0 + 1);
        for tx in 0..tw {
            let x0 = tx * w / tw;
            let x1 = ((tx + 1) * w / tw).max(x0 + 1);
            let mut acc = [0.0f32; 3];
            // sample at most ~8x8 points per block to bound the cost on huge covers
            let sy = ((y1 - y0) / 8).max(1);
            let sx = ((x1 - x0) / 8).max(1);
            let mut n = 0.0;
            let mut y = y0;
            while y < y1 {
                let mut x = x0;
                while x < x1 {
                    let c = image.pixels[y * w + x];
                    acc[0] += c.r() as f32;
                    acc[1] += c.g() as f32;
                    acc[2] += c.b() as f32;
                    n += 1.0;
                    x += sx;
                }
                y += sy;
            }
            buf[ty * tw + tx] = [acc[0] / n, acc[1] / n, acc[2] / n];
        }
    }
    // separable box blur, 3 passes ≈ gaussian
    let radius = 4usize;
    let mut tmp = buf.clone();
    for _ in 0..3 {
        box_blur_h(&buf, &mut tmp, tw, th, radius);
        box_blur_v(&tmp, &mut buf, tw, th, radius);
    }
    let pixels = buf
        .iter()
        .map(|&[r, g, b]| {
            let l = 0.299 * r + 0.587 * g + 0.114 * b;
            let sat = 1.25;
            let f = |c: f32| ((l + (c - l) * sat) * 0.62).clamp(0.0, 255.0) as u8;
            Color32::from_rgb(f(r), f(g), f(b))
        })
        .collect();
    ColorImage::new([tw, th], pixels)
}

fn box_blur_h(src: &[[f32; 3]], dst: &mut [[f32; 3]], w: usize, h: usize, r: usize) {
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0; 3];
            let lo = x.saturating_sub(r);
            let hi = (x + r).min(w - 1);
            for xx in lo..=hi {
                let p = src[y * w + xx];
                acc[0] += p[0];
                acc[1] += p[1];
                acc[2] += p[2];
            }
            let n = (hi - lo + 1) as f32;
            dst[y * w + x] = [acc[0] / n, acc[1] / n, acc[2] / n];
        }
    }
}

fn box_blur_v(src: &[[f32; 3]], dst: &mut [[f32; 3]], w: usize, h: usize, r: usize) {
    for y in 0..h {
        let lo = y.saturating_sub(r);
        let hi = (y + r).min(h - 1);
        for x in 0..w {
            let mut acc = [0.0; 3];
            for yy in lo..=hi {
                let p = src[yy * w + x];
                acc[0] += p[0];
                acc[1] += p[1];
                acc[2] += p[2];
            }
            let n = (hi - lo + 1) as f32;
            dst[y * w + x] = [acc[0] / n, acc[1] / n, acc[2] / n];
        }
    }
}

/// Paint the backdrop (or a seed-colored fallback) plus a readability scrim over `rect`
/// with rounded corners.
pub fn paint_backdrop(ui: &Ui, rect: Rect, backdrop: Option<Tex>, seed: &str, rounding: u8) {
    let painter = ui.painter_at(rect);
    match backdrop {
        Some(t) => {
            egui::Image::new((t.id, rect.size()))
                .uv(cover_uv(t.size, rect.size()))
                .corner_radius(rounding)
                .paint_at(ui, rect);
        }
        None => {
            let hue = seed_hue(seed);
            let c0: Color32 = egui::ecolor::Hsva::new(hue, 0.45, 0.30, 1.0).into();
            let c1: Color32 = egui::ecolor::Hsva::new((hue + 0.12) % 1.0, 0.40, 0.12, 1.0).into();
            gradient_rounded_rect(&painter, rect, rounding as f32, c0, c1);
        }
    }
    // Vertical scrim: a quad-strip mesh would not follow the rounded corners, so use the
    // rounded fan with a vertical color function (the fan centre sits on the midline).
    let h = rect.height().max(1.0);
    gradient_rounded_rect_with(&painter, rect, rounding as f32, |p| {
        let t = ((p.y - rect.top()) / h).clamp(0.0, 1.0);
        Color32::from_black_alpha((60.0 + 100.0 * t) as u8)
    });
}

/// Outer margin around the page content (inside the backdrop card).
const MARGIN: f32 = 32.0;
/// Gap between the artwork column and the lyrics column (wide layout).
const COL_GAP: f32 = 48.0;
/// Narrowest comfortable lyrics column (wide layout).
const LYRICS_MIN_W: f32 = 360.0;
/// Widest lyrics column; beyond it the two columns are centered as a group.
const LYRICS_MAX_W: f32 = 760.0;
/// Smallest artwork that still justifies the two-column layout.
const ART_MIN_WIDE: f32 = 220.0;
/// Largest artwork (logical px).
const ART_MAX: f32 = 680.0;
/// Share of the content width the artwork may take (wide layout).
const ART_WIDTH_SHARE: f32 = 0.5;
/// Space between the artwork and the title.
const INFO_GAP: f32 = 22.0;
/// Space between the album label and the lyrics (stacked layout).
const STACK_GAP: f32 = 28.0;
/// Lyrics height kept free in the stacked layout (≈ 3 lines + controls).
const LYRICS_MIN_H: f32 = 200.0;

/// Where the parts of the page go; computed from the space the page actually gets (the
/// sidebar and the right panel are already taken out), so opening the queue / lyrics panel
/// switches to the stacked layout instead of squeezing or overlapping the columns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NowPlayingLayout {
    /// Two columns (artwork + info | lyrics); otherwise stacked.
    pub wide: bool,
    /// Content area (page minus margins).
    pub inner: Rect,
    pub art: Rect,
    /// Title / artist / album block.
    pub info: Rect,
    /// Lyrics column; the lyrics are clipped to it.
    pub lyrics: Rect,
}

/// Pure layout: `info_h` is the height of the title / artist / album block, `bottom_reserve`
/// space kept free under the artwork column (spectrum strip).
///
/// Wide: `art = min(height − info − reserve, 50 % of width, width − gap − 360, 680)`; the
/// two-column layout is used while that leaves the artwork at least [`ART_MIN_WIDE`] wide.
pub fn compute_layout(full: Rect, info_h: f32, bottom_reserve: f32) -> NowPlayingLayout {
    let margin = if full.width() < 520.0 { 20.0 } else { MARGIN };
    let inner = full.shrink(margin);
    let (w, h) = (inner.width(), inner.height());
    let art_by_width = (w * ART_WIDTH_SHARE).min(w - COL_GAP - LYRICS_MIN_W);

    if art_by_width >= ART_MIN_WIDE {
        let avail_h = h - bottom_reserve - INFO_GAP - info_h;
        let art = art_by_width.min(avail_h).clamp(120.0, ART_MAX);
        let block_h = art + INFO_GAP + info_h;
        let top = inner.top() + ((h - bottom_reserve - block_h) * 0.45).max(0.0);
        let lyrics_w = (w - art - COL_GAP).min(LYRICS_MAX_W);
        let left = inner.left() + ((w - art - COL_GAP - lyrics_w) / 2.0).max(0.0);
        let art_rect = Rect::from_min_size(pos2(left, top), Vec2::splat(art));
        // Text may run a little wider than a small cover, never into the column gap.
        let info_w = art + COL_GAP * 0.5;
        let info = Rect::from_min_size(pos2(art_rect.center().x - info_w / 2.0, art_rect.bottom() + INFO_GAP), vec2(info_w, info_h));
        let lyrics = Rect::from_min_size(pos2(art_rect.right() + COL_GAP, inner.top()), vec2(lyrics_w, h));
        NowPlayingLayout { wide: true, inner, art: art_rect, info, lyrics }
    } else {
        let mut art = (w * 0.55).min(h * 0.36).min(300.0);
        // Keep room for a few lyric lines; shrink the artwork first.
        let left_for_lyrics = h - art - INFO_GAP - info_h - STACK_GAP;
        if left_for_lyrics < LYRICS_MIN_H {
            art -= LYRICS_MIN_H - left_for_lyrics;
        }
        let art = art.max(64.0);
        let art_rect = Rect::from_min_size(pos2(inner.center().x - art / 2.0, inner.top()), Vec2::splat(art));
        let info = Rect::from_min_size(pos2(inner.left(), art_rect.bottom() + INFO_GAP), vec2(w, info_h));
        let lyrics_top = (info.bottom() + STACK_GAP).min(inner.bottom());
        let lyrics = Rect::from_min_max(pos2(inner.left(), lyrics_top), inner.max);
        NowPlayingLayout { wide: false, inner, art: art_rect, info, lyrics }
    }
}

const TITLE_SIZE: f32 = 24.0;
const ARTIST_SIZE: f32 = 16.0;
const ALBUM_SIZE: f32 = 13.5;
const INFO_SPACING: f32 = 4.0;

/// Height of the title / artist / album block (empty artist / album lines are skipped).
fn info_height(ui: &Ui, props: &NowPlayingProps<'_>) -> f32 {
    let ctx = ui.ctx();
    let title = theme::bold_font(ctx, TITLE_SIZE);
    ctx.fonts_mut(|f| {
        let mut h = f.row_height(&title);
        for (text, size) in [(props.artist, ARTIST_SIZE), (props.album, ALBUM_SIZE)] {
            if !text.is_empty() {
                h += INFO_SPACING + f.row_height(&FontId::proportional(size));
            }
        }
        h
    })
}

/// Draw the page filling the available rect.
pub fn now_playing(ui: &mut Ui, props: &NowPlayingProps<'_>, state: &mut NowPlayingState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    paint_backdrop(ui, full, props.backdrop, props.seed, theme::radius::LG);
    let tr = props.tr;

    // The overlay never survives leaving the page (or the track losing its artwork).
    let pass = ui.ctx().cumulative_pass_nr();
    if state.overlay_open && (pass > state.last_pass + 1 || props.artwork.is_none() || !props.has_track) {
        state.close_overlay();
    }
    state.last_pass = pass;

    if !props.has_track {
        state.layout = None;
        state.wanted_art_px = 0.0;
        let c = full.center();
        let r = Rect::from_center_size(c - vec2(0.0, 30.0), Vec2::splat(64.0));
        ui.painter().circle_filled(r.center(), 40.0, Color32::from_white_alpha(18));
        paint_icon(ui.painter(), r.shrink(14.0), Icon::Note, Color32::from_white_alpha(160));
        let g = ui.painter().layout_no_wrap(tr.t("now.no_track").to_owned(), theme::bold_font(ui.ctx(), 16.0), Color32::WHITE);
        ui.painter().galley(pos2(c.x - g.size().x / 2.0, r.bottom() + 24.0), g, Color32::WHITE);
        return actions;
    }

    let layout = compute_layout(full, info_height(ui, props), state.bottom_reserve.max(0.0));
    state.layout = Some(layout);

    artwork_button(ui, props, state, layout.art);
    track_info(ui, props, layout.info, &mut actions);
    lyrics_column(ui, props, state, layout.lyrics, !layout.wide, &mut actions);

    let overlay_side = artwork_overlay(ui.ctx(), props, state);
    state.wanted_art_px = layout.art.width().max(overlay_side) * ui.ctx().pixels_per_point();
    actions
}

/// The cover; clicking it (when there is artwork) opens the large view.
fn artwork_button(ui: &mut Ui, props: &NowPlayingProps<'_>, state: &mut NowPlayingState, rect: Rect) {
    paint_shadow(ui.painter(), rect, 14, 40, 140);
    paint_artwork(ui, rect, props.artwork, 14, props.seed);
    if props.artwork.is_none() {
        return;
    }
    let resp = ui
        .interact(rect, ui.id().with("np_artwork"), Sense::click())
        .on_hover_cursor(CursorIcon::ZoomIn)
        .on_hover_text(props.tr.t("now.enlarge_artwork"));
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, 14u8, Color32::from_white_alpha((12.0 * hover) as u8));
    }
    if resp.clicked() {
        state.overlay_open = true;
    }
}

fn track_info(ui: &mut Ui, props: &NowPlayingProps<'_>, rect: Rect, actions: &mut Vec<UiAction>) {
    let mut tui = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Center)));
    tui.set_clip_rect(rect.expand(2.0).intersect(ui.clip_rect()));
    tui.spacing_mut().item_spacing.y = INFO_SPACING;
    tui.add(
        egui::Label::new(
            // Title, artist and album are separate elements, each in its language's font.
            crate::presentation::ui::text::rich(props.title, TITLE_SIZE, true).color(Color32::WHITE),
        )
        .truncate(),
    );
    let link = |ui: &mut Ui, text: &str, size: f32, alpha: u8| -> bool {
        if text.is_empty() {
            return false;
        }
        let resp = ui
            .add(egui::Label::new(crate::presentation::ui::text::rich(text, size, false).color(Color32::from_white_alpha(alpha))).truncate().sense(Sense::click()))
            .on_hover_cursor(CursorIcon::PointingHand);
        if resp.hovered() {
            let r = resp.rect;
            ui.painter().hline(r.x_range(), r.bottom() - 1.0, egui::Stroke::new(1.0, Color32::from_white_alpha(alpha)));
        }
        resp.clicked()
    };
    if link(&mut tui, props.artist, ARTIST_SIZE, 220) {
        actions.push(UiAction::ShowArtist(props.artist.to_owned()));
    }
    if link(&mut tui, props.album, ALBUM_SIZE, 150) {
        actions.push(UiAction::ShowAlbum { album: props.album.to_owned(), artist: props.artist.to_owned() });
    }
}

fn lyrics_column(
    ui: &mut Ui,
    props: &NowPlayingProps<'_>,
    state: &mut NowPlayingState,
    rect: Rect,
    centered: bool,
    actions: &mut Vec<UiAction>,
) {
    if rect.height() < 48.0 || rect.width() < 60.0 {
        return;
    }
    let tr = props.tr;
    match &props.lyrics {
        Some(l) if !l.lines.is_empty() => {
            let mut lui = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
            // Painting *and* hit-testing stay inside the column: partially visible lines
            // never cover the album link or the artwork.
            lui.set_clip_rect(rect.intersect(ui.clip_rect()));
            if centered {
                // Stacked layout: center the lyrics under the centered artwork.
                let l = LyricsProps { align: Align::Center, ..*l };
                actions.extend(lyrics_view(&mut lui, &l, &mut state.lyrics));
            } else {
                actions.extend(lyrics_view(&mut lui, l, &mut state.lyrics));
            }
        }
        _ => {
            let painter = ui.painter_at(rect);
            let c = rect.center();
            let ir = Rect::from_center_size(c - vec2(0.0, 36.0), Vec2::splat(30.0));
            paint_icon(&painter, ir, Icon::Lyrics, Color32::from_white_alpha(120));
            let g = painter.layout_no_wrap(tr.t("now.no_lyrics").to_owned(), theme::bold_font(ui.ctx(), 16.0), Color32::from_white_alpha(220));
            painter.galley(pos2(c.x - g.size().x / 2.0, c.y - 8.0), g, Color32::WHITE);
            let hint = painter.layout(
                tr.t("now.no_lyrics_hint").to_owned(),
                FontId::proportional(12.5),
                Color32::from_white_alpha(140),
                rect.width().min(360.0),
            );
            painter.galley(pos2(c.x - hint.size().x / 2.0, c.y + 18.0), hint, Color32::WHITE);
        }
    }
}

/// Large artwork view over the whole window: dimmed backdrop, the cover fitted to the
/// window, a short fade / scale-in. Esc, a click anywhere or the close button dismisses it.
/// Returns the displayed cover side (logical px, 0 when closed).
fn artwork_overlay(ctx: &egui::Context, props: &NowPlayingProps<'_>, state: &mut NowPlayingState) -> f32 {
    let anim_id = egui::Id::new("np_artwork_overlay").with(state.overlay_gen);
    if state.overlay_open && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        state.overlay_open = false;
    }
    let t = ctx.animate_bool_with_time(anim_id, state.overlay_open, 0.16);
    let Some(tex) = props.artwork.filter(|_| t > 0.0) else {
        return 0.0;
    };
    let screen = ctx.content_rect();
    let pad = (screen.width().min(screen.height()) * 0.06).clamp(16.0, 56.0);
    let caption_h = 56.0;
    let side = (screen.width() - pad * 2.0).min(screen.height() - pad * 2.0 - caption_h).max(64.0);
    let eased = 1.0 - (1.0 - t).powi(3);

    egui::Area::new(anim_id)
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_clip_rect(screen);
            let bg = ui.interact(screen, anim_id.with("bg"), Sense::click());
            let painter = ui.painter();
            painter.rect_filled(screen, 0u8, Color32::from_black_alpha((220.0 * t) as u8));

            let block_h = side + caption_h;
            let center = pos2(screen.center().x, screen.center().y - block_h / 2.0 + side / 2.0);
            let shown = side * (0.96 + 0.04 * eased);
            let art = Rect::from_center_size(center, Vec2::splat(shown));
            paint_shadow(painter, art, 12, 48, (150.0 * t) as u8);
            egui::Image::new((tex.id, art.size()))
                .uv(cover_uv(tex.size, art.size()))
                .corner_radius(12)
                .tint(Color32::from_white_alpha((255.0 * t) as u8))
                .paint_at(ui, art);

            // Caption: title, then artist · album, on a soft pill so whatever lies under
            // the dimmed layer (transport controls) never competes with it.
            let width = side.min(screen.width() - pad * 2.0);
            let title = painter.layout(
                props.title.to_owned(),
                theme::bold_font(ui.ctx(), 17.0),
                Color32::from_white_alpha((235.0 * t) as u8),
                width,
            );
            let sub = [props.artist, props.album].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ");
            let sub = (!sub.is_empty())
                .then(|| painter.layout(sub, FontId::proportional(13.0), Color32::from_white_alpha((170.0 * t) as u8), width));
            let text_w = title.size().x.max(sub.as_ref().map_or(0.0, |g| g.size().x));
            let text_h = title.size().y + sub.as_ref().map_or(0.0, |g| g.size().y + 2.0);
            let top = art.bottom() + 12.0;
            let pill = Rect::from_min_size(pos2(screen.center().x - text_w / 2.0, top), vec2(text_w, text_h)).expand2(vec2(14.0, 6.0));
            painter.rect_filled(pill, 12u8, Color32::from_black_alpha((110.0 * t) as u8));
            let mut y = top;
            painter.galley(pos2(screen.center().x - title.size().x / 2.0, y), title.clone(), Color32::WHITE);
            y += title.size().y + 2.0;
            if let Some(g) = sub {
                painter.galley(pos2(screen.center().x - g.size().x / 2.0, y), g, Color32::WHITE);
            }

            // Close button (any click closes; the button makes that discoverable).
            let close = Rect::from_center_size(screen.right_top() + vec2(-pad.max(28.0), pad.max(28.0)), Vec2::splat(36.0));
            let cr = ui
                .interact(close, anim_id.with("close"), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(props.tr.t("now.close_artwork"));
            let a = if cr.hovered() { 60.0 } else { 28.0 };
            painter.circle_filled(close.center(), 18.0, Color32::from_white_alpha((a * t) as u8));
            paint_icon(painter, close.shrink(10.0), Icon::Close, Color32::from_white_alpha((230.0 * t) as u8));

            if state.overlay_open && (bg.clicked() || cr.clicked()) {
                state.overlay_open = false;
            }
        });
    if state.overlay_open {
        side
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h))
    }

    #[test]
    fn wide_layout_scales_the_artwork() {
        // 1280×800 window, sidebar expanded, no right panel: ~1056×692 page.
        let l = compute_layout(page(1056.0, 692.0), 90.0, 0.0);
        assert!(l.wide);
        assert_eq!(l.art.width(), 496.0); // 50 % of the 992 px content width
        assert!(l.lyrics.width() >= LYRICS_MIN_W);
        assert!(l.lyrics.left() >= l.art.right() + COL_GAP - 0.01);
        assert!(l.info.bottom() <= l.inner.bottom());
        // Height-limited: art + info fill the column height.
        let l = compute_layout(page(1600.0, 500.0), 90.0, 0.0);
        assert!(l.wide);
        assert_eq!(l.art.width(), 500.0 - 64.0 - INFO_GAP - 90.0);
        // Large window: capped, columns centered as a group.
        let l = compute_layout(page(1700.0, 1000.0), 90.0, 48.0);
        assert_eq!(l.art.width(), ART_MAX);
        assert_eq!(l.lyrics.width(), LYRICS_MAX_W);
        let slack_l = l.art.left() - l.inner.left();
        assert!((slack_l - (l.inner.right() - l.lyrics.right())).abs() < 0.01);
        assert!(l.info.bottom() <= l.inner.bottom() - 48.0);
    }

    #[test]
    fn layout_follows_the_available_width() {
        // 1280×800 with the right panel open: still two columns, lyrics keep 360 px.
        let l = compute_layout(page(716.0, 692.0), 90.0, 0.0);
        assert!(l.wide);
        assert!(l.lyrics.width() >= LYRICS_MIN_W);
        assert!(l.art.width() >= ART_MIN_WIDE);
        // 1000×700 with the right panel open: stacked, no overlap.
        let l = compute_layout(page(436.0, 592.0), 90.0, 0.0);
        assert!(!l.wide);
        assert!(l.lyrics.top() >= l.info.bottom() + STACK_GAP - 0.01);
        assert!(l.lyrics.height() >= LYRICS_MIN_H);
        assert!(l.info.top() >= l.art.bottom());
        // Short and narrow: the artwork shrinks before the lyrics do.
        let l = compute_layout(page(420.0, 500.0), 90.0, 0.0);
        assert!(!l.wide);
        assert!(l.lyrics.height() >= LYRICS_MIN_H - 0.01, "{l:?}");
        assert!(l.art.width() < 160.0, "{l:?}");
    }

    #[test]
    fn backdrop_is_small_and_blurred() {
        let w = 400;
        let h = 300;
        let mut px = vec![Color32::BLACK; w * h];
        for y in 0..h {
            for x in w / 2..w {
                px[y * w + x] = Color32::WHITE;
            }
        }
        let img = ColorImage::new([w, h], px);
        let t = std::time::Instant::now();
        let out = make_backdrop(&img);
        assert!(t.elapsed().as_millis() < 500);
        assert_eq!(out.size[0], 48);
        assert_eq!(out.size[1], 36);
        // the hard edge is smoothed: the middle column is between black and white
        let mid = out.pixels[18 * 48 + 24].r();
        assert!(mid > 20 && mid < 150, "mid={mid}");
        assert!(make_backdrop(&ColorImage::new([0, 0], vec![])).size == [1, 1]);
    }
}
