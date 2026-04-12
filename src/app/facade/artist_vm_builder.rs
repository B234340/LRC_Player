//! Artist VM builder extracted from `AppFacade` to reduce facade size.
//!
// This module is a child of `facade` so it can access private fields/methods.

use crate::app::view_models::ArtistTabVm;
use crate::core::library::ArtistKind;

pub fn build_artist_tab(facade: &super::AppFacade) -> ArtistTabVm {
    let visible_tracks = facade
        .selected_artist
        .map(|artist_id| {
            facade.track_rows_for_tracks(
                facade
                    .library
                    .visible_tracks_for_artist(facade.selected_artist_kind, artist_id),
            )
        })
        .unwrap_or_default();

    let (artists, loading) = match facade.selected_artist_kind {
        ArtistKind::TrackArtist => (
            facade.browse_cache.track_artists.visible_entries(),
            facade.browse_cache.track_artists.loading_state(),
        ),
        ArtistKind::AlbumArtist => (
            facade.browse_cache.album_artists.visible_entries(),
            facade.browse_cache.album_artists.loading_state(),
        ),
        ArtistKind::Composer => (
            facade.browse_cache.composers.visible_entries(),
            facade.browse_cache.composers.loading_state(),
        ),
    };

    // Compute per-artist track hydration progress
    let (tracks_loading, tracks_loaded, tracks_total) = if let Some(artist_id) = facade.selected_artist {
        let idx = artist_id.0 as usize;
        let loaded = facade
            .library
            .indexes
            .tracks_by_artist
            .get(idx)
            .map(|v| v.len())
            .unwrap_or(0);
        // try to resolve total from persisted index entries; fallback to library index
        let total = match facade.selected_artist_kind {
            ArtistKind::TrackArtist => facade
                .artist_index
                .track_artists
                .iter()
                .find(|e| facade.library.artist_name(artist_id, &facade.strings) == Some(e.name.clone()))
                .map(|e| e.tracks.len())
                .or_else(|| facade.library.indexes.tracks_by_artist.get(idx).map(|v| v.len()))
                .unwrap_or(0),
            ArtistKind::AlbumArtist => facade
                .artist_index
                .album_artists
                .iter()
                .find(|e| facade.library.artist_name(artist_id, &facade.strings) == Some(e.name.clone()))
                .map(|e| e.tracks.len())
                .or_else(|| facade.library.indexes.tracks_by_album_artist.get(idx).map(|v| v.len()))
                .unwrap_or(0),
            ArtistKind::Composer => facade
                .artist_index
                .composers
                .iter()
                .find(|e| facade.library.artist_name(artist_id, &facade.strings) == Some(e.name.clone()))
                .map(|e| e.tracks.len())
                .or_else(|| facade.library.indexes.tracks_by_composer.get(idx).map(|v| v.len()))
                .unwrap_or(0),
        };

        (facade.browse_loader.is_some() && loaded < total, loaded, total)
    } else {
        (false, 0, 0)
    };

    ArtistTabVm {
        kind: facade.selected_artist_kind,
        artists,
        selected_artist: facade.selected_artist,
        visible_tracks,
        loading,
        tracks_loading,
        tracks_loaded,
        tracks_total,
    }
}
