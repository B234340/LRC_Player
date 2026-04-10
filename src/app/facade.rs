use crate::app::commands::{AppCommand, TrackListOrigin};
use crate::app::events::{AppEvent, PlaybackEvent};
use crate::app::view_models::{
    AppViewModel, FavoriteStateVm, PlayTabVm, QueueTabVm, ScanTabVm, TrackRowVm,
};
use crate::core::ids::{StrId, TrackId};
use crate::core::library::Library;
use crate::core::persistence::TabId;
use crate::core::playback::PlaybackModel;
use crate::core::playlists::PlaylistStore;
use crate::core::queue::Queue;
use crate::core::scan::{ScanCommand, ScanModel, ScanState};
use crate::core::strings::StringInterner;

pub struct AppFacade {
    pub strings: StringInterner,
    pub library: Library,
    pub queue: Queue,
    pub playlists: PlaylistStore,
    pub playback: PlaybackModel,
    pub scan: ScanModel,
    pub current_tab: TabId,
}

impl Default for AppFacade {
    fn default() -> Self {
        let mut strings = StringInterner::default();
        let favorite_name = strings.intern("favorite");

        Self {
            strings,
            library: Library::default(),
            queue: Queue::default(),
            playlists: PlaylistStore::new(favorite_name),
            playback: PlaybackModel::default(),
            scan: ScanModel::default(),
            current_tab: TabId::Queue,
        }
    }
}

impl AppFacade {
    pub fn bootstrap_summary(&self) -> String {
        format!(
            "lrc_player initialized: tab={:?}, queue_entries={}, playlists={}, scan_state={:?}",
            self.current_tab,
            self.queue.entries.len(),
            self.playlists.playlists.len(),
            self.scan.state
        )
    }

    pub fn load_demo_state(&mut self) {
        if self.playlists.selected.is_none() {
            self.playlists.selected = Some(self.playlists.favorite_playlist);
        }
    }

    pub fn handle_command(&mut self, command: AppCommand) -> Vec<AppEvent> {
        match command {
            AppCommand::ClickTrack {
                visible_tracks,
                clicked_index,
                origin,
            } => self.handle_click_track(visible_tracks, clicked_index, origin),
            AppCommand::PlayNext { track_id } => {
                self.queue.insert_after_current(track_id);
                vec![AppEvent::QueueChanged]
            }
            AppCommand::AppendToQueue { track_id } => {
                self.queue.push_track(track_id);
                vec![AppEvent::QueueChanged]
            }
            AppCommand::AddToPlaylist {
                playlist_id,
                track_id,
            } => {
                self.playlists.add_track(playlist_id, track_id);
                vec![AppEvent::PlaylistChanged]
            }
            AppCommand::ToggleFavorite { track_id } => {
                self.playlists.toggle_favorite(track_id);
                vec![AppEvent::PlaylistChanged]
            }
            AppCommand::ShowTrackDetails { track_id } => {
                vec![AppEvent::TrackDetailsRequested(track_id)]
            }
            AppCommand::QueueRemoveByContext { entry_id } => {
                self.queue.remove_entry(entry_id);
                self.sync_playback_from_queue()
            }
            AppCommand::TogglePlayPause => {
                self.playback.toggle_pause();
                vec![AppEvent::Playback(PlaybackEvent::StateChanged(
                    self.playback.state,
                ))]
            }
            AppCommand::SeekBy { delta_ms } => {
                self.playback.seek_by(delta_ms);
                vec![AppEvent::Playback(PlaybackEvent::PositionUpdated {
                    position_ms: self.playback.position_ms,
                    buffered_ms: self.playback.buffered_ms,
                })]
            }
            AppCommand::SeekTo { position_ms } => {
                self.playback.seek_to(position_ms);
                vec![AppEvent::Playback(PlaybackEvent::PositionUpdated {
                    position_ms: self.playback.position_ms,
                    buffered_ms: self.playback.buffered_ms,
                })]
            }
            AppCommand::NextTrack => {
                self.queue.advance();
                self.sync_playback_from_queue()
            }
            AppCommand::PreviousTrack => {
                self.queue.previous();
                self.sync_playback_from_queue()
            }
            AppCommand::SelectTab(tab) => {
                if self.current_tab == TabId::Scan && tab != TabId::Scan {
                    self.scan.reset_if_terminal();
                }
                self.current_tab = tab;
                vec![AppEvent::TabChanged(tab)]
            }
            AppCommand::Scan(scan_command) => self.handle_scan_command(scan_command),
            AppCommand::CreatePlaylist { name } => {
                let name_id = self.strings.intern(name);
                let _ = self.playlists.create_user_playlist(name_id);
                vec![AppEvent::PlaylistChanged]
            }
        }
    }

