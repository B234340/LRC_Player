use crate::application::dto::{LibraryDto, PlaylistDto, SettingsDto, StateDto};

use super::{LibraryVm, PlaylistVm, QueueVm, ScanVm, SettingsVm};

#[derive(Debug, Clone)]
pub struct AppVm {
    pub title: String,
    pub active_tab: Option<String>,
    pub settings: SettingsVm,
    pub library: LibraryVm,
    pub playlists: Vec<PlaylistVm>,
    pub scan: ScanVm,
    pub queue: QueueVm,
}

impl AppVm {
    pub fn new(
        title: impl Into<String>,
        settings: SettingsDto,
        state: StateDto,
        playlists: &[PlaylistDto],
        library: &LibraryDto,
        queue: QueueVm,
    ) -> Self {
        Self {
            title: title.into(),
            active_tab: state.last_tab.clone(),
            settings: SettingsVm::from(&settings),
            library: LibraryVm::from(library),
            playlists: playlists.iter().map(PlaylistVm::from).collect(),
            scan: ScanVm::from(&state),
            queue,
        }
    }
}
