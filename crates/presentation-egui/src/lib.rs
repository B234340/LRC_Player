//! Egui presentation crate and facade re-exports for app composition.

pub use lrc_player_domain as domain;
pub use lrc_player_application as application;
pub use lrc_player_application::config;
pub use lrc_player_application::localization;
pub use lrc_player_application::infrastructure as infrastructure;
pub use lrc_player_ports as ports;
pub use lrc_player_shared as shared;
pub mod presentation;
