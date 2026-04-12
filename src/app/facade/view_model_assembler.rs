//! ViewModel assembler: builds the `AppViewModel` snapshot from facade state.
//!
// This module is a child of the `facade` module so it has access to
// `AppFacade`'s private fields and helper methods. It centralizes the
// top-level view-model assembly so `facade.rs` can delegate the
// presentation responsibilities and remain smaller.

use crate::app::view_models::AppViewModel;

/// Build (and assign) the `AppViewModel` snapshot from the given facade state.
pub fn view_model(facade: &mut super::AppFacade) -> &crate::app::view_models::AppViewModel {
    let prev_selected_album = facade.selected_album;
    let prev_selected_artist = facade.selected_artist;

    facade.ensure_browse_cache_current();

    if prev_selected_album != facade.selected_album || prev_selected_artist != facade.selected_artist {
        eprintln!(
            "selection_overwrite: prev_album={:?} prev_artist={:?} new_album={:?} new_artist={:?}",
            prev_selected_album,
            prev_selected_artist,
            facade.selected_album,
            facade.selected_artist
        );
    }

    // Ensure cached heavy sub-VMs exist in the `VmCache`.
    if facade.cache.album_vm.is_none() {
        facade.cache.album_vm = Some(facade.album_tab_vm());
    }
    if facade.cache.artist_vm.is_none() {
        facade.cache.artist_vm = Some(facade.artist_tab_vm());
    }

    // Determine if any fine-grained dirty flags require a top-level snapshot rebuild.
    let needs_rebuild = facade.app_view_model.is_none()
        || facade.cache.album_vm.is_none()
        || facade.cache.artist_vm.is_none()
        || facade.dirty_queue
        || facade.dirty_play
        || facade.dirty_playlist
        || facade.dirty_library
        || facade.dirty_scan
        || facade.dirty_settings
        || facade.view_model_dirty;

    if needs_rebuild {
        let prev_vm = facade.app_view_model.as_ref();
        let queue_vm = if prev_vm.is_none() || facade.dirty_queue || facade.current_tab == crate::core::persistence::TabId::Queue {
            facade.queue_tab_vm()
        } else {
            prev_vm.unwrap().queue.clone()
        };

        let play_vm = if prev_vm.is_none() || facade.dirty_play || facade.current_tab == crate::core::persistence::TabId::Play {
            facade.play_tab_vm()
        } else {
            prev_vm.unwrap().play.clone()
        };

        let album_vm = facade.cache.album_vm.clone().unwrap();
        let artist_vm = facade.cache.artist_vm.clone().unwrap();

        let playlist_vm = if prev_vm.is_none() || facade.dirty_playlist || facade.current_tab == crate::core::persistence::TabId::Playlist {
            facade.playlist_tab_vm()
        } else {
            prev_vm.unwrap().playlist.clone()
        };

        let library_vm = if prev_vm.is_none() || facade.dirty_library || facade.current_tab == crate::core::persistence::TabId::Library {
            facade.library_tab_vm()
        } else {
            prev_vm.unwrap().library.clone()
        };

        let scan_vm = if prev_vm.is_none() || facade.dirty_scan || facade.current_tab == crate::core::persistence::TabId::Scan {
            facade.scan_tab_vm()
        } else {
            prev_vm.unwrap().scan.clone()
        };

        let settings_vm = if prev_vm.is_none() || facade.dirty_settings || facade.current_tab == crate::core::persistence::TabId::Settings {
            facade.settings_tab_vm()
        } else {
            prev_vm.unwrap().settings.clone()
        };

        let vm = AppViewModel {
            current_tab: facade.current_tab,
            queue: queue_vm,
            play: play_vm,
            album: album_vm,
            artist: artist_vm,
            playlist: playlist_vm,
            library: library_vm,
            scan: scan_vm,
            settings: settings_vm,
        };

        facade.app_view_model = Some(vm);
        // clear fine-grained dirty flags now that we've rebuilt affected VMs
        facade.dirty_queue = false;
        facade.dirty_play = false;
        facade.dirty_playlist = false;
        facade.dirty_library = false;
        facade.dirty_scan = false;
        facade.dirty_settings = false;
        facade.view_model_dirty = false;
    }

    // Safe to unwrap because we ensure Some above.
    facade.app_view_model.as_ref().unwrap()
}
