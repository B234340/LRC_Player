//! Persistent transport bar: artwork + title/artist, prev/play/next, shuffle/repeat,
//! times + seek bar, lyrics/queue/EQ toggles and volume. Degrades gracefully when narrow.

use eframe::egui::{
    self, pos2, vec2, Align, CursorIcon, FontId, Id, Layout, Rect, RichText, Sense, Stroke, Ui,
    UiBuilder, Vec2,
};

use crate::presentation::ui::{
    actions::{Page, RepeatMode, RightPanel, UiAction},
    format::format_time,
    i18n::Tr,
    text,
    icons::{icon_button, icon_button_ex, play_button, Icon, IconButtonStyle},
    theme::{self, metrics},
    widgets::{artwork::paint_artwork, seek_bar, volume_control, Tex},
};

pub struct TransportProps<'a> {
    pub has_track: bool,
    pub title: &'a str,
    pub artist: &'a str,
    pub artwork: Option<Tex>,
    /// Seed for the placeholder gradient (album name works well).
    pub artwork_seed: &'a str,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    /// Linear gain `0..=1`.
    pub volume: f32,
    pub muted: bool,
    pub right_panel: Option<RightPanel>,
    pub eq_open: bool,
    pub eq_enabled: bool,
    /// Number of queued tracks (badge on the queue button).
    pub queue_len: usize,
    /// No audio output device is available: the volume area shows a quiet indicator.
    pub no_output: bool,
    /// Favorite state of the playing track (`None` hides the heart).
    pub favorite: Option<bool>,
    pub tr: Tr,
}

/// Small count pill at the top-right corner of an icon button.
fn count_badge(ui: &Ui, button: Rect, count: usize, pal: &theme::Palette) {
    let text = if count > 999 { "999+".to_owned() } else { count.to_string() };
    let g = ui.painter().layout_no_wrap(text, FontId::proportional(9.5), pal.accent_text);
    let w = (g.size().x + 8.0).max(15.0);
    let r = Rect::from_center_size(pos2(button.right() - 4.0, button.top() + 7.0), vec2(w, 15.0));
    // Ring in the bar color so the pill reads as separate from the icon.
    ui.painter().rect_filled(r.expand(1.5), 9.0, pal.bg);
    ui.painter().rect_filled(r, 7.5, pal.accent);
    ui.painter().galley(r.center() - g.size() / 2.0, g, pal.accent_text);
}

// ---------------------------------------------------------------------------------------
// Width budget
// ---------------------------------------------------------------------------------------

/// Horizontal padding of the bar and gap between its three regions.
const PAD: f32 = 14.0;
const REGION_GAP: f32 = 16.0;
/// Left: 52 px artwork + 12 px gap + at least 96 px of title / artist.
const LEFT_MIN: f32 = 52.0 + 12.0 + 96.0;
const LEFT_MAX: f32 = 340.0;
/// Center: shuffle, previous, play (36), next, repeat (34 each + gaps) = 204 …
const CONTROLS_ROW_W: f32 = 36.0 + 2.0 * 34.0 + 2.0 * 6.0 + 2.0 * (34.0 + 10.0);
/// … and the seek bar (≥ 160 px) with both times (40 px + 8 px gap each side) = 256.
const TIME_W: f32 = 40.0;
const SEEK_MIN: f32 = 160.0;
const CENTER_MIN: f32 = if CONTROLS_ROW_W > SEEK_MIN + 2.0 * (TIME_W + 8.0) { CONTROLS_ROW_W } else { SEEK_MIN + 2.0 * (TIME_W + 8.0) };
/// Right: lyrics, queue, EQ (≈ 31 px each, 2 px gaps), 6 px gap, mute (36) + volume
/// slider (≥ 64) + small slack = 216.
const SLIDER_MIN: f32 = 64.0;
const SLIDER_MAX: f32 = 96.0;
const RIGHT_FIXED: f32 = 3.0 * 31.0 + 3.0 * 2.0 + 6.0 + 36.0 + 2.0 + 9.0;
const RIGHT_MIN: f32 = RIGHT_FIXED + SLIDER_MIN;
const RIGHT_MAX: f32 = 330.0;

/// Narrowest bar that shows every control: `2 × 14 + 160 + 16 + 256 + 16 + 216 = 692`.
/// The bar spans the whole window (the sidebar sits above it), so this is also the
/// window's minimum width apart from the root margin (see `shell::MIN_WINDOW_WIDTH`).
pub const MIN_WIDTH_ALL_CONTROLS: f32 = 2.0 * PAD + LEFT_MIN + REGION_GAP + CENTER_MIN + REGION_GAP + RIGHT_MIN;

