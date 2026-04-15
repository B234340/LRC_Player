// Thin wrapper that exposes the existing CPAL-backed output implementation.
// If a platform-specific backend lives in `backends/cpal.rs` it may be used here.

pub use crate::infrastructure::audio::cpal_backend::CpalOutput;
