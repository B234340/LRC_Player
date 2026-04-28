#[cfg(test)]
mod tests {
    use lrc_player::application::view_model::QueueVm;
    use lrc_player::application::usecases::build_app_vm::*;
    use lrc_player::application::dto::{LibraryDto, PlaylistDto, SettingsDto, StateDto};
    use lrc_player::domain::model::PlaybackMode;
    use lrc_player::presentation::app::ArtistViewMode;

    #[test]
    fn builds_app_vm_from_dtos() {
        let settings = SettingsDto::new(Some("USB Audio".into()), 64);
        let state = StateDto {
            last_tab: Some("library".into()),
            last_play_path: None,
            last_play_position_ms: Some(10_000),
            last_scan_status: Some("library".into()),
            scan_scanned: Some(10),
            scan_parsed: Some(8),
            scan_registered: Some(4),
            scan_targets: vec!["/tmp/music".into()],
            scan_excludes: vec!["/tmp/music/temp".into()],
            queue_tracks: vec!["/tmp/track1.mp3".into(), "/tmp/track2.mp3".into()],
            selected_queue_index: Some(0),
            scan_lrc_targets: Vec::new(),
            queue_playback_mode: PlaybackMode::Sequential,
            artist_view_mode: ArtistViewMode::Artist,
            lrc_overlay_enabled: false,
            fft_overlay_enabled: false,
            side_panel_expanded: true,
            favorite_directories: Vec::new(),
        };
        let playlists = vec![PlaylistDto {
            id: "pl1".into(),
            name: "Favorites".into(),
            tracks: Vec::new(),
        }];
        let library = LibraryDto {
            album_count: 2,
            track_count: 42,
            recent_locations: vec![],
            recent_directories: vec![],
            directory_tree: vec![],
            favorite_directories: vec!["/tmp/music".into()],
            albums: vec!["Album X".into(), "Album Y".into()],
            selected_album: None,
            selected_album_tracks: Vec::new(),
            artists: vec!["Artist A".into(), "Artist B".into()],
            selected_artist: None,
            selected_artist_tracks: Vec::new(),
            selected_directory: None,
            selected_directory_tracks: Vec::new(),
        };
        let queue = QueueVm::new(vec!["track1".into(), "track2".into()]);

        let vm = build_app_vm("LRC Player", &settings, &state, &playlists, &library, queue);

        assert_eq!(vm.title, "LRC Player");
        assert_eq!(vm.active_tab.as_deref(), Some("library"));
        assert_eq!(
            vm.settings.preferred_output_device.as_deref(),
            Some("USB Audio")
        );
        assert_eq!(vm.settings.resampler_taps, 64);
        assert_eq!(vm.library.track_count, 42);
        assert_eq!(vm.playlists.len(), 1);
        assert_eq!(vm.playlists[0].name, "Favorites");
        assert_eq!(vm.scan.last_scan_status.as_deref(), Some("library"));
        assert_eq!(vm.queue.tracks.len(), 2);
    }
}
