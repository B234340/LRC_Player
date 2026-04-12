// Playlist VM builder: extracted from `AppFacade` to reduce facade size.
use std::sync::Arc;

use crate::app::view_models::{PlaylistEntryVm, PlaylistTabVm};

pub fn build_playlist_tab(facade: &super::AppFacade) -> PlaylistTabVm {
    let playlists = facade
        .playlists
        .playlists
        .iter()
        .map(|playlist| PlaylistEntryVm {
            playlist_id: playlist.id,
            name: Arc::from(
                facade
                    .strings
                    .resolve(playlist.name)
                    .unwrap_or_else(|| format!("playlist-{}", playlist.id.0))
                    .into_boxed_str(),
            ),
            track_count: playlist.tracks.len(),
            is_system: playlist.kind == crate::core::playlists::PlaylistKind::SystemFavorite,
        })
        .collect();

    let visible_tracks = facade
        .playlists
        .selected
        .and_then(|selected| {
            facade
                .playlists
                .playlists
                .iter()
                .find(|playlist| playlist.id == selected)
                .map(|playlist| facade.track_rows_for_tracks(&playlist.tracks))
        })
        .unwrap_or_default();

    PlaylistTabVm {
        playlists,
        selected_playlist: facade.playlists.selected,
        visible_tracks,
    }
}
