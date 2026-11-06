pub mod artwork;
pub mod audio;
pub mod decode;
pub mod dedup;
pub mod library_snapshot;
pub mod lyrics;
pub mod persistence;
pub mod scan;

// Re-export io_worker from scan for backwards compatibility with existing imports
pub use scan::io_worker;
