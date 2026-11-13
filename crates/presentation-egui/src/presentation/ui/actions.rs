//! User intents emitted by the `ui` components.
//!
//! Components never call into the application; they return [`UiAction`]s that the app
//! interprets. All payloads are plain data so the enum stays independent of other crates
//! (except [`EqSettings`], whose API is stable).

use std::path::PathBuf;

pub use lrc_player_application::infrastructure::audio::engine::EqSettings;

/// Top-level pages reachable from the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Page {
    #[default]
    NowPlaying,
    Library,
    /// Full-page play queue (now playing + up next).
    Queue,
    Albums,
    Artists,
    Playlists,
    Folders,
    Settings,
}

impl Page {
    /// Pages in sidebar order (Settings is pinned to the bottom separately).
    pub const MAIN: [Page; 7] = [
        Page::NowPlaying,
        Page::Queue,
        Page::Library,
        Page::Albums,
        Page::Artists,
        Page::Playlists,
        Page::Folders,
    ];
}

/// Content of the toggleable right-hand panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RightPanel {
    Queue,
    Lyrics,
}

/// Repeat mode shown by the transport bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// Sortable track-table columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SortColumn {
    /// Original (library / playlist) order.
    #[default]
    Index,
    Title,
    Artist,
    Album,
    Duration,
    Year,
    Genre,
}

/// Sort column + direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct SortSpec {
    pub column: SortColumn,
    pub descending: bool,
}

/// Every user intent the `ui` components can emit.
#[derive(Clone, Debug, PartialEq)]
pub enum UiAction {
    // ----- transport -----
    PlayPause,
    Next,
    Previous,
    /// Absolute seek, in ms.
    Seek(u64),
    /// Relative seek, in ms (keyboard ←/→).
    SeekRelative(i64),
    /// New volume as a *linear gain* `0.0..=1.0` (already mapped with `ui_to_gain`).
    SetVolume(f32),
    ToggleMute,
    ToggleShuffle,
    CycleRepeat,

    // ----- navigation / chrome -----
    Navigate(Page),
    /// Go back to the previous location (← button, mouse back button, Alt+←).
    Back,
    ToggleSidebar,
    ToggleRightPanel(RightPanel),
    /// Toggle the EQ popover/window.
    ToggleEq,
    /// Focus the page's search box (Ctrl/Cmd+F).
    FocusSearch,
    OpenPlaylist(String),
    /// Open the album page (key from `AlbumCard::key`).
    OpenAlbum(String),
    /// Open the artist page by artist name.
    OpenArtist(String),
    /// Open the artist page in a given grouping (`album_artist` = Album artist mode), e.g.
    /// from the artist shown on an album card.
    OpenArtistAs { name: String, album_artist: bool },

    // ----- library / tracks -----
    /// Replace the queue with `paths` and start at `start`.
    PlayTracks { paths: Vec<PathBuf>, start: usize },
    /// Play the whole album (key from `AlbumCard::key`).
    PlayAlbum(String),
    /// Insert right after the current track.
    PlayNext(Vec<PathBuf>),
    /// Append to the end of the queue.
    Enqueue(Vec<PathBuf>),
    AddToPlaylist { playlist_id: String, paths: Vec<PathBuf> },
    /// Add the tracks to / remove them from the favorites list.
    SetFavorite { paths: Vec<PathBuf>, favorite: bool },
    /// Toggle the favorite state of the playing track.
    ToggleFavoriteCurrent,
    /// Show the album the track belongs to (album name, artist).
    ShowAlbum { album: String, artist: String },
    /// Open the artist page of a track's artist (switches the Artists view to "Artist").
    ShowArtist(String),
    /// Open the album page of the album `path` belongs to.
    ShowAlbumOfTrack(PathBuf),
    OpenContainingFolder(PathBuf),
    ShowDetails(PathBuf),
    /// Table header clicked.
    Sort(SortSpec),
    /// Search query changed.
    Search(String),
    /// Rows of a reorderable track table were dragged: `from`/`to` are indices into the
    /// table's *source rows* (`to` is the insertion index before removal).
    Reorder { from: usize, to: usize },
    /// Remove rows (source-row indices) from the list the table shows (e.g. a playlist).
    RemoveRows(Vec<usize>),
    /// Show / hide an optional track-table column (header context menu).
    SetColumnVisible { column: crate::presentation::ui::views::Column, visible: bool },

    // ----- queue -----
    QueuePlayIndex(usize),
    QueueReorder { from: usize, to: usize },
    QueueRemove(usize),
    QueueClear,
    QueueSaveAsPlaylist,
    /// Insert tracks into the queue before index `at` (drag & drop onto the queue).
    QueueInsert { paths: Vec<PathBuf>, at: usize },

    // ----- lyrics -----
    /// Seek to the start of lyric line `index` (`start_ms` is the line's lyric time,
    /// before offsets; prefer `LyricsTimeline::seek_position_for_line(index)`).
    SeekToLyricLine { index: usize, start_ms: u64 },
    LyricOffsetDelta(i64),
    LyricOffsetReset,
    SaveLyricOffset,
    /// Shift the lyric text by this many lines (relative; positive = later).
    LyricShiftLines(i32),
    /// Shift the karaoke text by this many characters (relative; positive = later).
    LyricShiftChars(i32),
    /// Undo the text shift.
    LyricShiftReset,
    /// Generate per-character timing with the external command (settings).
    GenerateLyricTiming,
    /// New lyric font size (points of the *active* line).
    SetLyricFontSize(f32),

    // ----- EQ -----
    SetEq(EqSettings),

    // ----- folders / scan -----
    AddFolder,
    RemoveFolder(PathBuf),
    Rescan,
    DropFiles(Vec<PathBuf>),

    // ----- appearance -----
    SetTheme(crate::presentation::ui::theme::ThemeMode),
    SetAccent(crate::presentation::ui::theme::Accent),
}
