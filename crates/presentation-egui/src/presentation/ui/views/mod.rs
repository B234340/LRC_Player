//! Page views.

pub mod album_grid;
pub mod eq_panel;
pub mod lyrics_view;
pub mod now_playing;
pub mod queue_panel;
pub mod search;
pub mod settings;
pub mod track_table;

pub use album_grid::{album_grid, AlbumGridProps, AlbumItem};
pub use eq_panel::{eq_panel, matching_preset, EqPanelProps, EqPreset, EQ_PRESETS};
pub use lyrics_view::{lyrics_view, LyricLine, LyricsProps, LyricsViewState};
pub use now_playing::{make_backdrop, now_playing, NowPlayingProps, NowPlayingState};
pub use queue_panel::{apply_reorder, queue_empty, queue_panel, queue_rows, QueueItem, QueueProps};
pub use search::{filter_and_sort, normalize};
pub use settings::{settings_row, settings_section};
pub use track_table::{track_table, Column, IndexMode, TableLinks, TrackDragPayload, TrackRow, TrackTableProps, TrackTableState};
