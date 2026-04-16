use crate::application::dto::{LibraryDto, PlaylistDto, SettingsDto, StateDto};
use crate::application::view_model::{AppVm, QueueVm};

/// Build a UI-ready `AppVm` from application data transfer objects.
pub fn build_app_vm(
    title: impl Into<String>,
    settings: &SettingsDto,
    state: &StateDto,
    playlists: &[PlaylistDto],
    library: &LibraryDto,
    queue: QueueVm,
) -> AppVm {
    AppVm::new(
        title,
        settings.clone(),
        state.clone(),
        playlists,
        library,
        queue,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::dto::{LibraryDto, PlaylistDto, SettingsDto, StateDto};
    use crate::application::view_model::QueueVm;

    #[test]
    fn builds_app_vm_from_dtos() {
        let settings = SettingsDto::new(Some("USB Audio".into()), 64);
        let state = StateDto {
            last_tab: Some("library".into()),
        };
        let playlists = vec![PlaylistDto {
            id: "pl1".into(),
            name: "Favorites".into(),
        }];
        let library = LibraryDto {
            album_count: 2,
            track_count: 42,
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