/// Region widths for a bar width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TransportLayout {
    pub left_w: f32,
    pub right_w: f32,
    pub slider_w: f32,
    /// Every control is shown (at or above [`MIN_WIDTH_ALL_CONTROLS`]).
    pub all_controls: bool,
    /// Below the minimum: queue / lyrics and a slim slider still fit (safety net).
    pub medium: bool,
}

pub(crate) fn transport_layout(w: f32) -> TransportLayout {
    if w >= MIN_WIDTH_ALL_CONTROLS {
        // Generous side regions on wide bars; on narrower ones the title area and the
        // volume slider shrink before the center (controls + seek bar) gets below its minimum.
        let mut left = (w * 0.27).clamp(LEFT_MIN, LEFT_MAX);
        let mut right = (w * 0.27).clamp(RIGHT_MIN, RIGHT_MAX);
        let center = |l: f32, r: f32| w - 2.0 * PAD - l - r - 2.0 * REGION_GAP;
        let deficit = CENTER_MIN - center(left, right);
        if deficit > 0.0 {
            let from_left = deficit.min(left - LEFT_MIN);
            left -= from_left;
            right -= (deficit - from_left).min(right - RIGHT_MIN);
        }
        let slider = (right - RIGHT_FIXED).clamp(SLIDER_MIN, SLIDER_MAX);
        TransportLayout { left_w: left, right_w: right, slider_w: slider, all_controls: true, medium: true }
    } else if w >= 560.0 {
        TransportLayout { left_w: 180.0, right_w: 150.0, slider_w: 64.0, all_controls: false, medium: true }
    } else {
        TransportLayout { left_w: 56.0, right_w: 40.0, slider_w: 0.0, all_controls: false, medium: false }
    }
}

/// Preferred height of the panel hosting the transport bar.
pub const HEIGHT: f32 = metrics::TRANSPORT_HEIGHT;

