// Domain layer facade — re-export domain-related modules from the crate root.
// This provides a stable `domain::` namespace without moving existing files.
pub use crate::core as core;
