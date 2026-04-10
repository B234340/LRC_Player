use std::cmp::Ordering;
use std::collections::HashMap;
use std::num::NonZeroU16;
use std::path::Path;

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

#[derive(Debug, Clone)]
pub struct TrackInput {
    pub title: String,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub composer: Option<String>,
    pub normalized_path: String,
    pub file_size: u64,
    pub modified_unix_secs: u64,
    pub file_name: String,
    pub track_number: Option<NonZeroU16>,
    pub duration_ms: u32,
    pub artwork_key: Option<ArtworkKey>,
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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct Library {
    pub tracks: TrackTable,
    pub albums: AlbumTable,
    pub artists: ArtistTable,
    pub paths: Vec<String>,
    pub directories: DirectoryTree,
    pub indexes: LibraryIndexes,
}

impl Default for DirectoryTree {
    fn default() -> Self {
        Self {
            nodes: vec![DirectoryNode::default()],
            root: DirectoryNodeId(0),
        }
    }
}

impl Default for Library {
    fn default() -> Self {
        Self {
            tracks: TrackTable::default(),
            albums: AlbumTable::default(),
            artists: ArtistTable::default(),
            paths: Vec::new(),
            directories: DirectoryTree::default(),
            indexes: LibraryIndexes::default(),
        }
    }
}

impl Library {
    pub fn upsert_track(&mut self, strings: &mut StringInterner, input: TrackInput) -> TrackId {
        let title_id = strings.intern(input.title);
        let file_name_id = strings.intern(input.file_name);
        let path_id = self.path_id_for(&input.normalized_path);
        let artist_id = input.artist.map(|value| self.artist_id_for(strings, value));
        let album_artist_id = input
            .album_artist
            .map(|value| self.artist_id_for(strings, value));
        let composer_id = input.composer.map(|value| self.artist_id_for(strings, value));
        let album_id = input
            .album
            .map(|value| self.album_id_for(strings, value, album_artist_id));
        let track_key = TrackKey {
            normalized_path: input.normalized_path,
            file_size: input.file_size,
            modified_unix_secs: input.modified_unix_secs,
        };

        let track_id = if let Some(existing) = self.resolve_track_key(&track_key) {
            let index = existing.0 as usize;
            self.tracks.titles[index] = title_id;
            self.tracks.stable_keys[index] = track_key;
            self.tracks.album_ids[index] = album_id;
            self.tracks.artist_ids[index] = artist_id;
            self.tracks.album_artist_ids[index] = album_artist_id;
            self.tracks.composer_ids[index] = composer_id;
            self.tracks.path_ids[index] = path_id;
            self.tracks.file_name_ids[index] = file_name_id;
            self.tracks.track_numbers[index] = input.track_number;
            self.tracks.durations_ms[index] = input.duration_ms;
            self.tracks.artwork_keys[index] = input.artwork_key;
            existing
        } else {
            let track_id = TrackId(self.tracks.titles.len() as u32);
            self.tracks.titles.push(title_id);
            self.tracks.stable_keys.push(track_key);
            self.tracks.album_ids.push(album_id);
            self.tracks.artist_ids.push(artist_id);
            self.tracks.album_artist_ids.push(album_artist_id);
            self.tracks.composer_ids.push(composer_id);
            self.tracks.path_ids.push(path_id);
            self.tracks.file_name_ids.push(file_name_id);
            self.tracks.track_numbers.push(input.track_number);
            self.tracks.durations_ms.push(input.duration_ms);
            self.tracks.artwork_keys.push(input.artwork_key);
            track_id
        };

        self.rebuild_indexes(strings);
        self.rebuild_directories(strings);
        track_id
    }

    pub fn resolve_track_key(&self, key: &TrackKey) -> Option<TrackId> {
        self.tracks
            .stable_keys
            .iter()
            .position(|existing| existing == key)
            .map(|index| TrackId(index as u32))
    }

    pub fn track_key(&self, track_id: TrackId) -> Option<&TrackKey> {
        self.tracks.stable_keys.get(track_id.0 as usize)
    }