    pub fn view_model(&self) -> AppViewModel {
        AppViewModel {
            current_tab: self.current_tab,
            queue: self.queue_tab_vm(),
            play: self.play_tab_vm(),
            scan: self.scan_tab_vm(),
        }
    }

    fn handle_click_track(
        &mut self,
        visible_tracks: Vec<TrackId>,
        clicked_index: usize,
        origin: TrackListOrigin,
    ) -> Vec<AppEvent> {
        if clicked_index >= visible_tracks.len() {
            return Vec::new();
        }

        match origin {
            TrackListOrigin::Queue => {
                self.queue.set_current_index(clicked_index);
            }
            _ => {
                self.queue.replace_with_tracks(&visible_tracks);
                self.queue.set_current_index(clicked_index);
            }
        }

        self.sync_playback_from_queue()
    }

    fn handle_scan_command(&mut self, command: ScanCommand) -> Vec<AppEvent> {
        self.scan.apply(command);
        vec![AppEvent::ScanStateChanged(self.scan.state)]
    }

    fn sync_playback_from_queue(&mut self) -> Vec<AppEvent> {
        match self.queue.current_track() {
            Some(track_id) => {
                self.playback.play_track(track_id);
                vec![
                    AppEvent::QueueChanged,
                    AppEvent::Playback(PlaybackEvent::TrackChanged(Some(track_id))),
                    AppEvent::Playback(PlaybackEvent::StateChanged(self.playback.state)),
                ]
            }
            None => {
                self.playback.stop();
                vec![
                    AppEvent::QueueChanged,
                    AppEvent::Playback(PlaybackEvent::TrackChanged(None)),
                    AppEvent::Playback(PlaybackEvent::StateChanged(self.playback.state)),
                ]
            }
        }
    }

    fn queue_tab_vm(&self) -> QueueTabVm {
        QueueTabVm {
            rows: self
                .queue
                .entries
                .iter()
                .enumerate()
                .map(|(index, entry)| self.track_row_vm(entry.track_id, Some(index)))
                .collect(),
            repeat_mode: self.queue.repeat_mode,
        }
    }

    fn play_tab_vm(&self) -> PlayTabVm {
        PlayTabVm {
            current_track: self
                .playback
                .current_track
                .map(|track_id| self.track_row_vm(track_id, None)),
            playback_state: self.playback.state,
            position_ms: self.playback.position_ms,
        }
    }

    fn scan_tab_vm(&self) -> ScanTabVm {
        ScanTabVm {
            include_dirs: self
                .scan
                .config
                .include_dirs
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            exclude_dirs: self
                .scan
                .config
                .exclude_dirs
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            visible_status: self.scan.visible_status(),
            cancel_visible: self.scan.state == ScanState::Scanning,
        }
    }

    fn track_row_vm(&self, track_id: TrackId, queue_index: Option<usize>) -> TrackRowVm {
        let track_index = track_id.0 as usize;
        let title = self
            .library
            .tracks
            .titles
            .get(track_index)
            .and_then(|id| self.strings.resolve(*id))
            .unwrap_or_else(|| format!("Track {}", track_id.0));
        let artist = self
            .library
            .tracks
            .artist_ids
            .get(track_index)
            .and_then(|artist_id| artist_id.and_then(|id| self.library.artist_name(id, &self.strings)))
            .unwrap_or_else(|| "Unknown".to_string());
        let album = self
            .library
            .tracks
            .album_ids
            .get(track_index)
            .and_then(|album_id| album_id.and_then(|id| self.library.album_title(id, &self.strings)))
            .unwrap_or_else(|| "Unknown".to_string());

        TrackRowVm {
            track_id,
            title,
            artist,
            album,
            artwork: None,
            favorite_state: if self.playlists.contains_in_favorite(track_id) {
                FavoriteStateVm::Included
            } else {
                FavoriteStateVm::Excluded
            },
            is_current: self.playback.current_track == Some(track_id),
            is_selected: queue_index
                .and_then(|index| self.queue.entries.get(index))
                .map(|entry| self.queue.selected.contains(&entry.id))
                .unwrap_or(false),
        }
    }
}

impl From<StrId> for String {
    fn from(value: StrId) -> Self {
        value.0.to_string()
    }
}