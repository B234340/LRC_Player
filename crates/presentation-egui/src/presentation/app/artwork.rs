//! Play-tab artwork: requested once per track from the background [`ArtworkLoader`]
//! (decode, square crop, downscale and a tiny blurred copy happen on the worker; the result,
//! including "no artwork", is cached so nothing is retried every frame).
//!
//! [`ArtworkLoader`]: crate::presentation::app::ArtworkLoader

use crate::presentation::app::GuiApp;

impl GuiApp {
    /// Requests the current track's artwork once and installs it when ready. Cheap per frame.
    pub(crate) fn ensure_play_artwork(&mut self) {
        if self.play_path.is_empty() {
            self.play_artwork_handle = None;
            self.play_artwork_blur_handle = None;
            self.play_artwork_requested_for = None;
            return;
        }
        if self.play_artwork_requested_for.as_deref() != Some(self.play_path.as_str()) {
            self.play_artwork_requested_for = Some(self.play_path.clone());
            self.play_artwork_handle = None;
            self.play_artwork_blur_handle = None;
            self.artwork.request_play_artwork(&self.play_path);
        }
        if let Some((_, regular, blur)) = self.artwork.take_play_artwork(&self.play_path) {
            self.play_artwork_handle = regular;
            self.play_artwork_blur_handle = blur;
        }
    }
}
