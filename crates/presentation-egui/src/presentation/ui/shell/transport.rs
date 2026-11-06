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
    pub tr: Tr,
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
    let wide = w >= 920.0;
    let medium = w >= 640.0;
    let pad = 14.0;
    let inner = full.shrink2(vec2(pad, 0.0));

    let left_w = if wide { (w * 0.27).clamp(220.0, 340.0) } else if medium { 200.0 } else { 56.0 };
    let right_w = if wide { (w * 0.27).clamp(230.0, 330.0) } else if medium { 150.0 } else { 40.0 };
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
            let text_rect = Rect::from_min_max(pos2(art.right() + 12.0, left.top()), left.max);
            let mut tui = ui.new_child(UiBuilder::new().max_rect(text_rect).layout(Layout::top_down(Align::Min)));
            tui.spacing_mut().item_spacing.y = 1.0;
            tui.add_space((text_rect.height() - 38.0) / 2.0);
            if props.has_track {
                tui.add(egui::Label::new(theme::strong(&tui, props.title).color(pal.text)).truncate());
                tui.add(egui::Label::new(RichText::new(props.artist).size(12.5).color(pal.text_muted)).truncate());
            } else {
                tui.add_space(9.0);
                tui.add(egui::Label::new(RichText::new(tr.t("transport.nothing_playing")).color(pal.text_faint)).truncate());
            }
        }
    }

    // ---------------- center: controls + seek ----------------
    {
        let cw = center.width();
        let show_modes = cw >= 300.0;
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
        let time_w = if props.duration_ms >= 3_600_000 { 52.0 } else { 40.0 };
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
        let slider_w = if wide { 96.0 } else if medium { 64.0 } else { 0.0 };
        // volume (right-most)
        // icon (34) + spacing + slider
        let vol_w = 36.0 + if slider_w > 0.0 { slider_w + 2.0 } else { 0.0 };
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
        if medium {
            rui.add_space(6.0);
            if wide
                && icon_button(&mut rui, Icon::Eq, 17.0, props.eq_open || props.eq_enabled, tr.t("transport.eq")).clicked()
            {
                actions.push(UiAction::ToggleEq);
            }
            let q = props.right_panel == Some(RightPanel::Queue);
            if icon_button(&mut rui, Icon::Queue, 17.0, q, tr.t("transport.queue")).clicked() {
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
