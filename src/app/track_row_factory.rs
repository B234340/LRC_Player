//! TrackRowFactory: centralizes track-row static/dynamic construction and caching.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use crate::core::ids::TrackId;
use crate::core::library::Library;
use crate::core::playlists::PlaylistStore;
use crate::core::playback::PlaybackModel;
use crate::core::queue::Queue;
use crate::core::strings::StringInterner;

use crate::app::view_models::{FavoriteStateVm, TrackRowStaticVm, TrackRowVm};

pub struct TrackRowFactory<'a> {
    library: &'a Library,
    playlists: &'a PlaylistStore,
    playback: &'a PlaybackModel,
    strings: &'a StringInterner,
    cache: &'a RefCell<HashMap<TrackId, TrackRowStaticVm>>,
}

impl<'a> TrackRowFactory<'a> {
    pub fn new(
        library: &'a Library,
        playlists: &'a PlaylistStore,
        playback: &'a PlaybackModel,
        strings: &'a StringInterner,
        cache: &'a RefCell<HashMap<TrackId, TrackRowStaticVm>>,
    ) -> Self {
        Self {
            library,
            playlists,
            playback,
            strings,
            cache,
        }
    }

    pub fn build(&self, track_id: TrackId, queue: &Queue, queue_index: Option<usize>) -> TrackRowVm {
        // If we have cached static data, build a dynamic VM from it
        if let Some(static_vm) = self.cache.borrow().get(&track_id) {
            return TrackRowVm {
                track_id: static_vm.track_id,
                title: static_vm.title.clone(),
                artist: static_vm.artist.clone(),
                album: static_vm.album.clone(),
                artwork: static_vm.artwork.clone(),
                favorite_state: if self.playlists.contains_in_favorite(track_id) {
                    FavoriteStateVm::Included
                } else {
                    FavoriteStateVm::Excluded
                },
                is_current: self.playback.current_track == Some(track_id),
                is_selected: queue_index
                    .and_then(|index| queue.entries.get(index))
                    .map(|entry| queue.selected.contains(&entry.track_id))
                    .unwrap_or(false),
            };
        }

        // Build static parts and cache them
        let track_index = track_id.0 as usize;
        let title_str = self
            .library
            .tracks
            .titles
            .get(track_index)
            .and_then(|id| self.strings.resolve(*id))
            .unwrap_or_else(|| format!("Track {}", track_id.0));
        let artist_str = self
            .library
            .tracks
            .artist_ids
            .get(track_index)
            .and_then(|artist_id| artist_id.and_then(|id| self.library.artist_name(id, &self.strings)))
            .unwrap_or_else(|| "Unknown".to_string());
        let album_str = self
            .library
            .tracks
            .album_ids
            .get(track_index)
            .and_then(|album_id| album_id.and_then(|id| self.library.album_title(id, &self.strings)))
            .unwrap_or_else(|| "Unknown".to_string());

        let title_arc: Arc<str> = Arc::from(title_str.into_boxed_str());
        let artist_arc: Arc<str> = Arc::from(artist_str.into_boxed_str());
        let album_arc: Arc<str> = Arc::from(album_str.into_boxed_str());

        let static_vm = TrackRowStaticVm {
            track_id,
            title: title_arc.clone(),
            artist: artist_arc.clone(),
            album: album_arc.clone(),
            artwork: None,
        };

        self.cache.borrow_mut().insert(track_id, static_vm);

        // Build dynamic VM from static parts
        TrackRowVm {
            track_id,
            title: title_arc,
            artist: artist_arc,
            album: album_arc,
            artwork: None,
            favorite_state: if self.playlists.contains_in_favorite(track_id) {
                FavoriteStateVm::Included
            } else {
                FavoriteStateVm::Excluded
            },
            is_current: self.playback.current_track == Some(track_id),
            is_selected: queue_index
                .and_then(|index| queue.entries.get(index))
                .map(|entry| queue.selected.contains(&entry.track_id))
                .unwrap_or(false),
        }
    }
}
