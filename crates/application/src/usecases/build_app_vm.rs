use crate::dto::{LibraryDto, PlaylistDto, SettingsDto, StateDto};
use crate::view_model::{AppVm, QueueVm};

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


