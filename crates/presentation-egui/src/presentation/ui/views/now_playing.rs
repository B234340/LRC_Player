//! Now Playing page: large artwork over a blurred, darkened artwork backdrop, track info and
//! the synced lyrics column.

use eframe::egui::{
    self, pos2, vec2, Align, Color32, ColorImage, CursorIcon, FontId, Layout, Rect,
    RichText, Sense, Ui, UiBuilder, Vec2,
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

/// Draw the page filling the available rect.
pub fn now_playing(ui: &mut Ui, props: &NowPlayingProps<'_>, state: &mut NowPlayingState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    paint_backdrop(ui, full, props.backdrop, props.seed, theme::radius::LG);
    let tr = props.tr;

    if !props.has_track {
        let c = full.center();
        let r = Rect::from_center_size(c - vec2(0.0, 30.0), Vec2::splat(64.0));
        ui.painter().circle_filled(r.center(), 40.0, Color32::from_white_alpha(18));
        paint_icon(ui.painter(), r.shrink(14.0), Icon::Note, Color32::from_white_alpha(160));
        let g = ui.painter().layout_no_wrap(tr.t("now.no_track").to_owned(), theme::bold_font(ui.ctx(), 16.0), Color32::WHITE);
        ui.painter().galley(pos2(c.x - g.size().x / 2.0, r.bottom() + 24.0), g, Color32::WHITE);
        return actions;
    }

    let margin = 32.0;
    let inner = full.shrink(margin);
    let wide = inner.width() >= 820.0;

    if wide {
        let col_w = (inner.width() * 0.42).clamp(300.0, 520.0);
        let left = Rect::from_min_size(inner.min, vec2(col_w, inner.height()));
        let right = Rect::from_min_max(pos2(left.right() + 40.0, inner.top()), inner.max);
        let art = (col_w - 24.0).min(inner.height() - 150.0).clamp(120.0, 460.0);
        let block_h = art + 130.0;
        let top = left.top() + ((left.height() - block_h) * 0.45).max(0.0);
        let art_rect = Rect::from_min_size(pos2(left.center().x - art / 2.0, top), Vec2::splat(art));
        track_info(ui, props, art_rect, col_w, &mut actions, Align::Center);
        lyrics_column(ui, props, state, right, false, &mut actions);
    } else {
        let art = (inner.width() * 0.5).min(inner.height() * 0.32).clamp(96.0, 260.0);
        let art_rect = Rect::from_min_size(pos2(inner.center().x - art / 2.0, inner.top()), Vec2::splat(art));
        track_info(ui, props, art_rect, inner.width(), &mut actions, Align::Center);
        let lyr = Rect::from_min_max(pos2(inner.left(), art_rect.bottom() + 120.0), inner.max);
        lyrics_column(ui, props, state, lyr, true, &mut actions);
    }
    actions
}

fn track_info(ui: &mut Ui, props: &NowPlayingProps<'_>, art_rect: Rect, width: f32, actions: &mut Vec<UiAction>, align: Align) {
    paint_shadow(ui.painter(), art_rect, 14, 40, 140);
    paint_artwork(ui, art_rect, props.artwork, 14, props.seed);
    let text_rect = Rect::from_min_size(
        pos2(art_rect.center().x - width / 2.0, art_rect.bottom() + 22.0),
        vec2(width, 120.0),
    );
    let layout = Layout::top_down(align);
    let mut tui = ui.new_child(UiBuilder::new().max_rect(text_rect).layout(layout));
    tui.spacing_mut().item_spacing.y = 4.0;
    tui.add(
        egui::Label::new(
            RichText::new(props.title)
                .font(theme::bold_font(tui.ctx(), 24.0))
                .color(Color32::WHITE),
        )
        .truncate(),
    );
    let link = |ui: &mut Ui, text: &str, size: f32, alpha: u8| -> bool {
        if text.is_empty() {
            return false;
        }
        let resp = ui
            .add(egui::Label::new(RichText::new(text).font(FontId::proportional(size)).color(Color32::from_white_alpha(alpha))).truncate().sense(Sense::click()))
            .on_hover_cursor(CursorIcon::PointingHand);
        if resp.hovered() {
            let r = resp.rect;
            ui.painter().hline(r.x_range(), r.bottom() - 1.0, egui::Stroke::new(1.0, Color32::from_white_alpha(alpha)));
        }
        resp.clicked()
    };
    if link(&mut tui, props.artist, 16.0, 220) {
        actions.push(UiAction::ShowArtist(props.artist.to_owned()));
    }
    if link(&mut tui, props.album, 13.5, 150) {
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
    let tr = props.tr;
    match &props.lyrics {
        Some(l) if !l.lines.is_empty() => {
            let mut lui = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
            if centered {
                // Stacked layout: center the lyrics under the centered artwork.
                let l = LyricsProps { align: Align::Center, ..*l };
                actions.extend(lyrics_view(&mut lui, &l, &mut state.lyrics));
            } else {
                actions.extend(lyrics_view(&mut lui, l, &mut state.lyrics));
            }
        }
        _ => {
            let c = rect.center();
            let ir = Rect::from_center_size(c - vec2(0.0, 36.0), Vec2::splat(30.0));
            paint_icon(ui.painter(), ir, Icon::Lyrics, Color32::from_white_alpha(120));
            let g = ui.painter().layout_no_wrap(tr.t("now.no_lyrics").to_owned(), theme::bold_font(ui.ctx(), 16.0), Color32::from_white_alpha(220));
            ui.painter().galley(pos2(c.x - g.size().x / 2.0, c.y - 8.0), g, Color32::WHITE);
            let hint = ui.painter().layout(
                tr.t("now.no_lyrics_hint").to_owned(),
                FontId::proportional(12.5),
                Color32::from_white_alpha(140),
                rect.width().min(360.0),
            );
            ui.painter().galley(pos2(c.x - hint.size().x / 2.0, c.y + 18.0), hint, Color32::WHITE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
