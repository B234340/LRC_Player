//! UI → アプリケーションの操作コマンド定義
//!
//! `AppCommand` は UI が発行する操作を列挙したもので、`AppFacade::handle_command` が
//! これらを受け取って状態変更や副作用（再生、スキャン、永続化トリガ等）を行います。
use crate::core::ids::{AlbumId, ArtistId, PlaylistId, QueueEntryId, TrackId};
use crate::core::library::{ArtistKind, DirectoryNodeId};
use crate::core::persistence::TabId;
use crate::core::scan::ScanCommand;

/// トラックリストがどのタブ/起点から来たかを示す列挙
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackListOrigin {
    Queue,
    Play,
    Album,
    Artist,
    Playlist,
    Library,
}

/// UI からファサードに送られるコマンド群
#[derive(Debug, Clone, PartialEq)]
pub enum AppCommand {
    /// リスト内のトラックがクリックされた。`visible_tracks` と `clicked_index` を渡すことで
    /// どのトラックが選択されたかと、リスト全体のコンテキストを伝える。
    ClickTrack {
        visible_tracks: Vec<TrackId>,
        clicked_index: usize,
        origin: TrackListOrigin,
    },
    PlayNext {
        track_id: TrackId,
    },
    AppendToQueue {
        track_id: TrackId,
    },
    AddToPlaylist {
        playlist_id: PlaylistId,
        track_id: TrackId,
    },
    ToggleFavorite {
        track_id: TrackId,
    },
    ShowTrackDetails {
        track_id: TrackId,
    },
    QueueRemoveByContext {
        entry_id: QueueEntryId,
    },
    TogglePlayPause,
    SeekBy {
        delta_ms: i64,
    },
    SeekTo {
        position_ms: u64,
    },
    NextTrack,
    PreviousTrack,
    SelectTab(TabId),
    SelectAlbum(Option<AlbumId>),
    SelectArtistKind(ArtistKind),
    SelectArtist(Option<ArtistId>),
    SelectPlaylist(Option<PlaylistId>),
    SelectDirectory(Option<DirectoryNodeId>),
    SetPreferredOutputDevice {
        device_name: Option<String>,
    },
    SetDebugMode {
        enabled: bool,
    },
    SetLogFilters {
        filters: Vec<String>,
    },
    Scan(ScanCommand),
    CreatePlaylist {
        name: String,
    },
}
