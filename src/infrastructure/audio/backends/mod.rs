// Platform-specific backend stubs. Implementations should live here.
#[cfg(target_os = "windows")]
pub mod wasapi;

#[cfg(target_os = "macos")]
pub mod coreaudio;

#[cfg(all(unix, not(target_os = "macos")))]
pub mod pipewire;

// Generic CPAL backend wrapper
pub mod cpal;

#[cfg(target_os = "windows")]
pub use wasapi::*;

#[cfg(target_os = "macos")]
pub use coreaudio::*;

#[cfg(all(unix, not(target_os = "macos")))]
pub use pipewire::*;

// expose cpal unconditionally as a generic backend
pub use cpal::*;
