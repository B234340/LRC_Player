//! App shell: sidebar, transport bar and the frames used to compose them.

pub mod drop_zone;
pub mod sidebar;
pub mod transport;

pub use drop_zone::file_drop;
pub use sidebar::{sidebar, PlaylistEntry, SidebarProps, SidebarState};
pub use transport::{transport_bar, TransportProps};

use eframe::egui::{Frame, Margin, Stroke};

use super::theme::{metrics, radius, Palette};

/// Allowance for a margin egui may put around the root UI (8 px on each side).
const ROOT_MARGIN_X: f32 = 16.0;
const ROOT_MARGIN_Y: f32 = 16.0;

/// Minimum window width: the transport bar spans the whole window (the sidebar and pages
/// sit above it), so the window must fit the bar with every control shown —
/// [`transport::MIN_WIDTH_ALL_CONTROLS`] (`2 × 14 pad + 160 artwork/title + 16 + 256
/// controls/seek bar + 16 + 249 lyrics/queue/EQ/quality/volume = 725`) + [`ROOT_MARGIN_X`] (16)
/// = 741. That leaves 741 − 16 − 216 − 8 = 501 px for pages with the sidebar expanded.
pub const MIN_WINDOW_WIDTH: f32 = transport::MIN_WIDTH_ALL_CONTROLS + ROOT_MARGIN_X;

/// Minimum window height: the sidebar's minimum ([`sidebar::SIDEBAR_MIN_HEIGHT`] = brand,
/// full nav, playlist heading + 2 rows and the settings / toggle block = 524) + the
/// transport bar (76) + [`ROOT_MARGIN_Y`] (16) = 616. The page header and several table
/// rows fit in the same height.
pub const MIN_WINDOW_HEIGHT: f32 = sidebar::SIDEBAR_MIN_HEIGHT + metrics::TRANSPORT_HEIGHT + ROOT_MARGIN_Y;

/// `[width, height]` for `ViewportBuilder::with_min_inner_size`.
pub const MIN_WINDOW_SIZE: [f32; 2] = [MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT];

/// Frame for the sidebar / transport / right panels (window background, no margin).
pub fn chrome_frame(p: &Palette) -> Frame {
    Frame::new().fill(p.bg)
}

/// Frame of the central area: an inset, rounded "surface" card on the window background.
/// Use as the `CentralPanel` frame; put page content inside [`page_frame`].
pub fn central_frame(p: &Palette) -> Frame {
    Frame::new().fill(p.bg).inner_margin(Margin { left: 0, right: 8, top: 8, bottom: 8 })
}

/// Rounded surface holding the page content.
pub fn page_frame(p: &Palette) -> Frame {
    Frame::new()
        .fill(p.surface)
        .corner_radius(radius::LG)
        .stroke(Stroke::new(1.0, p.border))
        .inner_margin(Margin { left: 24, right: 24, top: 20, bottom: 8 })
}

/// Frame of the right-hand panel content (queue / lyrics).
pub fn right_panel_frame(p: &Palette) -> Frame {
    Frame::new().fill(p.bg).inner_margin(Margin { left: 8, right: 14, top: 14, bottom: 8 })
}
