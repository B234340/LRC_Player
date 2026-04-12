// Library entry for integration tests and external consumers.
// This re-exports the internal modules so tests in `tests/` can import them.
pub mod app;
pub mod audio;
pub mod core;
pub mod logging;
pub mod decode;
pub mod ui;
pub mod domain;
pub mod application;
pub mod infrastructure;
pub mod presentation;
pub mod shared;
