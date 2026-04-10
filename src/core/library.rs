use crate::core::artwork::ArtworkKey;
use crate::core::ids::{AlbumId, ArtistId, PathId, StrId, TrackId, TrackKey};
use crate::core::strings::StringInterner;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DirectoryNodeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtistKind {
    TrackArtist,
    AlbumArtist,
    Composer,
}

#[derive(Debug, Default, Clone)]
pub struct TrackTable {
    pub titles: Vec<StrId>,
    pub stable_keys: Vec<TrackKey>,
    pub album_ids: Vec<Option<AlbumId>>,
    pub artist_ids: Vec<Option<ArtistId>>,
    pub album_artist_ids: Vec<Option<ArtistId>>,
    pub composer_ids: Vec<Option<ArtistId>>,
    pub path_ids: Vec<PathId>,
    pub file_name_ids: Vec<StrId>,
    pub track_numbers: Vec<Option<std::num::NonZeroU16>>,
    pub durations_ms: Vec<u32>,
    pub artwork_keys: Vec<Option<ArtworkKey>>,
}

#[derive(Debug, Default, Clone)]
pub struct AlbumTable {
    pub titles: Vec<StrId>,
    pub album_artist_ids: Vec<Option<ArtistId>>,
    pub representative_artwork: Vec<Option<ArtworkKey>>,
}

#[derive(Debug, Default, Clone)]
pub struct ArtistTable {
    pub names: Vec<StrId>,
}

#[derive(Debug, Default, Clone)]
pub struct DirectoryNode {
    pub name: StrId,
    pub children: Vec<DirectoryNodeId>,
    pub tracks: Vec<TrackId>,
}

#[derive(Debug, Default, Clone)]
pub struct DirectoryTree {
    pub nodes: Vec<DirectoryNode>,
    pub root: DirectoryNodeId,
}

#[derive(Debug, Default, Clone)]
pub struct LibraryIndexes {
    pub tracks_by_album: Vec<Vec<TrackId>>,
    pub tracks_by_artist: Vec<Vec<TrackId>>,
    pub tracks_by_album_artist: Vec<Vec<TrackId>>,
    pub tracks_by_composer: Vec<Vec<TrackId>>,
}

#[derive(Debug, Default, Clone)]
pub struct Library {
    pub tracks: TrackTable,
    pub albums: AlbumTable,
    pub artists: ArtistTable,
    pub directories: DirectoryTree,
    pub indexes: LibraryIndexes,
}

impl Library {
    pub fn resolve_track_key(&self, key: &TrackKey) -> Option<TrackId> {
        self.tracks
            .stable_keys
            .iter()
            .position(|existing| existing == key)
            .map(|index| TrackId(index as u32))
    }

    pub fn album_title(&self, album_id: AlbumId, strings: &StringInterner) -> Option<String> {
        self.albums.titles.get(album_id.0 as usize).and_then(|id| strings.resolve(*id))
    }

    pub fn artist_name(&self, artist_id: ArtistId, strings: &StringInterner) -> Option<String> {
        self.artists.names.get(artist_id.0 as usize).and_then(|id| strings.resolve(*id))
    }

    pub fn visible_tracks_for_album(&self, album_id: AlbumId) -> &[TrackId] {
        self.indexes
            .tracks_by_album
            .get(album_id.0 as usize)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn visible_tracks_for_artist(&self, artist_kind: ArtistKind, artist_id: ArtistId) -> &[TrackId] {
        let index = artist_id.0 as usize;
        match artist_kind {
            ArtistKind::TrackArtist => self
                .indexes
                .tracks_by_artist
                .get(index)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            ArtistKind::AlbumArtist => self
                .indexes
                .tracks_by_album_artist
                .get(index)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            ArtistKind::Composer => self
                .indexes
                .tracks_by_composer
                .get(index)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        }
    }
}