use crate::core::models::TrackId;
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AppTab {
    Queue,
    Play,
    Album,
    Artist,
    Playlist,
    Library,
    Scan,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ArtistKind {
    Artist,
    AlbumArtist,
    Composer,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum QueuePlaybackMode {
    SingleRepeat,
    QueueRepeat,
    QueueShuffle,
}

#[derive(Default)]
pub(super) struct DirectoryNode {
    pub(super) children: BTreeMap<String, DirectoryNode>,
    pub(super) tracks: Vec<LibraryTrackView>,
}

impl DirectoryNode {
    pub(super) fn descendant_folder_count(&self) -> usize {
        self.children.len()
            + self
                .children
                .values()
                .map(DirectoryNode::descendant_folder_count)
                .sum::<usize>()
    }

    pub(super) fn descendant_track_count(&self) -> usize {
        self.tracks.len()
            + self
                .children
                .values()
                .map(DirectoryNode::descendant_track_count)
                .sum::<usize>()
    }
}

#[derive(Clone)]
pub(super) struct LibraryTrackView {
    pub(super) id: TrackId,
    pub(super) title: String,
    pub(super) artist: String,
    pub(super) album: String,
    pub(super) full_path: String,
    pub(super) lyric_path: Option<String>,
}

#[derive(Clone)]
pub(super) struct AlbumView {
    pub(super) key: String,
    pub(super) title: String,
    pub(super) subtitle: String,
    pub(super) track_ids: Vec<TrackId>,
    pub(super) preview_track_id: TrackId,
}

#[derive(Clone)]
pub(super) struct ArtistView {
    pub(super) name: String,
    pub(super) track_ids: Vec<TrackId>,
}