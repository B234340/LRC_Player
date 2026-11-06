//! Track-row artwork drawing.
//!
//! Lookups go through the app's background [`ArtworkLoader`] (cached per track, including
//! "no artwork"); nothing here touches the file system.
//!
//! [`ArtworkLoader`]: crate::presentation::app::ArtworkLoader

use eframe::egui;

use crate::presentation::app::{ArtworkMode, GuiApp};

/// Draws the artwork for `track_path` (or a placeholder while loading / when there is none).
pub(crate) fn render_track_artwork(
    ui: &mut egui::Ui,
    app: &mut GuiApp,
    track_path: &str,
    mode: ArtworkMode,
    art_size: egui::Vec2,
    is_playing: bool,
) {
    if let Some(texture) = app.artwork.track_thumbnail(track_path, mode) {
        ui.add(egui::Image::new((texture.id(), art_size)).corner_radius(4.0));
        return;
    }
    let art_bg = if is_playing {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().faint_bg_color
    };
    let (art_rect, _) = ui.allocate_exact_size(art_size, egui::Sense::hover());
    ui.painter().rect_filled(art_rect, 4.0, art_bg);
    ui.painter().text(
        art_rect.center(),
        egui::Align2::CENTER_CENTER,
        "ART",
        egui::TextStyle::Small.resolve(ui.style()),
        ui.visuals().weak_text_color(),
    );
}
