//! Album VM builder extracted from `AppFacade` to reduce facade size.
//!
// This module is a child of `facade` so it can access private fields/methods.

use crate::app::view_models::AlbumTabVm;

pub fn build_album_tab(facade: &super::AppFacade) -> AlbumTabVm {
    let visible_tracks = facade
        .selected_album
        .map(|album_id| {
            facade.track_rows_for_tracks(facade.library.visible_tracks_for_album(album_id))
        })
        .unwrap_or_default();

    // Compute per-album track hydration progress
    let (tracks_loading, tracks_loaded, tracks_total) = if let Some(album_id) = facade.selected_album {
        let idx = album_id.0 as usize;
        let total = facade
            .album_index
            .get(idx)
            .map(|a| a.tracks.len())
            .or_else(|| facade.library.indexes.tracks_by_album.get(idx).map(|v| v.len()))
            .unwrap_or(0);
        let loaded = facade
            .library
            .indexes
            .tracks_by_album
            .get(idx)
            .map(|v| v.len())
            .unwrap_or(0);
        (facade.browse_loader.is_some() && loaded < total, loaded, total)
    } else {
        (false, 0, 0)
    };

    AlbumTabVm {
        albums: facade.browse_cache.albums.visible_entries(),
        expanded_album: facade.selected_album,
        visible_tracks,
        loading: facade.browse_cache.albums.loading_state(),
        tracks_loading,
        tracks_loaded,
        tracks_total,
    }
}
