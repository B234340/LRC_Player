//! App shell: sidebar, transport bar and the frames used to compose them.

pub mod drop_zone;
pub mod sidebar;
pub mod transport;

pub use drop_zone::file_drop;
pub use sidebar::{sidebar, PlaylistEntry, SidebarProps, SidebarState};
pub use transport::{transport_bar, TransportProps};

use eframe::egui::{Frame, Margin, Stroke};

use super::theme::{radius, Palette};

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
