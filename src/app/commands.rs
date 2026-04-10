use crate::core::ids::{AlbumId, ArtistId, PlaylistId, QueueEntryId, TrackId};
use crate::core::library::{ArtistKind, DirectoryNodeId};
use crate::core::persistence::TabId;
use crate::core::scan::ScanCommand;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackListOrigin {
    Queue,
    Play,
    Album,
    Artist,
    Playlist,
    Library,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppCommand {
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
    Scan(ScanCommand),
    CreatePlaylist {
        name: String,
    },
}