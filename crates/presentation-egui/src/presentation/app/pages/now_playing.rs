//! Now Playing: artwork over the blurred backdrop, title / artist / album links, synced
//! lyrics (karaoke, click-to-seek, offset + font controls) and an optional slim spectrum.
//!
//! The view reports how large the cover is shown (`wanted_art_px`, physical px, including
//! the enlarged view); beyond 1024 px the page asks the artwork loader for a sharper copy
//! (1536 / 2048 px). The loader frees it a few frames after the page stops asking, i.e.
//! shortly after leaving this page.

use eframe::egui::{self, pos2, Rect, Ui};

use crate::presentation::app::{lyrics::lyric_lines, spectrum::paint_spectrum_strip, GuiApp};

/// Height of the spectrum strip under the artwork column, and the gap above it.
const SPECTRUM_H: f32 = 34.0;
const SPECTRUM_GAP: f32 = 14.0;
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
        line_shift: app.lyrics_shift.lines,
        char_shift: app.lyrics_shift.chars,
        can_char_shift: app.loaded_timeline.as_ref().is_some_and(|t| t.karaoke_char_count() > 1),
        can_generate_timing: !app.config.align_command.trim().is_empty() && app.selected_lyrics_index.is_some(),
        generating_timing: app.lyric_alignment_running(),
        desktop_lyrics: app.config.desktop_lyrics.enabled,
        tr,
    });
    let show_spectrum = app.config.show_spectrum && has_track;
    app.ui.now.bottom_reserve = if show_spectrum { SPECTRUM_H + SPECTRUM_GAP } else { 0.0 };
    // Sharper cover when it is shown beyond the regular 1024 px texture (last frame's size).
    let hires = match &app.play_artwork_handle {
        Some(_) if has_track => app.artwork.play_artwork_hires(&app.play_path, app.ui.now.wanted_art_px),
        _ => None,
    };
    let artwork = hires.as_ref().or(app.play_artwork_handle.as_ref()).map(Tex::from);
    let props = NowPlayingProps {
        has_track,
        title: &title,
        artist: &artist,
        album: &album,
        artwork,
        backdrop: app.play_backdrop_handle.as_ref().map(Tex::from),
        seed: &seed,
        lyrics,
        favorite: has_track.then(|| app.is_favorite_path(&app.play_path)),
        lyrics_search: (has_track && app.lyrics_candidates.is_empty()).then(|| app.lyrics_fetch_running()),
        tr,
    };
    let actions = now_playing(&mut child, &props, &mut app.ui.now);

    // Slim spectrum under the artwork column (wide layout only), drawn over the backdrop in
    // the space the view kept free for it.
    let layout = app.ui.now.layout.filter(|l| l.wide);
    if let Some(layout) = layout.filter(|_| show_spectrum) {
        if app.is_playing() {
            if let Some(tap) = app.spectrum_tap() {
                app.spectrum.update(&tap);
            }
        }
        let (art, inner) = (layout.art, layout.inner);
        let top = (inner.bottom() - SPECTRUM_H).max(layout.info.bottom() + 8.0);
        let strip = Rect::from_min_max(pos2(art.left() + 24.0, top), pos2(art.right() - 24.0, inner.bottom()));
        paint_spectrum_strip(&ui.painter_at(rect), strip, &app.spectrum, egui::Color32::from_white_alpha(200));
    }
    actions
}