/// Draw the transport bar into `ui`'s max rect (give it [`HEIGHT`]).
pub fn transport_bar(ui: &mut Ui, props: &TransportProps<'_>) -> Vec<UiAction> {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let mut actions = Vec::new();
    let full = ui.max_rect();
    ui.painter().hline(full.x_range(), full.top() + 0.5, Stroke::new(1.0, pal.border));

    let w = full.width();
    let lay = transport_layout(w);
    let (wide, medium) = (lay.all_controls, lay.medium);
    let pad = PAD;
    let inner = full.shrink2(vec2(pad, 0.0));

    let (left_w, right_w) = (lay.left_w, lay.right_w);
    let left = Rect::from_min_size(inner.min, vec2(left_w, inner.height()));
    let right = Rect::from_min_max(pos2(inner.right() - right_w, inner.top()), inner.max);
    let center = Rect::from_min_max(pos2(left.right() + 16.0, inner.top()), pos2(right.left() - 16.0, inner.bottom()));

    // ---------------- left: artwork + text ----------------
    {
        let art = Rect::from_min_size(pos2(left.left(), left.center().y - 26.0), Vec2::splat(52.0));
        let art_resp = ui
            .interact(art, Id::new("transport_art"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(tr.t("transport.open_now_playing"));
        if props.has_track {
            paint_artwork(ui, art, props.artwork, 6, props.artwork_seed);
        } else {
            ui.painter().rect_filled(art, 6.0, pal.hover);
            crate::presentation::ui::icons::paint_icon(ui.painter(), art.shrink(16.0), Icon::Note, pal.text_faint);
        }
        if art_resp.hovered() {
            ui.painter().rect_filled(art, 6.0, egui::Color32::from_black_alpha(60));
        }
        if art_resp.clicked() {
            actions.push(UiAction::Navigate(Page::NowPlaying));
        }
        if left_w > 120.0 {
            let mut text_rect = Rect::from_min_max(pos2(art.right() + 12.0, left.top()), left.max);
            // Heart toggle after the title / artist (its slot is always reserved, so the text
            // does not re-truncate when the favorite state changes).
            if let (true, Some(favorite)) = (props.has_track, props.favorite) {
                const HEART: f32 = 30.0;
                text_rect.max.x -= HEART;
                // Right after the wider of title / artist (as truncated to the text area).
                let measure = |s: &str, size: f32, bold: bool| {
                    ui.fonts_mut(|f| f.layout_no_wrap(s.to_owned(), text::font_id(s, size, bold), pal.text).size().x)
                };
                let text_w = measure(props.title, theme::size::BODY, true)
                    .max(measure(props.artist, 12.5, false))
                    .min(text_rect.width());
                let heart = Rect::from_center_size(
                    pos2(text_rect.left() + text_w + HEART / 2.0 + 2.0, left.center().y),
                    Vec2::splat(28.0),
                );
                let colors = crate::presentation::ui::widgets::FavoriteColors::from_palette(&pal);
                if crate::presentation::ui::widgets::favorite_button(ui, heart, Id::new("transport_favorite"), favorite, colors, &tr)
                    .clicked()
                {
                    actions.push(UiAction::ToggleFavoriteCurrent);
                }
            }
            let mut tui = ui.new_child(UiBuilder::new().max_rect(text_rect).layout(Layout::top_down(Align::Min)));
            tui.spacing_mut().item_spacing.y = 1.0;
            tui.add_space((text_rect.height() - 38.0) / 2.0);
            if props.has_track {
                // Title and artist are separate elements, each in the font of its language.
                tui.add(egui::Label::new(text::rich(props.title, theme::size::BODY, true).color(pal.text)).truncate());
                tui.add(egui::Label::new(text::rich(props.artist, 12.5, false).color(pal.text_muted)).truncate());
            } else {
                tui.add_space(9.0);
                tui.add(egui::Label::new(RichText::new(tr.t("transport.nothing_playing")).color(pal.text_faint)).truncate());
            }
        }
    }

    // ---------------- center: controls + seek ----------------
    {
        let cw = center.width();
        let show_modes = cw >= CONTROLS_ROW_W;
        let mut row_w = 36.0 + 2.0 * 34.0 + 2.0 * 6.0;
        if show_modes {
            row_w += 2.0 * (34.0 + 10.0);
        }
        let row = Rect::from_center_size(pos2(center.center().x, center.top() + 26.0), vec2(row_w, 38.0));
        let mut rui = ui.new_child(UiBuilder::new().max_rect(row).layout(Layout::left_to_right(Align::Center)));
        rui.spacing_mut().item_spacing.x = 6.0;
        let enabled = props.has_track;
        if show_modes
            && icon_button(&mut rui, Icon::Shuffle, 17.0, props.shuffle, tr.t("transport.shuffle")).clicked()
        {
            actions.push(UiAction::ToggleShuffle);
        }
        if show_modes {
            rui.add_space(4.0);
        }
        if icon_button_ex(&mut rui, Icon::Previous, 18.0, IconButtonStyle { enabled, color: Some(pal.text), ..Default::default() }, tr.t("transport.previous")).clicked() {
            actions.push(UiAction::Previous);
        }
        let tip = if props.playing { tr.t("transport.pause") } else { tr.t("transport.play") };
        if play_button(&mut rui, props.playing, 36.0, tip).clicked() {
            actions.push(UiAction::PlayPause);
        }
        if icon_button_ex(&mut rui, Icon::Next, 18.0, IconButtonStyle { enabled, color: Some(pal.text), ..Default::default() }, tr.t("transport.next")).clicked() {
            actions.push(UiAction::Next);
        }
        if show_modes {
            rui.add_space(4.0);
            let (icon, key) = match props.repeat {
                RepeatMode::Off => (Icon::Repeat, "transport.repeat_off"),
                RepeatMode::All => (Icon::Repeat, "transport.repeat_all"),
                RepeatMode::One => (Icon::RepeatOne, "transport.repeat_one"),
            };
            if icon_button(&mut rui, icon, 17.0, props.repeat != RepeatMode::Off, tr.t(key)).clicked() {
                actions.push(UiAction::CycleRepeat);
            }
        }

        // seek row
        let time_w = if props.duration_ms >= 3_600_000 { 52.0 } else { TIME_W };
        let seek_y = center.bottom() - 20.0;
        let bar_w = (cw - 2.0 * (time_w + 8.0)).clamp(40.0, 620.0);
        let total = bar_w + 2.0 * (time_w + 8.0);
        let seek_row = Rect::from_center_size(pos2(center.center().x, seek_y), vec2(total, 18.0));
        let bar_rect = Rect::from_center_size(seek_row.center(), vec2(bar_w, 18.0));
        let mut bui = ui.new_child(UiBuilder::new().max_rect(bar_rect).layout(Layout::left_to_right(Align::Center)));
        let out = seek_bar(&mut bui, Id::new("transport_seek"), props.position_ms, props.duration_ms, bar_w, false);
        if let Some(ms) = out.seek_to {
            actions.push(UiAction::Seek(ms));
        }
        let shown_pos = out.preview_ms.unwrap_or(props.position_ms);
        let font = FontId::proportional(11.5);
        let tcol = if out.preview_ms.is_some() { pal.text } else { pal.text_muted };
        let (lt, rt) = if props.duration_ms > 0 || props.has_track {
            (format_time(shown_pos), format_time(props.duration_ms))
        } else {
            ("-:--".to_owned(), "-:--".to_owned())
        };
        let g = ui.painter().layout_no_wrap(lt, font.clone(), tcol);
        ui.painter().galley(pos2(bar_rect.left() - 8.0 - g.size().x, seek_y - g.size().y / 2.0), g, tcol);
        let g = ui.painter().layout_no_wrap(rt, font, pal.text_muted);
        ui.painter().galley(pos2(bar_rect.right() + 8.0, seek_y - g.size().y / 2.0), g, pal.text_muted);
    }

    // ---------------- right: toggles + volume ----------------
    {
        let mut rui = ui.new_child(UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)));
        rui.spacing_mut().item_spacing.x = 2.0;
        let slider_w = lay.slider_w;
        // volume (right-most)
        // icon (34) + spacing + slider
        let vol_w = 36.0 + if slider_w > 0.0 { slider_w + 2.0 } else { 0.0 };
        if props.no_output {
            // Quiet "no device" indicator in place of the volume control.
            rui.allocate_ui_with_layout(vec2(vol_w, 40.0), Layout::left_to_right(Align::Center), |ui| {
                let style = IconButtonStyle { color: Some(pal.text_faint), ..Default::default() };
                if icon_button_ex(ui, Icon::Mute, 18.0, style, tr.t("transport.no_output_tip")).clicked() {
                    actions.push(UiAction::Navigate(Page::Settings));
                }
                if slider_w > 0.0 {
                    ui.add(egui::Label::new(RichText::new(tr.t("transport.no_output")).size(11.5).color(pal.text_faint)).truncate());
                }
            });
        } else {
            let vol = rui
                .allocate_ui_with_layout(vec2(vol_w, 40.0), Layout::left_to_right(Align::Center), |ui| {
                    volume_control(ui, Id::new("transport_volume"), props.volume, props.muted, slider_w, &tr)
                })
                .inner;
            if let Some(g) = vol.set_gain {
                actions.push(UiAction::SetVolume(g));
            }
            if vol.toggle_mute {
                actions.push(UiAction::ToggleMute);
            }
        }
        if medium {
            rui.add_space(6.0);
            if wide
                && icon_button(&mut rui, Icon::Eq, 17.0, props.eq_open || props.eq_enabled, tr.t("transport.eq")).clicked()
            {
                actions.push(UiAction::ToggleEq);
            }
            let q = props.right_panel == Some(RightPanel::Queue);
            let tip = tr.f("transport.queue_n", &[&props.queue_len]);
            let qb = icon_button(&mut rui, Icon::Queue, 17.0, q, &tip);
            if props.queue_len > 0 {
                count_badge(&rui, qb.rect, props.queue_len, &pal);
            }
            if qb.clicked() {
                actions.push(UiAction::ToggleRightPanel(RightPanel::Queue));
            }
            let l = props.right_panel == Some(RightPanel::Lyrics);
            if icon_button(&mut rui, Icon::Lyrics, 17.0, l, tr.t("transport.lyrics")).clicked() {
                actions.push(UiAction::ToggleRightPanel(RightPanel::Lyrics));
            }
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_controls_fit_at_and_above_the_minimum() {
        assert_eq!(CENTER_MIN, 256.0);
        assert_eq!(MIN_WIDTH_ALL_CONTROLS, 692.0);
        for w in [MIN_WIDTH_ALL_CONTROLS, 700.0, 760.0, 900.0, 1264.0, 2000.0] {
            let l = transport_layout(w);
            assert!(l.all_controls, "w={w}");
            let center = w - 2.0 * PAD - l.left_w - l.right_w - 2.0 * REGION_GAP;
            assert!(center >= CENTER_MIN - 0.01, "w={w} center={center}");
            assert!(center >= CONTROLS_ROW_W, "shuffle / repeat shown at w={w}");
            assert!(l.left_w >= LEFT_MIN - 0.01 && l.right_w >= RIGHT_MIN - 0.01);
            assert!(l.slider_w >= SLIDER_MIN && l.right_w >= RIGHT_FIXED + l.slider_w - 0.01, "w={w}");
        }
        assert!(!transport_layout(MIN_WIDTH_ALL_CONTROLS - 1.0).all_controls);
    }
}
