use std::io;

use crate::app::commands::{AppCommand, TrackListOrigin};
use crate::app::events::{AppEvent, PlaybackEvent};
use crate::app::view_models::{
    AlbumEntryVm, AlbumTabVm, AppViewModel, ArtistEntryVm, ArtistTabVm, DeferredListStateVm,
    DirectoryEntryVm, FavoriteStateVm, LibraryTabVm, PlayTabVm, PlaylistEntryVm,
    PlaylistTabVm, QueueTabVm, ScanTabVm, SettingItemVm, SettingsTabVm, TrackRowVm,
};
use crate::audio::output::{AudioDeviceOutput, AudioOutputConfig, BufferedAudioOutput};
use crate::core::ids::{AlbumId, ArtistId, StrId, TrackId};
use crate::core::library::{ArtistKind, DirectoryNodeId, Library};
use crate::core::persistence::{AppConfigFile, AppStateFile, JsonStore, PlaylistsFile, TabId};
use crate::core::playback::PlaybackState;
use crate::core::playback::PlaybackModel;
use crate::core::playlists::{PlaylistKind, PlaylistStore};
use crate::core::queue::Queue;
use crate::core::scan::{
    build_library_from_track_inputs, ScanCommand, ScanModel, ScanOutcome, ScanState,
    ScanWorkerHandle, ScanWorkerMessage,
};
use crate::core::strings::StringInterner;
use crate::decode::decoder::{DecodeWorkerHandle, DecodeWorkerMessage, Decoder};
use crate::decode::source::DecodedSource;

const AUDIO_BLOCK_CAPACITY: usize = 8_192;
const BROWSE_HYDRATE_BATCH_SIZE: usize = 64;
const INITIAL_DECODE_PRIME_BLOCKS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
struct AlbumLogEntry {
    album_id: AlbumId,
    title: String,
    album_artist: String,
    track_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArtistLogEntry {
    artist_id: ArtistId,
    name: String,
    track_count: usize,
}

#[derive(Debug, Clone, Default)]
struct DeferredAlbumListState {
    log_entries: Vec<AlbumLogEntry>,
    hydrated_entries: Vec<AlbumEntryVm>,
}

#[derive(Debug, Clone, Default)]
struct DeferredArtistListState {
    log_entries: Vec<ArtistLogEntry>,
    hydrated_entries: Vec<ArtistEntryVm>,
}

#[derive(Debug, Clone, Default)]
struct BrowseCacheState {
    library_revision: u64,
    albums: DeferredAlbumListState,
    track_artists: DeferredArtistListState,
    album_artists: DeferredArtistListState,
    composers: DeferredArtistListState,
}

impl DeferredAlbumListState {
    fn reset(&mut self, log_entries: Vec<AlbumLogEntry>) {
        self.log_entries = log_entries;
        self.hydrated_entries.clear();
    }

    fn hydrate_next_batch(&mut self) {
        let start = self.hydrated_entries.len();
        let end = (start + BROWSE_HYDRATE_BATCH_SIZE).min(self.log_entries.len());
        if start >= end {
            return;
        }

        self.hydrated_entries.extend(self.log_entries[start..end].iter().map(|entry| AlbumEntryVm {
            album_id: entry.album_id,
            title: entry.title.clone(),
            album_artist: entry.album_artist.clone(),
            track_count: entry.track_count,
            is_provisional: false,
        }));
    }

    fn visible_entries(&self) -> Vec<AlbumEntryVm> {
        let mut entries = self.hydrated_entries.clone();
        entries.extend(self.log_entries[self.hydrated_entries.len()..].iter().map(|entry| AlbumEntryVm {
            album_id: entry.album_id,
            title: entry.title.clone(),
            album_artist: entry.album_artist.clone(),
            track_count: entry.track_count,
            is_provisional: true,
        }));
        entries
    }

    fn loading_state(&self) -> DeferredListStateVm {
        DeferredListStateVm {
            total_entries: self.log_entries.len(),
            hydrated_entries: self.hydrated_entries.len(),
            is_loading: self.hydrated_entries.len() < self.log_entries.len(),
        }
    }
}

impl DeferredArtistListState {
    fn reset(&mut self, log_entries: Vec<ArtistLogEntry>) {
        self.log_entries = log_entries;
        self.hydrated_entries.clear();
    }

    fn hydrate_next_batch(&mut self) {
        let start = self.hydrated_entries.len();
        let end = (start + BROWSE_HYDRATE_BATCH_SIZE).min(self.log_entries.len());
        if start >= end {
            return;
        }

        self.hydrated_entries.extend(self.log_entries[start..end].iter().map(|entry| ArtistEntryVm {
            artist_id: entry.artist_id,
            name: entry.name.clone(),
            track_count: entry.track_count,
            is_provisional: false,
        }));
    }

    fn visible_entries(&self) -> Vec<ArtistEntryVm> {
        let mut entries = self.hydrated_entries.clone();
        entries.extend(self.log_entries[self.hydrated_entries.len()..].iter().map(|entry| ArtistEntryVm {
            artist_id: entry.artist_id,
            name: entry.name.clone(),
            track_count: entry.track_count,
            is_provisional: true,
        }));
        entries
    }

