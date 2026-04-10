use serde::{Deserialize, Serialize};

use crate::core::ids::{PlaylistId, StrId, TrackId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaylistKind {
    SystemFavorite,
    UserDefined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: StrId,
    pub tracks: Vec<TrackId>,
    pub kind: PlaylistKind,
}

#[derive(Debug, Clone)]
pub struct PlaylistStore {
    pub playlists: Vec<Playlist>,
    pub selected: Option<PlaylistId>,
    pub favorite_playlist: PlaylistId,
    next_playlist_id: u32,
}

impl PlaylistStore {
    pub fn new(favorite_name: StrId) -> Self {
        Self {
            playlists: vec![Playlist {
                id: PlaylistId(0),
                name: favorite_name,
                tracks: Vec::new(),
                kind: PlaylistKind::SystemFavorite,
            }],
            selected: Some(PlaylistId(0)),
            favorite_playlist: PlaylistId(0),
            next_playlist_id: 1,
        }
    }

    pub fn create_user_playlist(&mut self, name: StrId) -> Result<PlaylistId, &'static str> {
        if self.playlists.iter().any(|playlist| playlist.name == name) {
            return Err("playlist name must be unique");
        }

        let id = PlaylistId(self.next_playlist_id);
        self.next_playlist_id += 1;
        self.playlists.push(Playlist {
            id,
            name,
            tracks: Vec::new(),
            kind: PlaylistKind::UserDefined,
        });
        self.selected = Some(id);
        Ok(id)
    }

    pub fn add_track(&mut self, playlist_id: PlaylistId, track_id: TrackId) {
        if let Some(playlist) = self.playlists.iter_mut().find(|playlist| playlist.id == playlist_id) {
            playlist.tracks.push(track_id);
        }
    }

    pub fn toggle_favorite(&mut self, track_id: TrackId) {
        if let Some(playlist) = self
            .playlists
            .iter_mut()
            .find(|playlist| playlist.id == self.favorite_playlist)
        {
            if let Some(index) = playlist.tracks.iter().position(|existing| *existing == track_id) {
                playlist.tracks.remove(index);
            } else {
                playlist.tracks.push(track_id);
            }
        }
    }

    pub fn contains_in_favorite(&self, track_id: TrackId) -> bool {
        self.playlists
            .iter()
            .find(|playlist| playlist.id == self.favorite_playlist)
            .map(|playlist| playlist.tracks.contains(&track_id))
            .unwrap_or(false)
    }

    pub fn replace_all(
        &mut self,
        playlists: Vec<Playlist>,
        selected: Option<PlaylistId>,
        favorite_playlist: PlaylistId,
    ) {
        let next_playlist_id = playlists
            .iter()
            .map(|playlist| playlist.id.0)
            .max()
            .map(|max_id| max_id + 1)
            .unwrap_or(0);

        self.playlists = playlists;
        self.selected = selected;
        self.favorite_playlist = favorite_playlist;
        self.next_playlist_id = next_playlist_id;
    }
}