    pub fn track_path(&self, track_id: TrackId) -> Option<&str> {
        let path_id = *self.tracks.path_ids.get(track_id.0 as usize)?;
        self.paths.get(path_id.0 as usize).map(String::as_str)
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

    fn path_id_for(&mut self, path: &str) -> PathId {
        if let Some(index) = self.paths.iter().position(|existing| existing == path) {
            return PathId(index as u32);
        }

        let id = PathId(self.paths.len() as u32);
        self.paths.push(path.to_string());
        id
    }

    fn artist_id_for(&mut self, strings: &mut StringInterner, name: String) -> ArtistId {
        let name_id = strings.intern(name);
        if let Some(index) = self.artists.names.iter().position(|existing| *existing == name_id) {
            return ArtistId(index as u32);
        }

        let artist_id = ArtistId(self.artists.names.len() as u32);
        self.artists.names.push(name_id);
        artist_id
    }

    fn album_id_for(
        &mut self,
        strings: &mut StringInterner,
        title: String,
        album_artist_id: Option<ArtistId>,
    ) -> AlbumId {
        let title_id = strings.intern(title);
        if let Some(index) = self
            .albums
            .titles
            .iter()
            .zip(self.albums.album_artist_ids.iter())
            .position(|(existing_title, existing_artist)| {
                *existing_title == title_id && *existing_artist == album_artist_id
            })
        {
            return AlbumId(index as u32);
        }

        let album_id = AlbumId(self.albums.titles.len() as u32);
        self.albums.titles.push(title_id);
        self.albums.album_artist_ids.push(album_artist_id);
        self.albums.representative_artwork.push(None);
        album_id
    }

    fn rebuild_indexes(&mut self, strings: &StringInterner) {
        self.indexes.tracks_by_album = vec![Vec::new(); self.albums.titles.len()];
        self.indexes.tracks_by_artist = vec![Vec::new(); self.artists.names.len()];
        self.indexes.tracks_by_album_artist = vec![Vec::new(); self.artists.names.len()];
        self.indexes.tracks_by_composer = vec![Vec::new(); self.artists.names.len()];

        for track_index in 0..self.tracks.titles.len() {
            let track_id = TrackId(track_index as u32);
            if let Some(album_id) = self.tracks.album_ids[track_index] {
                self.indexes.tracks_by_album[album_id.0 as usize].push(track_id);
            }
            if let Some(artist_id) = self.tracks.artist_ids[track_index] {
                self.indexes.tracks_by_artist[artist_id.0 as usize].push(track_id);
            }
            if let Some(artist_id) = self.tracks.album_artist_ids[track_index] {
                self.indexes.tracks_by_album_artist[artist_id.0 as usize].push(track_id);
            }
            if let Some(artist_id) = self.tracks.composer_ids[track_index] {
                self.indexes.tracks_by_composer[artist_id.0 as usize].push(track_id);
            }
        }

        let track_numbers = &self.tracks.track_numbers;
        let file_name_ids = &self.tracks.file_name_ids;
        let album_ids = &self.tracks.album_ids;
        let album_titles = &self.albums.titles;

        for tracks in &mut self.indexes.tracks_by_album {
            tracks.sort_by(|left, right| {
                Self::compare_album_tracks_with(track_numbers, file_name_ids, *left, *right, strings)
            });
        }

        for tracks in &mut self.indexes.tracks_by_artist {
            tracks.sort_by(|left, right| {
                Self::compare_artist_tracks_with(
                    album_ids,
                    album_titles,
                    file_name_ids,
                    *left,
                    *right,
                    strings,
                )
            });
        }

        for tracks in &mut self.indexes.tracks_by_album_artist {
            tracks.sort_by(|left, right| {
                Self::compare_artist_tracks_with(
                    album_ids,
                    album_titles,
                    file_name_ids,
                    *left,
                    *right,
                    strings,
                )
            });
        }

        for tracks in &mut self.indexes.tracks_by_composer {
            tracks.sort_by(|left, right| {
                Self::compare_artist_tracks_with(
                    album_ids,
                    album_titles,
                    file_name_ids,
                    *left,
                    *right,
                    strings,
                )
            });
        }
    }

    fn rebuild_directories(&mut self, strings: &mut StringInterner) {
        let root_name = strings.intern("/");
        self.directories = DirectoryTree {
            nodes: vec![DirectoryNode {
                name: root_name,
                children: Vec::new(),
                tracks: Vec::new(),
            }],
            root: DirectoryNodeId(0),
        };

        let mut child_lookup: HashMap<(u32, StrId), DirectoryNodeId> = HashMap::new();
        let file_name_ids = self.tracks.file_name_ids.clone();

        for track_index in 0..self.tracks.titles.len() {
            let track_id = TrackId(track_index as u32);
            let Some(full_path) = self.track_path(track_id).map(str::to_owned) else {
                continue;
            };

            let mut current = self.directories.root;
            if let Some(parent) = Path::new(&full_path).parent() {
                for component in parent.components() {
                    let component_name = component.as_os_str().to_string_lossy().trim().to_string();
                    if component_name.is_empty() || component_name == "/" || component_name == "\\" {
                        continue;
                    }

                    let name_id = strings.intern(component_name);
                    let key = (current.0, name_id);
                    let next = if let Some(existing) = child_lookup.get(&key) {
                        *existing
                    } else {
                        let node_id = DirectoryNodeId(self.directories.nodes.len() as u32);
                        self.directories.nodes.push(DirectoryNode {
                            name: name_id,
                            children: Vec::new(),
                            tracks: Vec::new(),
                        });
                        self.directories.nodes[current.0 as usize].children.push(node_id);
                        child_lookup.insert(key, node_id);
                        node_id
                    };
                    current = next;
                }
            }

            self.directories.nodes[current.0 as usize].tracks.push(track_id);
        }

        for node in &mut self.directories.nodes {
            node.tracks.sort_by(|left, right| {
                Self::compare_file_names_with(&file_name_ids, *left, *right, strings)
            });
        }
    }

    fn compare_album_tracks_with(
        track_numbers: &[Option<NonZeroU16>],
        file_name_ids: &[StrId],
        left: TrackId,
        right: TrackId,
        strings: &StringInterner,
    ) -> Ordering {
        let left_index = left.0 as usize;
        let right_index = right.0 as usize;
        let left_number = track_numbers[left_index];
        let right_number = track_numbers[right_index];

        match (left_number, right_number) {
            (Some(left_number), Some(right_number)) => left_number
                .cmp(&right_number)
                .then_with(|| Self::compare_file_names_with(file_name_ids, left, right, strings)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Self::compare_file_names_with(file_name_ids, left, right, strings),
        }
    }

    fn compare_artist_tracks_with(
        album_ids: &[Option<AlbumId>],
        album_titles: &[StrId],
        file_name_ids: &[StrId],
        left: TrackId,
        right: TrackId,
        strings: &StringInterner,
    ) -> Ordering {
        let left_album = album_ids
            .get(left.0 as usize)
            .and_then(|album_id| album_id.and_then(|id| album_titles.get(id.0 as usize).copied()))
            .and_then(|title_id| strings.resolve(title_id))
            .unwrap_or_else(|| "Unknown".to_string());
        let right_album = album_ids
            .get(right.0 as usize)
            .and_then(|album_id| album_id.and_then(|id| album_titles.get(id.0 as usize).copied()))
            .and_then(|title_id| strings.resolve(title_id))
            .unwrap_or_else(|| "Unknown".to_string());

        left_album
            .cmp(&right_album)
            .then_with(|| Self::compare_file_names_with(file_name_ids, left, right, strings))
    }

    fn compare_file_names_with(
        file_name_ids: &[StrId],
        left: TrackId,
        right: TrackId,
        strings: &StringInterner,
    ) -> Ordering {
        let left_name = file_name_ids
            .get(left.0 as usize)
            .and_then(|id| strings.resolve(*id))
            .unwrap_or_default();
        let right_name = file_name_ids
            .get(right.0 as usize)
            .and_then(|id| strings.resolve(*id))
            .unwrap_or_default();
        left_name.cmp(&right_name)
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::{ArtistKind, Library, TrackInput};
    use crate::core::strings::StringInterner;

    fn track_input(
        title: &str,
        album: &str,
        artist: &str,
        path: &str,
        file_name: &str,
        track_number: u16,
        file_size: u64,
    ) -> TrackInput {
        TrackInput {
            title: title.to_string(),
            album: Some(album.to_string()),
            artist: Some(artist.to_string()),
            album_artist: Some(artist.to_string()),
            composer: None,
            normalized_path: path.to_string(),
            file_size,
            modified_unix_secs: 10,
            file_name: file_name.to_string(),
            track_number: NonZeroU16::new(track_number),
            duration_ms: 1_000,
            artwork_key: None,
        }
    }

    #[test]
    fn upsert_track_reuses_track_id_for_duplicate_key() {
        let mut library = Library::default();
        let mut strings = StringInterner::default();

        let first = library.upsert_track(
            &mut strings,
            track_input(
                "Track A",
                "Album X",
                "Artist A",
                "/music/album_x/01-track-a.flac",
                "01-track-a.flac",
                1,
                100,
            ),
        );
        let second = library.upsert_track(
            &mut strings,
            track_input(
                "Track A (Updated)",
                "Album X",
                "Artist A",
                "/music/album_x/01-track-a.flac",
                "01-track-a.flac",
                1,
                100,
            ),
        );

        assert_eq!(first, second);
        assert_eq!(library.tracks.stable_keys.len(), 1);
        assert_eq!(library.track_path(first), Some("/music/album_x/01-track-a.flac"));
    }

    #[test]
    fn album_and_artist_indexes_follow_spec_sorting() {
        let mut library = Library::default();
        let mut strings = StringInterner::default();

        let second_track = library.upsert_track(
            &mut strings,
            track_input(
                "Second",
                "Album X",
                "Artist A",
                "/music/album_x/02-second.flac",
                "02-second.flac",
                2,
                200,
            ),
        );
        let first_track = library.upsert_track(
            &mut strings,
            track_input(
                "First",
                "Album X",
                "Artist A",
                "/music/album_x/01-first.flac",
                "01-first.flac",
                1,
                201,
            ),
        );

        assert_eq!(library.visible_tracks_for_album(crate::core::ids::AlbumId(0)), &[first_track, second_track]);
        assert_eq!(
            library.visible_tracks_for_artist(ArtistKind::TrackArtist, crate::core::ids::ArtistId(0)),
            &[first_track, second_track]
        );
        assert_eq!(library.directories.nodes.len() >= 2, true);
    }
}