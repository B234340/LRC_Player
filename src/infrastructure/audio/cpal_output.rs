// Thin wrapper that exposes the existing CPAL-backed output implementation.
// If a platform-specific backend lives in `backends/cpal.rs` it may be used here.

pub use crate::infrastructure::audio::cpal_backend::CpalOutput;

/// List available output devices by name.
pub fn available_device_names() -> Vec<String> {
    CpalOutput::available_device_names()
}

/// Get the default output device name when available.
pub fn default_device_name() -> Option<String> {
    CpalOutput::default_device_name()
}
