//! Album-grid artwork, served by the background [`ArtworkLoader`]
//! (resolution, decode and 256 px thumbnails happen on the worker; albums without artwork
//! are remembered and not retried until the library changes).
//!
//! [`ArtworkLoader`]: crate::presentation::app::ArtworkLoader

use eframe::egui;

use crate::presentation::app::GuiApp;

impl GuiApp {
    /// Drops all cached artwork (e.g. after the library was cleared).
    pub(crate) fn clear_album_artwork_cache(&mut self) {
        self.artwork.clear();
    }

    /// Cover texture for an album card, requesting it from the worker on first use.
    pub(crate) fn album_artwork_texture(&mut self, album_title: &str) -> Option<egui::TextureHandle> {
        self.artwork.album_cover(album_title)
    }
}