    fn loading_state(&self) -> DeferredListStateVm {
        DeferredListStateVm {
            total_entries: self.log_entries.len(),
            hydrated_entries: self.hydrated_entries.len(),
            is_loading: self.hydrated_entries.len() < self.log_entries.len(),
        }
    }
}

pub struct AppFacade {
    pub strings: StringInterner,
    pub library: Library,
    pub queue: Queue,
    pub playlists: PlaylistStore,
    pub playback: PlaybackModel,
    pub scan: ScanModel,
    pub settings: AppConfigFile,
    pub audio_output: BufferedAudioOutput,
    _audio_device: Option<AudioDeviceOutput>,
    decoder: Decoder,
    decode_worker: Option<DecodeWorkerHandle>,
    decode_completion_pending: Option<TrackId>,
    scan_worker: Option<ScanWorkerHandle>,
    pub current_tab: TabId,
    selected_album: Option<AlbumId>,
    selected_artist_kind: ArtistKind,
    selected_artist: Option<ArtistId>,
    selected_directory: Option<DirectoryNodeId>,
    browse_cache: BrowseCacheState,
}

impl Default for AppFacade {
    fn default() -> Self {
        let mut strings = StringInterner::default();
        let favorite_name = strings.intern("favorite");
        let audio_config = if cfg!(test) {
            AudioOutputConfig::default()
        } else {
            AudioDeviceOutput::preferred_config().unwrap_or_else(AudioOutputConfig::default)
        };
        let audio_output = BufferedAudioOutput::new(audio_config, AUDIO_BLOCK_CAPACITY);
        let audio_device = if cfg!(test) {
            None
        } else {
            AudioDeviceOutput::try_start(&audio_output).ok()
        };
        audio_output.set_playback_active(false);

        Self {
            strings,
            library: Library::default(),
            queue: Queue::default(),
            playlists: PlaylistStore::new(favorite_name),
            playback: PlaybackModel::default(),
            scan: ScanModel::default(),
            settings: AppConfigFile::default(),
            audio_output,
            _audio_device: audio_device,
            decoder: Decoder::default(),
            decode_worker: None,
            decode_completion_pending: None,
            scan_worker: None,
            current_tab: TabId::Queue,
            selected_album: None,
            selected_artist_kind: ArtistKind::TrackArtist,
            selected_artist: None,
            selected_directory: None,
            browse_cache: BrowseCacheState::default(),
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

    pub fn save_to_store(&self, store: &JsonStore) -> io::Result<()> {
        let mut config = self.settings.clone();
        config.include_dirs = self.scan.config.include_dirs.clone();
        config.exclude_dirs = self.scan.config.exclude_dirs.clone();
        store.save_config(&config)?;
        store.save_state(&AppStateFile::capture(
            self.current_tab,
            &self.queue,
            &self.playback,
            &self.library,
        ))?;
        store.save_playlists(&PlaylistsFile::capture(
            &self.playlists,
            &self.library,
            &self.strings,
        ))
    }

    pub fn bootstrap_from_store(&mut self, store: &JsonStore) -> io::Result<()> {
        let config = store.load_config()?;
        self.settings = config.clone();
        self.scan.config.include_dirs = config.include_dirs;
        self.scan.config.exclude_dirs = config.exclude_dirs;

        if !self.scan.config.include_dirs.is_empty() {
            let (library, _) = self.scan.run_full_scan(&mut self.strings);
            self.library = library;
        }

        self.restore_loaded_state(store)
    }

    pub fn restore_from_store(&mut self, store: &JsonStore) -> io::Result<()> {
        self.restore_loaded_state(store)
    }

    fn restore_loaded_state(&mut self, store: &JsonStore) -> io::Result<()> {
        let config = store.load_config()?;
        self.settings = config.clone();
        self.scan.config.include_dirs = config.include_dirs;
        self.scan.config.exclude_dirs = config.exclude_dirs;

        let state = store.load_state()?;
        self.current_tab = state.last_tab;
        let restored_queue = state.resolve_queue_track_ids(&self.library);
        self.queue.replace_with_tracks(&restored_queue);
        self.queue.current_index = state
            .current_index
            .filter(|index| (*index as usize) < self.queue.entries.len());
        self.queue.repeat_mode = state.repeat_mode;

        if let Some(track_id) = self.queue.current_track() {
            self.playback.restore_track(track_id, state.playback_position_ms);
            let _ = self.prepare_audio(track_id);
        } else {
            self.playback.stop();
        }
        self.sync_audio_transport();

        let playlist_file = store.load_playlists()?;
        let (playlists, favorite_playlist) =
            playlist_file.restore_playlists(&self.library, &mut self.strings);
        self.playlists
            .replace_all(playlists, Some(favorite_playlist), favorite_playlist);

        Ok(())
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
                self.sync_audio_transport();
                vec![AppEvent::Playback(PlaybackEvent::StateChanged(
                    self.playback.state,
                ))]
            }
            AppCommand::SeekBy { delta_ms } => {
                self.playback.seek_by(delta_ms);
                self.sync_audio_transport();
                vec![AppEvent::Playback(PlaybackEvent::PositionUpdated {
                    position_ms: self.playback.position_ms,
                    buffered_ms: self.playback.buffered_ms,
                })]
            }
            AppCommand::SeekTo { position_ms } => {
                self.playback.seek_to(position_ms);
                self.sync_audio_transport();
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
            AppCommand::SelectAlbum(album_id) => {
                self.selected_album = album_id;
                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::SelectArtistKind(kind) => {
                self.selected_artist_kind = kind;
                self.selected_artist = None;
                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::SelectArtist(artist_id) => {
                self.selected_artist = artist_id;
                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::SelectPlaylist(playlist_id) => {
                self.playlists.selected = playlist_id;
                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::SelectDirectory(node_id) => {
                self.selected_directory = node_id;
                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::SetPreferredOutputDevice { device_name } => {
                self.settings.preferred_output_device = device_name.clone();
                let audio_device = match device_name {
                    Some(ref name) => AudioDeviceOutput::try_start_by_name(&self.audio_output, name).ok(),
                    None => AudioDeviceOutput::try_start(&self.audio_output).ok(),
                };
                self._audio_device = audio_device;
                vec![AppEvent::TabChanged(self.current_tab)]
            }
            AppCommand::Scan(scan_command) => self.handle_scan_command(scan_command),
            AppCommand::CreatePlaylist { name } => {
                let name_id = self.strings.intern(name);
                let _ = self.playlists.create_user_playlist(name_id);
                vec![AppEvent::PlaylistChanged]
            }
        }
    }

    pub fn view_model(&mut self) -> AppViewModel {
        self.ensure_browse_cache_current();

        AppViewModel {
            current_tab: self.current_tab,
            queue: self.queue_tab_vm(),
            play: self.play_tab_vm(),
            album: self.album_tab_vm(),
            artist: self.artist_tab_vm(),
            playlist: self.playlist_tab_vm(),
            library: self.library_tab_vm(),
            scan: self.scan_tab_vm(),
            settings: self.settings_tab_vm(),
        }
    }

    pub fn poll_background_tasks(&mut self) -> Vec<AppEvent> {
        self.ensure_browse_cache_current();
        let mut events = Vec::new();
        let mut finished = None;

        if let Some(worker) = self.scan_worker.as_ref() {
            for message in worker.drain_messages() {
                match message {
                    ScanWorkerMessage::Progress(progress) => {
                        self.scan.progress = progress;
                    }
                    ScanWorkerMessage::Finished(outcome) => {
                        finished = Some(outcome);
                    }
                }
            }
        }

        if let Some(outcome) = finished {
            self.complete_scan(outcome);
            self.scan_worker = None;
            events.push(AppEvent::ScanStateChanged(self.scan.state));
        }

        let mut decode_failed_for = None;
        let mut decode_worker_finished = false;
        if let Some(worker) = self.decode_worker.as_ref() {
            for message in worker.drain_messages() {
                match message {
                    DecodeWorkerMessage::SourceExhausted { track_id } => {
                        self.decode_completion_pending = Some(track_id);
                        decode_worker_finished = true;
                    }
                    DecodeWorkerMessage::Failed { track_id, .. } => {
                        decode_failed_for = Some(track_id);
                        decode_worker_finished = true;
                    }
                }
            }
        }
        if decode_worker_finished {
            self.decode_worker = None;
        }

        if let Some(track_id) = decode_failed_for {
            if self.playback.current_track == Some(track_id) && self.audio_output.pending_samples() == 0 {
                events.extend(self.advance_after_track_end());
            }
        }

        if let Some(track_id) = self.decode_completion_pending {
            if self.playback.current_track == Some(track_id) && self.audio_output.pending_samples() == 0 {
                self.decode_completion_pending = None;
                events.extend(self.advance_after_track_end());
            }
        }

        self.hydrate_browse_lists();

        events
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
        match command {
            ScanCommand::StartFullScan => {
                if self.scan_worker.is_none() {
                    self.scan.apply(ScanCommand::StartFullScan);
                    self.scan_worker = Some(ScanWorkerHandle::spawn(self.scan.config.clone()));
                }
                vec![AppEvent::ScanStateChanged(self.scan.state)]
            }
            ScanCommand::Stop => {
                if let Some(worker) = self.scan_worker.as_ref() {
                    worker.cancel();
                }
                self.scan.apply(ScanCommand::Stop);
                vec![AppEvent::ScanStateChanged(self.scan.state)]
            }
            other => {
                self.scan.apply(other);
                vec![AppEvent::ScanStateChanged(self.scan.state)]
            }
        }
    }

    fn sync_playback_from_queue(&mut self) -> Vec<AppEvent> {
        self.cancel_decode_worker();

        let queue_len = self.queue.entries.len();
        if queue_len == 0 {
            self.playback.stop();
            self.sync_audio_transport();
            return vec![
                AppEvent::QueueChanged,
                AppEvent::Playback(PlaybackEvent::TrackChanged(None)),
                AppEvent::Playback(PlaybackEvent::StateChanged(self.playback.state)),
            ];
        }

        let original_index = self.queue.current_index;
        for attempt in 0..queue_len {
            let Some(track_id) = self.queue.current_track() else {
                break;
            };

            self.playback.play_track(track_id);
            if self.prepare_audio(track_id).is_ok() && self.audio_output.buffered_blocks() > 0 {
                self.sync_audio_transport();
                return vec![
                    AppEvent::QueueChanged,
                    AppEvent::Playback(PlaybackEvent::TrackChanged(Some(track_id))),
                    AppEvent::Playback(PlaybackEvent::StateChanged(self.playback.state)),
                    AppEvent::Playback(PlaybackEvent::PositionUpdated {
                        position_ms: self.playback.position_ms,
                        buffered_ms: self.playback.buffered_ms,
                    }),
                ];
            }

            if attempt + 1 < queue_len {
                self.queue.advance();
            }
        }

        self.queue.current_index = original_index;
        self.audio_output.clear();
        self.playback.stop();
        self.sync_audio_transport();
        vec![
            AppEvent::QueueChanged,
            AppEvent::Playback(PlaybackEvent::TrackChanged(None)),
            AppEvent::Playback(PlaybackEvent::StateChanged(self.playback.state)),
        ]
    }

    fn prepare_audio(&mut self, track_id: TrackId) -> io::Result<()> {
        self.cancel_decode_worker();
        self.audio_output.clear();
        let Some(source) = DecodedSource::from_library(track_id, &self.library) else {
            self.playback.buffered_ms = 0;
            return Ok(());
        };

        let initial_blocks = self.decoder.decode_blocks(
            &source,
            self.audio_output.config(),
            INITIAL_DECODE_PRIME_BLOCKS.min(self.audio_output.block_capacity()),
        )?;
        let initial_count = self.audio_output.try_enqueue_many(initial_blocks);
        self.decode_worker = Some(DecodeWorkerHandle::spawn(
            source,
            self.audio_output.config(),
            self.audio_output.clone(),
            initial_count,
        ));
        self.decode_completion_pending = None;
        self.playback.buffered_ms = self.audio_output.buffered_ms();
        Ok(())
    }

    fn cancel_decode_worker(&mut self) {
        if let Some(worker) = self.decode_worker.as_ref() {
            worker.cancel();
        }
        self.decode_worker = None;
        self.decode_completion_pending = None;
    }

    fn advance_after_track_end(&mut self) -> Vec<AppEvent> {
        self.queue.advance();
        self.sync_playback_from_queue()
    }

    fn sync_audio_transport(&self) {
        let active = matches!(self.playback.state, PlaybackState::Playing | PlaybackState::Buffering);
        self.audio_output.set_playback_active(active);
    }

    fn complete_scan(&mut self, outcome: ScanOutcome) {
        self.scan.progress = outcome.progress.clone();
        self.scan.finish_with(outcome.result);
        self.library = build_library_from_track_inputs(&mut self.strings, outcome.tracks);
        self.ensure_browse_cache_current();
    }

    fn ensure_browse_cache_current(&mut self) {
        if self.browse_cache.library_revision == self.library.revision {
            return;
        }

        self.browse_cache.albums.reset(
            self.library
                .albums
                .titles
                .iter()
                .enumerate()
                .map(|(index, title_id)| AlbumLogEntry {
                    album_id: AlbumId(index as u32),
                    title: self
                        .strings
                        .resolve(*title_id)
                        .unwrap_or_else(|| "Unknown".to_string()),
                    album_artist: self
                        .library
                        .albums
                        .album_artist_ids
                        .get(index)
                        .and_then(|artist_id| {
                            artist_id.and_then(|id| self.library.artist_name(id, &self.strings))
                        })
                        .unwrap_or_else(|| "Unknown".to_string()),
                    track_count: self
                        .library
                        .indexes
                        .tracks_by_album
                        .get(index)
                        .map_or(0, Vec::len),
                })
                .collect(),
        );
        self.browse_cache.track_artists.reset(self.artist_log_entries_for(ArtistKind::TrackArtist));
        self.browse_cache.album_artists.reset(self.artist_log_entries_for(ArtistKind::AlbumArtist));
        self.browse_cache.composers.reset(self.artist_log_entries_for(ArtistKind::Composer));
        self.browse_cache.library_revision = self.library.revision;
    }

    fn hydrate_browse_lists(&mut self) {
        self.browse_cache.albums.hydrate_next_batch();
        self.browse_cache.track_artists.hydrate_next_batch();
        self.browse_cache.album_artists.hydrate_next_batch();
        self.browse_cache.composers.hydrate_next_batch();
    }

    fn artist_log_entries_for(&self, kind: ArtistKind) -> Vec<ArtistLogEntry> {
        let source: &Vec<Vec<TrackId>> = match kind {
            ArtistKind::TrackArtist => &self.library.indexes.tracks_by_artist,
            ArtistKind::AlbumArtist => &self.library.indexes.tracks_by_album_artist,
            ArtistKind::Composer => &self.library.indexes.tracks_by_composer,
        };

        self.library
            .artists
            .names
            .iter()
            .enumerate()
            .filter_map(|(index, name_id)| {
                let track_count = source.get(index).map_or(0, Vec::len);
                if track_count == 0 {
                    return None;
                }
                Some(ArtistLogEntry {
                    artist_id: ArtistId(index as u32),
                    name: self
                        .strings
                        .resolve(*name_id)
                        .unwrap_or_else(|| "Unknown".to_string()),
                    track_count,
                })
            })
            .collect()
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

    fn album_tab_vm(&self) -> AlbumTabVm {
        let visible_tracks = self
            .selected_album
            .map(|album_id| self.track_rows_for_tracks(self.library.visible_tracks_for_album(album_id)))
            .unwrap_or_default();

        AlbumTabVm {
            albums: self.browse_cache.albums.visible_entries(),
            expanded_album: self.selected_album,
            visible_tracks,
            loading: self.browse_cache.albums.loading_state(),
        }
    }

    fn artist_tab_vm(&self) -> ArtistTabVm {
        let visible_tracks = self
            .selected_artist
            .map(|artist_id| {
                self.track_rows_for_tracks(
                    self.library
                        .visible_tracks_for_artist(self.selected_artist_kind, artist_id),
                )
            })
            .unwrap_or_default();

        let (artists, loading) = match self.selected_artist_kind {
            ArtistKind::TrackArtist => (
                self.browse_cache.track_artists.visible_entries(),
                self.browse_cache.track_artists.loading_state(),
            ),
            ArtistKind::AlbumArtist => (
                self.browse_cache.album_artists.visible_entries(),
                self.browse_cache.album_artists.loading_state(),
            ),
            ArtistKind::Composer => (
                self.browse_cache.composers.visible_entries(),
                self.browse_cache.composers.loading_state(),
            ),
        };

        ArtistTabVm {
            kind: self.selected_artist_kind,
            artists,
            selected_artist: self.selected_artist,
            visible_tracks,
            loading,
        }
    }

    fn playlist_tab_vm(&self) -> PlaylistTabVm {
        let playlists = self
            .playlists
            .playlists
            .iter()
            .map(|playlist| PlaylistEntryVm {
                playlist_id: playlist.id,
                name: self
                    .strings
                    .resolve(playlist.name)
                    .unwrap_or_else(|| format!("playlist-{}", playlist.id.0)),
                track_count: playlist.tracks.len(),
                is_system: playlist.kind == PlaylistKind::SystemFavorite,
            })
            .collect();

        let visible_tracks = self
            .playlists
            .selected
            .and_then(|selected| {
                self.playlists
                    .playlists
                    .iter()
                    .find(|playlist| playlist.id == selected)
                    .map(|playlist| self.track_rows_for_tracks(&playlist.tracks))
            })
            .unwrap_or_default();

        PlaylistTabVm {
            playlists,
            selected_playlist: self.playlists.selected,
            visible_tracks,
        }
    }

    fn library_tab_vm(&self) -> LibraryTabVm {
        let nodes = self
            .library
            .directories
            .nodes
            .get(self.library.directories.root.0 as usize)
            .map(|root| {
                root.children
                    .iter()
                    .filter_map(|node_id| {
                        self.library.directories.nodes.get(node_id.0 as usize).map(|node| {
                            DirectoryEntryVm {
                                node_id: *node_id,
                                name: self
                                    .strings
                                    .resolve(node.name)
                                    .unwrap_or_else(|| "Unknown".to_string()),
                                child_count: node.children.len(),
                                track_count: node.tracks.len(),
                            }
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let visible_tracks = self
            .selected_directory
            .and_then(|node_id| self.library.directories.nodes.get(node_id.0 as usize))
            .map(|node| self.track_rows_for_tracks(&node.tracks))
            .unwrap_or_default();

        LibraryTabVm {
            nodes,
            selected_directory: self.selected_directory,
            visible_tracks,
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
            cancel_visible: matches!(self.scan.state, ScanState::Scanning | ScanState::Cancelling),
        }
    }

    fn settings_tab_vm(&self) -> SettingsTabVm {
        SettingsTabVm {
            items: vec![
                SettingItemVm {
                    key: "accent_color".to_string(),
                    value: self.settings.accent_color.clone(),
                },
                SettingItemVm {
                    key: "theme".to_string(),
                    value: self.settings.theme.clone(),
                },
                SettingItemVm {
                    key: "seek_step_ms".to_string(),
                    value: self.settings.seek_step_ms.to_string(),
                },
                SettingItemVm {
                    key: "restore_position_policy".to_string(),
                    value: self.settings.restore_position_policy.clone(),
                },
                SettingItemVm {
                    key: "full_auto_scan".to_string(),
                    value: self.settings.full_auto_scan.to_string(),
                },
                SettingItemVm {
                    key: "fallback_artwork_path".to_string(),
                    value: self
                        .settings
                        .fallback_artwork_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "none".to_string()),
                },
                SettingItemVm {
                    key: "play_empty_state".to_string(),
                    value: self.settings.play_empty_state.clone(),
                },
            ],
            available_output_devices: AudioDeviceOutput::list_output_devices(),
            preferred_output_device: self.settings.preferred_output_device.clone(),
        }
    }

    fn track_rows_for_tracks(&self, tracks: &[TrackId]) -> Vec<TrackRowVm> {
        tracks
            .iter()
            .copied()
            .map(|track_id| self.track_row_vm(track_id, None))
            .collect()
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use hound::{SampleFormat, WavSpec, WavWriter};
    use tempfile::TempDir;

    use super::AppFacade;
    use crate::app::commands::{AppCommand, TrackListOrigin};
    use crate::app::events::AppEvent;
    use crate::core::ids::TrackId;
    use crate::core::library::TrackInput;
    use crate::core::persistence::{JsonStore, PersistencePaths, TabId};
    use crate::core::playback::PlaybackState;
    use crate::core::scan::{ScanCommand, ScanState};

    fn temp_paths() -> PersistencePaths {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("lrc-player-facade-{unique}"));
        PersistencePaths {
            config_json: root.join("config.json"),
            state_json: root.join("state.json"),
            playlists_json: root.join("playlists.json"),
        }
    }

    fn poll_scan_until_finished(facade: &mut AppFacade) {
        for _ in 0..10_000 {
            facade.poll_background_tasks();
            if !matches!(facade.scan.state, ScanState::Scanning | ScanState::Cancelling) {
                return;
            }
            thread::yield_now();
        }

        panic!("scan worker did not finish in time");
    }

    fn write_wav(path: &std::path::Path, frames: u32) {
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec).expect("wav file should be created");
        for index in 0..frames {
            let phase = (index % 256) as i16;
            let sample = (phase * 128) - 16_384;
            writer.write_sample(sample).expect("left sample should be written");
            writer.write_sample(sample).expect("right sample should be written");
        }
        writer.finalize().expect("wav file should finalize");
    }

    #[test]
    fn save_and_restore_round_trip() {
        let paths = temp_paths();
        let store = JsonStore::new(paths.clone());

        let mut facade = AppFacade::default();
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "Track One".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: "/music/album/01-track-one.flac".to_string(),
                file_size: 1,
                modified_unix_secs: 10,
                file_name: "01-track-one.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "Track Two".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: "/music/album/02-track-two.flac".to_string(),
                file_size: 2,
                modified_unix_secs: 10,
                file_name: "02-track-two.flac".to_string(),
                track_number: None,
                duration_ms: 2_000,
                artwork_key: None,
            },
        );

        facade.queue.replace_with_tracks(&[TrackId(0), TrackId(1)]);
        facade.queue.set_current_index(1);
        facade.playback.restore_track(TrackId(1), 3_210);
        facade.current_tab = TabId::Play;
        let _ = facade.handle_command(AppCommand::Scan(ScanCommand::AddIncludeDir("/music".into())));
        let _ = facade.handle_command(AppCommand::CreatePlaylist {
            name: "Roadtrip".to_string(),
        });
        if let Some(playlist_id) = facade.playlists.selected {
            let _ = facade.handle_command(AppCommand::AddToPlaylist {
                playlist_id,
                track_id: TrackId(1),
            });
        }
        let _ = facade.handle_command(AppCommand::ToggleFavorite { track_id: TrackId(0) });

        facade.save_to_store(&store).expect("save should succeed");

        let mut restored = AppFacade::default();
        restored.library = facade.library.clone();
        restored.strings = facade.strings.clone();
        restored
            .restore_from_store(&store)
            .expect("restore should succeed");

        assert_eq!(restored.current_tab, TabId::Play);
        assert_eq!(restored.queue.current_track(), Some(TrackId(1)));
        assert_eq!(restored.playback.position_ms, 3_210);
        assert_eq!(restored.scan.config.include_dirs, vec![std::path::PathBuf::from("/music")]);
        assert!(restored.playlists.contains_in_favorite(TrackId(0)));

        let root = paths
            .config_json
            .parent()
            .expect("config parent should exist")
            .to_path_buf();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bootstrap_from_store_rescans_and_restores_queue() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let include_root = temp_dir.path().join("library_root");
        let music_dir = include_root.join("Artist").join("Album");
        fs::create_dir_all(&music_dir).expect("music dir should be created");
        fs::write(music_dir.join("01-track-one.flac"), b"fLaCdemo").expect("track one should be created");
        fs::write(music_dir.join("02-track-two.flac"), b"fLaCdemo").expect("track two should be created");

        let paths = temp_paths();
        let store = JsonStore::new(paths.clone());

        let mut facade = AppFacade::default();
        let _ = facade.handle_command(AppCommand::Scan(ScanCommand::AddIncludeDir(
            include_root,
        )));
        let _ = facade.handle_command(AppCommand::Scan(ScanCommand::StartFullScan));
        poll_scan_until_finished(&mut facade);
        facade.queue.replace_with_tracks(&[TrackId(0), TrackId(1)]);
        facade.queue.set_current_index(1);
        facade.playback.restore_track(TrackId(1), 999);
        facade
            .save_to_store(&store)
            .expect("save should succeed");

        let mut restored = AppFacade::default();
        restored
            .bootstrap_from_store(&store)
            .expect("bootstrap should succeed");

        assert_eq!(restored.library.tracks.titles.len(), 2);
        assert_eq!(restored.queue.current_track(), Some(TrackId(1)));
        assert_eq!(restored.playback.position_ms, 999);

        let root = paths
            .config_json
            .parent()
            .expect("config parent should exist")
            .to_path_buf();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn view_model_exposes_album_artist_playlist_library_and_settings_tabs() {
        let mut facade = AppFacade::default();
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "01-track-one".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: "/music/Artist/Album/01-track-one.flac".to_string(),
                file_size: 1,
                modified_unix_secs: 10,
                file_name: "01-track-one.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );
        let playlist_id = facade
            .playlists
            .create_user_playlist(facade.strings.intern("Roadtrip"))
            .expect("playlist should be created");
        facade.playlists.add_track(playlist_id, TrackId(0));
        facade.playlists.selected = Some(playlist_id);
        facade.selected_album = Some(crate::core::ids::AlbumId(0));
        facade.selected_artist = Some(crate::core::ids::ArtistId(0));
        facade.selected_directory = facade.library.directories.nodes[0].children.first().copied();

        let view_model = facade.view_model();

        assert_eq!(view_model.album.albums.len(), 1);
        assert_eq!(view_model.album.visible_tracks.len(), 1);
        assert_eq!(view_model.artist.artists.len(), 1);
        assert_eq!(view_model.artist.visible_tracks.len(), 1);
        assert_eq!(view_model.playlist.playlists.len(), 2);
        assert_eq!(view_model.playlist.visible_tracks.len(), 1);
        assert_eq!(view_model.library.nodes.len(), 1);
        assert!(!view_model.settings.items.is_empty());
    }

    #[test]
    fn select_album_command_updates_visible_tracks() {
        let mut facade = AppFacade::default();
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "track-a".to_string(),
                album: Some("Album A".to_string()),
                artist: Some("Artist A".to_string()),
                album_artist: Some("Artist A".to_string()),
                composer: None,
                normalized_path: "/music/a/01.flac".to_string(),
                file_size: 1,
                modified_unix_secs: 10,
                file_name: "01.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "track-b".to_string(),
                album: Some("Album B".to_string()),
                artist: Some("Artist B".to_string()),
                album_artist: Some("Artist B".to_string()),
                composer: None,
                normalized_path: "/music/b/01.flac".to_string(),
                file_size: 2,
                modified_unix_secs: 10,
                file_name: "01.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );

        let _ = facade.view_model();
        let _ = facade.handle_command(AppCommand::SelectAlbum(Some(crate::core::ids::AlbumId(1))));
        let view_model = facade.view_model();

        assert_eq!(view_model.album.expanded_album, Some(crate::core::ids::AlbumId(1)));
        assert_eq!(view_model.album.visible_tracks.len(), 1);
        assert_eq!(view_model.album.visible_tracks[0].title, "track-b");
    }

    #[test]
    fn select_artist_command_updates_visible_tracks() {
        let mut facade = AppFacade::default();
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "track-a".to_string(),
                album: Some("Album A".to_string()),
                artist: Some("Artist A".to_string()),
                album_artist: Some("Artist A".to_string()),
                composer: None,
                normalized_path: "/music/a/01.flac".to_string(),
                file_size: 1,
                modified_unix_secs: 10,
                file_name: "01.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "track-b".to_string(),
                album: Some("Album B".to_string()),
                artist: Some("Artist B".to_string()),
                album_artist: Some("Artist B".to_string()),
                composer: None,
                normalized_path: "/music/b/01.flac".to_string(),
                file_size: 2,
                modified_unix_secs: 10,
                file_name: "01.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );

        let _ = facade.view_model();
        let _ = facade.handle_command(AppCommand::SelectArtist(Some(crate::core::ids::ArtistId(1))));
        let view_model = facade.view_model();

        assert_eq!(view_model.artist.selected_artist, Some(crate::core::ids::ArtistId(1)));
        assert_eq!(view_model.artist.visible_tracks.len(), 1);
        assert_eq!(view_model.artist.visible_tracks[0].title, "track-b");
    }

    #[test]
    fn album_and_artist_lists_start_from_log_snapshot_then_hydrate_in_background() {
        let mut facade = AppFacade::default();
        for index in 0..80u32 {
            facade.library.upsert_track(
                &mut facade.strings,
                TrackInput {
                    title: format!("Track {index}"),
                    album: Some(format!("Album {index}")),
                    artist: Some(format!("Artist {index}")),
                    album_artist: Some(format!("Artist {index}")),
                    composer: None,
                    normalized_path: format!("/music/Artist{index}/Album{index}/{index:02}.flac"),
                    file_size: u64::from(index) + 1,
                    modified_unix_secs: 10,
                    file_name: format!("{index:02}.flac"),
                    track_number: None,
                    duration_ms: 1_000,
                    artwork_key: None,
                },
            );
        }

        let initial = facade.view_model();

        assert_eq!(initial.album.loading.hydrated_entries, 0);
        assert!(initial.album.loading.is_loading);
        assert!(initial.album.albums.iter().all(|entry| entry.is_provisional));
        assert_eq!(initial.artist.loading.hydrated_entries, 0);
        assert!(initial.artist.loading.is_loading);
        assert!(initial.artist.artists.iter().all(|entry| entry.is_provisional));

        facade.poll_background_tasks();
        let hydrated = facade.view_model();

        assert!(hydrated.album.loading.hydrated_entries > 0);
        assert!(hydrated.album.loading.hydrated_entries < hydrated.album.loading.total_entries);
        assert!(hydrated
            .album
            .albums
            .iter()
            .take(hydrated.album.loading.hydrated_entries)
            .all(|entry| !entry.is_provisional));
        assert!(hydrated.artist.loading.hydrated_entries > 0);
        assert!(hydrated.artist.loading.hydrated_entries < hydrated.artist.loading.total_entries);
        assert!(hydrated
            .artist
            .artists
            .iter()
            .take(hydrated.artist.loading.hydrated_entries)
            .all(|entry| !entry.is_provisional));
    }

    #[test]
    fn playback_prefills_audio_buffer_for_existing_file() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let file_path = temp_dir.path().join("sample.wav");
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&file_path, spec).expect("wav file should be created");
        for index in 0..16i16 {
            let sample = (index * 256) - 2048;
            writer.write_sample(sample).expect("left sample should be written");
            writer.write_sample(sample).expect("right sample should be written");
        }
        writer.finalize().expect("wav file should finalize");

        let mut facade = AppFacade::default();
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "sample".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: file_path.to_string_lossy().to_string(),
                file_size: fs::metadata(&file_path)
                    .expect("metadata should exist")
                    .len(),
                modified_unix_secs: 10,
                file_name: "sample.wav".to_string(),
                track_number: None,
                duration_ms: 0,
                artwork_key: None,
            },
        );
        facade.queue.replace_with_tracks(&[TrackId(0)]);

        let events = facade.handle_command(AppCommand::NextTrack);

        assert!(!events.is_empty());
        assert!(facade.audio_output.buffered_blocks() > 0);
        assert!(facade.playback.buffered_ms > 0);
    }

    #[test]
    fn start_scan_runs_in_background_until_polled() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let include_root = temp_dir.path().join("library_root");
        let music_dir = include_root.join("Artist").join("Album");
        fs::create_dir_all(&music_dir).expect("music dir should be created");
        fs::write(music_dir.join("01-track-one.flac"), b"fLaCdemo").expect("track should exist");

        let mut facade = AppFacade::default();
        let _ = facade.handle_command(AppCommand::Scan(ScanCommand::AddIncludeDir(include_root)));

        let events = facade.handle_command(AppCommand::Scan(ScanCommand::StartFullScan));

        assert_eq!(events, vec![AppEvent::ScanStateChanged(ScanState::Scanning)]);
        assert_eq!(facade.library.tracks.titles.len(), 0);

        poll_scan_until_finished(&mut facade);

        assert_eq!(facade.library.tracks.titles.len(), 1);
        assert_eq!(facade.scan.visible_status(), ScanState::Completed);
    }

    #[test]
    fn exhausted_track_advances_to_next_track_after_buffer_drains() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let first = temp_dir.path().join("first.wav");
        let second = temp_dir.path().join("second.wav");
        write_wav(&first, 2_048);
        write_wav(&second, 2_048);

        let mut facade = AppFacade::default();
        for (index, path, name) in [(0, &first, "first.wav"), (1, &second, "second.wav")] {
            facade.library.upsert_track(
                &mut facade.strings,
                TrackInput {
                    title: format!("track-{index}"),
                    album: Some("Album".to_string()),
                    artist: Some("Artist".to_string()),
                    album_artist: Some("Artist".to_string()),
                    composer: None,
                    normalized_path: path.to_string_lossy().to_string(),
                    file_size: fs::metadata(path).expect("metadata should exist").len(),
                    modified_unix_secs: 10,
                    file_name: name.to_string(),
                    track_number: None,
                    duration_ms: 0,
                    artwork_key: None,
                },
            );
        }
        facade.queue.replace_with_tracks(&[TrackId(0), TrackId(1)]);

        let _ = facade.handle_command(AppCommand::ClickTrack {
            visible_tracks: vec![TrackId(0), TrackId(1)],
            clicked_index: 0,
            origin: TrackListOrigin::Queue,
        });

        for _ in 0..10_000 {
            while facade.audio_output.buffered_blocks() > 0 {
                let _ = facade.audio_output.pop_or_silence();
            }
            facade.poll_background_tasks();
            if facade.queue.current_track() == Some(TrackId(1)) {
                break;
            }
            thread::yield_now();
        }

        assert_eq!(facade.queue.current_track(), Some(TrackId(1)));
        assert_eq!(facade.playback.current_track, Some(TrackId(1)));
        assert_eq!(facade.playback.state, PlaybackState::Playing);
    }

    #[test]
    fn undecodable_track_skips_to_next_track() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let broken = temp_dir.path().join("broken.mp3");
        let valid = temp_dir.path().join("valid.wav");
        fs::write(&broken, b"not-audio").expect("broken file should be written");

        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&valid, spec).expect("wav file should be created");
        for _ in 0..16 {
            writer.write_sample(0i16).expect("left sample should be written");
            writer.write_sample(0i16).expect("right sample should be written");
        }
        writer.finalize().expect("wav file should finalize");

        let mut facade = AppFacade::default();
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "broken".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: broken.to_string_lossy().to_string(),
                file_size: 9,
                modified_unix_secs: 10,
                file_name: "broken.mp3".to_string(),
                track_number: None,
                duration_ms: 0,
                artwork_key: None,
            },
        );
        facade.library.upsert_track(
            &mut facade.strings,
            TrackInput {
                title: "valid".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: valid.to_string_lossy().to_string(),
                file_size: 64,
                modified_unix_secs: 10,
                file_name: "valid.wav".to_string(),
                track_number: None,
                duration_ms: 0,
                artwork_key: None,
            },
        );
        facade.queue.replace_with_tracks(&[TrackId(0), TrackId(1)]);
        facade.queue.set_current_index(0);

        let events = facade.handle_command(AppCommand::ClickTrack {
            visible_tracks: vec![TrackId(0), TrackId(1)],
            clicked_index: 0,
            origin: TrackListOrigin::Queue,
        });

        assert!(!events.is_empty());
        assert_eq!(facade.queue.current_track(), Some(TrackId(1)));
        assert!(facade.audio_output.buffered_blocks() > 0);
    }
}