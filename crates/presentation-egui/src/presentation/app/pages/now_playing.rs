//! Now Playing: artwork over the blurred backdrop, title / artist / album links, synced
//! lyrics (karaoke, click-to-seek, offset + font controls) and an optional slim spectrum.

use eframe::egui::{self, pos2, Rect, Ui};

use crate::presentation::app::{lyrics::lyric_lines, spectrum::paint_spectrum_strip, GuiApp};
use crate::presentation::ui::{
    i18n::Tr,
    views::{now_playing, LyricsProps, NowPlayingProps},
    widgets::Tex,
    UiAction,
};

pub(super) fn show(app: &mut GuiApp, ui: &mut Ui, tr: Tr) -> Vec<UiAction> {
    let rect = ui.available_rect_before_wrap();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    child.set_clip_rect(rect);

    let has_track = !app.play_path.is_empty();
    let title = app.current_title();
    let artist = app.play_artist.clone().unwrap_or_default();
    let album = app.play_album.clone().unwrap_or_default();
    let seed = if album.is_empty() { title.clone() } else { album.clone() };
    let lines = app
        .loaded_timeline
        .as_ref()
        .filter(|t| !t.is_empty())
        .map(|t| lyric_lines(t, app.config.karaoke));
    let pos = app.loaded_timeline.as_ref().and_then(|t| t.position_at(app.play_position_ms));
    let lyrics = lines.as_ref().map(|lines| LyricsProps {
        lines,
        revision: app.lyrics_revision,
        current: pos.map(|p| p.line_index),
        highlighted_chars: pos.map(|p| p.highlighted_chars).unwrap_or(0.0),
        offset_ms: app.lyrics_offset_ms,
        font_size: app.config.lyric_font_size,
        align: egui::Align::Min,
        on_backdrop: true,
        show_controls: true,
        can_save_offset: app.selected_lyrics_index.is_some(),
        tr,
    });
    let props = NowPlayingProps {
        has_track,
        title: &title,
        artist: &artist,
        album: &album,
        artwork: app.play_artwork_handle.as_ref().map(Tex::from),
        backdrop: app.play_backdrop_handle.as_ref().map(Tex::from),
        seed: &seed,
        lyrics,
        tr,
    };
    let actions = now_playing(&mut child, &props, &mut app.ui.now);

    // Slim spectrum under the artwork column (wide layout only), drawn over the backdrop.
    let inner = rect.shrink(32.0);
    if app.config.show_spectrum && has_track && inner.width() >= 820.0 {
        if app.is_playing() {
            if let Some(tap) = app.spectrum_tap() {
                app.spectrum.update(&tap);
            }
        }
        let col_w = (inner.width() * 0.42).clamp(300.0, 520.0);
        let strip = Rect::from_min_max(pos2(inner.left() + 24.0, inner.bottom() - 34.0), pos2(inner.left() + col_w - 24.0, inner.bottom()));
        paint_spectrum_strip(ui.painter(), strip, &app.spectrum, egui::Color32::from_white_alpha(200));
    }
    actions
}
