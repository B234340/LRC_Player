pub mod audio;
pub mod dedup;
pub mod persistence;
pub mod scan;
pub mod decode;

// Re-export io_worker from scan for backwards compatibility with existing imports
pub use scan::io_worker as io_worker;
