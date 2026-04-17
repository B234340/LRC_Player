use crate::application::dto::{LibraryDto, PlaylistDto, SettingsDto, StateDto};
use crate::application::usecases::build_app_vm::build_app_vm;
use crate::application::view_model::{AppVm, QueueVm};
use crate::config::{Config, Language, LogLevel, Theme};
use crate::infrastructure::audio;
use crate::infrastructure::decode::SymphoniaDecoder;
use crate::ports::audio_output::{AudioFormat, AudioOutput, SampleFormat};
use crossbeam_channel::Receiver;
use crate::infrastructure::lyrics::Lyrics;
use crate::infrastructure::persistence::{MetaCache, PlaylistsFile, StateFile};
use image::io::Reader as ImageReader;
use std::path::{Path, PathBuf};
use crate::localization::{language_name, supported_languages, translate};
use crate::presentation::tabs;
use crate::shared::ipc as ipc_mod;
use cpal::traits::{DeviceTrait, HostTrait};
use eframe::{egui, App, CreationContext};
use fontdb::{Database, Family, Source, Style};
use std::collections::HashSet;
use std::fs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Library,
    Play,
    Playlist,
    Album,
    Artist,
    Queue,
    Scan,
    Settings,
}

impl Tab {
    fn label(&self) -> &'static str {
        match self {
            Tab::Library => "Library",
            Tab::Play => "Play",
            Tab::Playlist => "Playlists",
            Tab::Album => "Albums",
            Tab::Artist => "Artists",
            Tab::Queue => "Queue",
            Tab::Scan => "Scan",
            Tab::Settings => "Settings",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "Library" => Some(Tab::Library),
            "Play" => Some(Tab::Play),
            "Playlists" => Some(Tab::Playlist),
            "Albums" => Some(Tab::Album),
            "Artists" => Some(Tab::Artist),
            "Queue" => Some(Tab::Queue),
            "Scan" => Some(Tab::Scan),
            "Settings" => Some(Tab::Settings),
            _ => None,
        }
    }
}

pub struct GuiApp {
    pub app_vm: AppVm,
    active_tab: Tab,
    device_names: Vec<String>,
    selected: Option<usize>,
    device_filter: String,
    pub(crate) play_path: String,
    pub(crate) lyrics_files: Vec<std::path::PathBuf>,
    pub(crate) selected_lyrics_index: Option<usize>,
    pub(crate) loaded_lyrics: Option<Lyrics>,
    playback_status_receiver: Option<Receiver<String>>,
    pub(crate) play_position_ms: u64,
    pub(crate) play_duration_ms: Option<u64>,
    pub(crate) play_title: Option<String>,
    pub(crate) play_artist: Option<String>,
    pub(crate) play_album: Option<String>,
    pub(crate) play_artwork_handle: Option<egui::TextureHandle>,
    pub(crate) play_artwork_source: Option<PathBuf>,
    pub(crate) lyrics_offset_ms: i64,
    pub(crate) play_status: Option<String>,
    scan_target_input: String,
    scan_exclude_input: String,
    scan_targets: Vec<String>,
    pub(crate) scan_excludes: Vec<String>,
    selected_album: Option<String>,
    selected_artist: Option<String>,
    selected_directory: Option<String>,
    favorite_directories: Vec<String>,
    favorite_directory_input: String,
    pub(crate) playlists: Vec<PlaylistDto>,
    pub(crate) new_playlist_name: String,
    pub(crate) selected_playlist: Option<usize>,
    pub(crate) playlist_track_input: String,
    queue: QueueVm,
    queue_item_input: String,
    pub(crate) selected_queue_index: Option<usize>,
    pub(crate) taps: usize,
    pub(crate) theme: Theme,
    pub(crate) log_level: LogLevel,
    pub(crate) language: Language,
    last_scan_status: Option<String>,
}

impl GuiApp {
    fn default_app_vm(
        active_tab: Tab,
        config: &Config,
        last_scan_status: Option<String>,
        scan_targets: Vec<String>,
        scan_excludes: Vec<String>,
        library: LibraryDto,
        playlists: &[PlaylistDto],
        queue: QueueVm,
    ) -> AppVm {
        let settings = SettingsDto::new(
            config.preferred_output_device.clone(),
            config.taps_for_device(config.preferred_output_device.as_ref()),
        );
        let state = StateDto {
            last_tab: Some(active_tab.label().to_string()),
            last_play_path: None,
            last_play_position_ms: None,
            last_scan_status,
            scan_scanned: None,
            scan_parsed: None,
            scan_registered: None,
            scan_targets,
            scan_excludes,
            queue_tracks: Vec::new(),
            selected_queue_index: None,
            favorite_directories: Vec::new(),
        };

        build_app_vm("LRC Player", &settings, &state, playlists, &library, queue)
    }

    fn load_state() -> StateDto {
        StateFile::open()
            .and_then(|store| store.load())
            .unwrap_or(StateDto {
                last_tab: None,
                last_play_path: None,
                last_scan_status: None,
                scan_scanned: None,
                scan_parsed: None,
                scan_registered: None,
                scan_targets: Vec::new(),
                scan_excludes: Vec::new(),
                queue_tracks: Vec::new(),
                selected_queue_index: None,
                last_play_position_ms: None,
                favorite_directories: Vec::new(),
            })
    }

    pub(crate) fn save_state(&self) {
        let state = StateDto {
            last_tab: Some(self.active_tab.label().to_string()),
            last_play_path: if self.play_path.is_empty() {
                None
            } else {
                Some(self.play_path.clone())
            },
            last_play_position_ms: Some(self.play_position_ms),
            last_scan_status: self.last_scan_status.clone(),
            scan_scanned: None,
            scan_parsed: None,
            scan_registered: None,
            scan_targets: self.scan_targets.clone(),
            scan_excludes: self.scan_excludes.clone(),
            queue_tracks: self.queue.tracks.clone(),
            selected_queue_index: self.selected_queue_index,
            favorite_directories: self.favorite_directories.clone(),
        };
        if let Ok(store) = StateFile::open() {
            let _ = store.save(&state);
        }
    }

    fn refresh_scan_status(&mut self) {
        if let Ok(store) = StateFile::open() {
            if let Ok(state) = store.load() {
                self.last_scan_status = state.last_scan_status.clone();
                self.app_vm.scan.last_scan_status = state.last_scan_status.clone();
            }
        }
    }

    fn load_playlists() -> Vec<PlaylistDto> {
        if let Ok(file) = PlaylistsFile::open() {
            file.load().unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    fn save_playlists(&self) {
        if let Ok(file) = PlaylistsFile::open() {
            let _ = file.save(&self.playlists);
        }
    }

    pub(crate) fn add_playlist(&mut self) {
        let name = self.new_playlist_name.trim().to_string();
        if name.is_empty() {
            return;
        }

        let id = format!(
            "playlist-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );

        self.playlists.push(PlaylistDto { id, name, tracks: Vec::new() });
        self.new_playlist_name.clear();
        self.save_playlists();
        self.update_app_vm();
    }

    pub(crate) fn remove_playlist(&mut self, index: usize) {
        if index < self.playlists.len() {
            self.playlists.remove(index);
            if self.selected_playlist == Some(index) {
                self.selected_playlist = None;
            }
            self.save_playlists();
            self.update_app_vm();
        }
    }

    fn add_queue_item(&mut self) {
        let item = self.queue_item_input.trim().to_string();
        if item.is_empty() {
            return;
        }
        self.queue.tracks.push(item);
        self.queue_item_input.clear();
        self.update_app_vm();
    }

    pub(crate) fn remove_queue_item(&mut self, index: usize) {
        if index < self.queue.tracks.len() {
            self.queue.tracks.remove(index);
            if self.selected_queue_index == Some(index) {
                self.selected_queue_index = None;
            }
            self.update_app_vm();
        }
    }

    pub(crate) fn move_queue_item_up(&mut self, index: usize) {
        if index == 0 || index >= self.queue.tracks.len() {
            return;
        }
        self.queue.tracks.swap(index, index - 1);
        self.selected_queue_index = Some(index - 1);
        self.update_app_vm();
    }

    pub(crate) fn move_queue_item_down(&mut self, index: usize) {
        if index + 1 >= self.queue.tracks.len() {
            return;
        }
        self.queue.tracks.swap(index, index + 1);
        self.selected_queue_index = Some(index + 1);
        self.update_app_vm();
    }

    pub(crate) fn add_track_to_selected_playlist(&mut self) {
        let track = self.playlist_track_input.trim().to_string();
        if track.is_empty() {
            return;
        }
        if let Some(index) = self.selected_playlist {
            if let Some(playlist) = self.playlists.get_mut(index) {
                if !playlist.tracks.contains(&track) {
                    playlist.tracks.push(track.clone());
                    self.playlist_track_input.clear();
                    self.save_playlists();
                    self.update_app_vm();
                }
            }
        }
    }

    pub(crate) fn remove_track_from_selected_playlist(&mut self, track_index: usize) {
        if let Some(index) = self.selected_playlist {
            if let Some(playlist) = self.playlists.get_mut(index) {
                if track_index < playlist.tracks.len() {
                    playlist.tracks.remove(track_index);
                    self.save_playlists();
                    self.update_app_vm();
                }
            }
        }
    }

    pub(crate) fn select_album(&mut self, album: Option<String>) {
        self.selected_album = album;
        self.selected_artist = None;
        self.selected_directory = None;
        self.update_app_vm();
    }

    pub(crate) fn select_artist(&mut self, artist: Option<String>) {
        self.selected_artist = artist;
        self.selected_album = None;
        self.selected_directory = None;
        self.update_app_vm();
    }

    pub(crate) fn select_directory(&mut self, directory: Option<String>) {
        self.selected_directory = directory;
        self.selected_album = None;
        self.selected_artist = None;
        self.update_app_vm();
    }

    fn add_favorite_directory(&mut self) {
        let dir = self.favorite_directory_input.trim().to_string();
        if dir.is_empty() || self.favorite_directories.contains(&dir) {
            return;
        }

        self.favorite_directories.push(dir);
        self.favorite_directory_input.clear();
        self.save_state();
        self.update_app_vm();
    }

    fn remove_favorite_directory(&mut self, index: usize) {
        if index < self.favorite_directories.len() {
            self.favorite_directories.remove(index);
            self.save_state();
            self.update_app_vm();
        }
    }

    pub(crate) fn play_selected_queue_item(&mut self) {
        if let Some(index) = self.selected_queue_index {
            if let Some(path) = self.queue.tracks.get(index) {
                self.play_path = path.clone();
                self.discover_lyrics_for_current_play_path();
                self.refresh_play_metadata();
                self.request_play();
            }
        }
    }

    pub(crate) fn refresh_play_metadata(&mut self) {
        let path = PathBuf::from(self.play_path.clone());
        if path.as_os_str().is_empty() || !path.exists() {
            self.play_duration_ms = None;
            self.play_title = None;
            self.play_artist = None;
            self.play_album = None;
            self.play_artwork_handle = None;
            self.play_artwork_source = None;
            return;
        }

        let decoder = SymphoniaDecoder::new();
        if let Ok(decoded) = decoder.decode_file(&path) {
            let frames = decoded.frames() as u64;
            let sample_rate = decoded.sample_rate as u64;
            if sample_rate > 0 {
                self.play_duration_ms = Some(frames.saturating_mul(1000) / sample_rate);
                if let Some(duration_ms) = self.play_duration_ms {
                    self.play_position_ms = self.play_position_ms.min(duration_ms);
                }
            }
            self.play_title = decoded.title;
            self.play_artist = decoded.artist;
            self.play_album = decoded.album;
        }
    }

    pub(crate) fn ensure_play_artwork(&mut self, ctx: &egui::Context) {
        let path = Path::new(&self.play_path);
        if path.as_os_str().is_empty() || !path.exists() {
            self.play_artwork_handle = None;
            self.play_artwork_source = None;
            return;
        }

        let artwork_path = self.discover_artwork_for_track(path);
        if artwork_path.is_none() {
            self.play_artwork_handle = None;
            self.play_artwork_source = None;
            return;
        }

        let artwork_path = artwork_path.unwrap();
        if self.play_artwork_source.as_ref() == Some(&artwork_path) && self.play_artwork_handle.is_some() {
            return;
        }

        let image_result: Result<image::DynamicImage, image::ImageError> = (|| {
            let reader = ImageReader::open(&artwork_path).map_err(image::ImageError::IoError)?;
            let reader = reader.with_guessed_format().map_err(image::ImageError::IoError)?;
            reader.decode()
        })();

        match image_result {
            Ok(image) => {
                let size = [image.width() as usize, image.height() as usize];
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    size,
                    &image.to_rgba8().into_vec(),
                );
                let texture = ctx.load_texture("play_artwork", color_image, egui::TextureOptions::default());
                self.play_artwork_handle = Some(texture);
                self.play_artwork_source = Some(artwork_path);
            }
            Err(_) => {
                self.play_artwork_handle = None;
                self.play_artwork_source = None;
            }
        }
    }

    fn discover_artwork_for_track(&self, path: &Path) -> Option<PathBuf> {
        let parent = path.parent()?;
        let stem = path.file_stem()?.to_string_lossy();
        let candidates = [
            format!("{}.jpg", stem),
            format!("{}.jpeg", stem),
            format!("{}.png", stem),
            format!("{}.webp", stem),
            format!("folder.jpg"),
            format!("cover.jpg"),
            format!("front.jpg"),
            format!("folder.png"),
            format!("cover.png"),
            format!("front.png"),
        ];

        for candidate in candidates.iter() {
            let candidate_path = parent.join(candidate);
            if candidate_path.exists() {
                return Some(candidate_path);
            }
        }
        None
    }

    pub(crate) fn discover_lyrics_for_current_play_path(&mut self) {
        if self.play_path.is_empty() {
            self.lyrics_files.clear();
            self.selected_lyrics_index = None;
            self.loaded_lyrics = None;
            return;
        }

        self.lyrics_files = Lyrics::discover_for_track(&self.play_path);
        self.selected_lyrics_index = None;
        self.loaded_lyrics = None;
    }

    pub(crate) fn load_lyrics_file(&mut self, index: usize) {
        if let Some(path) = self.lyrics_files.get(index) {
            match Lyrics::from_lrc(path) {
                Ok(lyrics) => {
                    self.loaded_lyrics = Some(lyrics);
                    self.selected_lyrics_index = Some(index);
                }
                Err(err) => {
                    eprintln!("Unable to load lyrics file {}: {}", path.display(), err);
                }
            }
        }
    }

    fn library_dto(&self) -> LibraryDto {
        let mut library = LibraryDto {
            album_count: 0,
            track_count: 0,
            recent_locations: Vec::new(),
            recent_directories: Vec::new(),
            directory_tree: Vec::new(),
            favorite_directories: self.favorite_directories.clone(),
            albums: Vec::new(),
            selected_album: self.selected_album.clone(),
            selected_album_tracks: Vec::new(),
            artists: Vec::new(),
            selected_artist: self.selected_artist.clone(),
            selected_artist_tracks: Vec::new(),
            selected_directory: self.selected_directory.clone(),
            selected_directory_tracks: Vec::new(),
        };

        if let Ok(cache) = MetaCache::open_default() {
            if let Ok((album_count, track_count)) = cache.library_counts() {
                library.album_count = album_count;
                library.track_count = track_count;
            }
            if let Ok(recent_locations) = cache.recent_locations(20) {
                library.recent_locations = recent_locations;
            }
            if let Ok(recent_directories) = cache.recent_directories(20) {
                library.recent_directories = recent_directories;
            }
            if let Ok(directory_tree) = cache.library_directories(20) {
                library.directory_tree = directory_tree;
            }
            if let Ok(albums) = cache.library_albums() {
                library.albums = albums;
            }
            if let Ok(artists) = cache.library_artists() {
                library.artists = artists;
            }
            if let Some(album) = self.selected_album.as_deref() {
                if let Ok(tracks) = cache.library_tracks_by_album(album) {
                    library.selected_album_tracks = tracks;
                }
            }
            if let Some(artist) = self.selected_artist.as_deref() {
                if let Ok(tracks) = cache.library_tracks_by_artist(artist) {
                    library.selected_artist_tracks = tracks;
                }
            }
            if let Some(directory) = self.selected_directory.as_deref() {
                if let Ok(tracks) = cache.library_tracks_by_directory(directory) {
                    library.selected_directory_tracks = tracks;
                }
            }
        }

        library
    }

    pub(crate) fn update_app_vm(&mut self) {
        let cfg = Config::load();
        self.language = cfg.language;
        let library = self.library_dto();
        self.app_vm = Self::default_app_vm(
            self.active_tab,
            &cfg,
            self.last_scan_status.clone(),
            self.scan_targets.clone(),
            self.scan_excludes.clone(),
            library,
            &self.playlists,
            self.queue.clone(),
        );
        self.save_state();
    }

    fn request_scan(&mut self) {
        let paths = self.scan_targets.clone();
        if paths.is_empty() {
            self.last_scan_status = Some("No scan targets configured".to_string());
            self.save_state();
            self.update_app_vm();
            return;
        }

        self.last_scan_status = Some("Requested scan".to_string());
        self.save_state();
        self.update_app_vm();

        let socket = ipc_mod::default_socket_path();
        let request = ipc_mod::ScanRequest {
            paths,
            excludes: self.scan_excludes.clone(),
            cancel: false,
        };
        std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                let _ = rt.block_on(async move {
                    ipc_mod::send_scan_request(&socket, &request).await
                });
            }
        });
    }

    pub(crate) fn request_play(&mut self) {
        let path = std::path::PathBuf::from(self.play_path.clone());
        if path.as_os_str().is_empty() {
            self.play_status = Some("Play path is empty.".to_string());
            return;
        }
        if !path.exists() {
            self.play_status = Some(format!("Play path does not exist: {}", path.display()));
            return;
        }

        self.discover_lyrics_for_current_play_path();
        self.play_status = Some(format!("Starting playback: {}", path.display()));
        self.save_state();

        let (status_tx, status_rx) = crossbeam_channel::unbounded();
        self.playback_status_receiver = Some(status_rx);
        let preferred_device = self
            .selected
            .and_then(|idx| self.device_names.get(idx).cloned());
        let taps = self.taps;

        std::thread::spawn(move || {
            let mut output = audio::CpalOutput::new();

            let requested = AudioFormat {
                sample_rate: 44_100,
                channels: 2,
                sample_format: SampleFormat::F32,
            };

            let actual = match output.start_with_device(requested, preferred_device) {
                Ok(fmt) => fmt,
                Err(e) => {
                    let _ = status_tx.send(format!("Playback failed to start output: {}", e));
                    return;
                }
            };

            let producer_tx = match output.producer_sender() {
                Some(tx) => tx,
                None => {
                    let _ = status_tx.send("Playback failed: producer channel unavailable".to_string());
                    let _ = output.stop();
                    return;
                }
            };

            let pool = output.pool_handle();
            let chunk_frames = 2048;

            let _ = status_tx.send(format!(
                "Playback running: {} Hz / {} ch",
                actual.sample_rate, actual.channels
            ));

            let decode_handle = audio::decoder::spawn_decode_to_producer(
                path,
                chunk_frames,
                actual.sample_rate,
                actual.channels,
                producer_tx,
                pool,
                taps,
            );

            if decode_handle.join().is_err() {
                let _ = status_tx.send("Playback failed: decode thread panicked".to_string());
                let _ = output.stop();
                return;
            }

            // Keep output alive long enough to drain buffered samples.
            std::thread::sleep(std::time::Duration::from_millis(1500));

            if let Err(e) = output.stop() {
                let _ = status_tx.send(format!("Playback stop warning: {}", e));
            }

            let _ = status_tx.send("Playback finished".to_string());
        });
    }

    fn poll_playback_status(&mut self) {
        if let Some(rx) = &self.playback_status_receiver {
            while let Ok(status) = rx.try_recv() {
                self.play_status = Some(status);
            }
        }
    }

    fn request_cancel(&mut self) {
        self.last_scan_status = Some("Requested scan cancel".to_string());
        self.save_state();
        self.update_app_vm();

        let socket = ipc_mod::default_socket_path();
        let request = ipc_mod::ScanRequest {
            paths: Vec::new(),
            excludes: self.scan_excludes.clone(),
            cancel: true,
        };
        std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                let _ = rt.block_on(async move {
                    ipc_mod::send_scan_request(&socket, &request).await
                });
            }
        });
    }

    fn add_scan_target(&mut self) {
        let target = self.scan_target_input.trim().to_string();
        if !target.is_empty() {
            self.scan_targets.push(target);
            self.scan_target_input.clear();
            self.save_state();
            self.update_app_vm();
        }
    }

    fn remove_scan_target(&mut self, index: usize) {
        if index < self.scan_targets.len() {
            self.scan_targets.remove(index);
            self.save_state();
            self.update_app_vm();
        }
    }

    fn add_scan_exclude(&mut self) {
        let exclude = self.scan_exclude_input.trim().to_string();
        if !exclude.is_empty() {
            self.scan_excludes.push(exclude);
            self.scan_exclude_input.clear();
            self.save_state();
            self.update_app_vm();
        }
    }

    fn remove_scan_exclude(&mut self, index: usize) {
        if index < self.scan_excludes.len() {
            self.scan_excludes.remove(index);
            self.save_state();
            self.update_app_vm();
        }
    }

    fn setup_fonts(cc: &CreationContext<'_>) {
        let mut fonts = egui::FontDefinitions::default();
        let fallback_fonts = Self::discover_cjk_fallback_fonts();

        if !fallback_fonts.is_empty() {
            for (name, bytes) in fallback_fonts.iter() {
                fonts
                    .font_data
                    .insert(name.clone(), egui::FontData::from_owned(bytes.clone()));
            }

            let family = fonts.families.entry(egui::FontFamily::Proportional).or_default();
            for (name, _) in fallback_fonts.iter().rev() {
                if !family.contains(name) {
                    family.insert(0, name.clone());
                }
            }

            let monospace = fonts.families.entry(egui::FontFamily::Monospace).or_default();
            for (name, _) in fallback_fonts.iter().rev() {
                if !monospace.contains(name) {
                    monospace.insert(0, name.clone());
                }
            }
        }

        cc.egui_ctx.set_fonts(fonts);
    }

    fn discover_cjk_fallback_fonts() -> Vec<(String, Vec<u8>)> {
        let mut database = Database::new();
        database.load_system_fonts();

        let candidates = [
            "Noto Sans CJK JP",
            "Noto Sans CJK SC",
            "Noto Sans CJK TC",
            "Noto Sans CJK KR",
            "Source Han Sans JP",
            "Source Han Sans SC",
            "Source Han Sans TC",
            "Source Han Sans KR",
            "Noto Sans JP",
            "Noto Sans SC",
            "Noto Sans TC",
            "IPAPGothic",
            "WenQuanYi Zen Hei",
            "Microsoft YaHei",
            "Microsoft JhengHei",
            "PingFang SC",
            "Hiragino Sans",
        ];

        let mut loaded = Vec::new();
        let mut seen = HashSet::new();

        for family_name in candidates.iter() {
            let query = fontdb::Query {
                families: &[Family::Name(family_name)],
                weight: fontdb::Weight::NORMAL,
                stretch: fontdb::Stretch::Normal,
                style: Style::Normal,
            };

            if let Some(face_id) = database.query(&query) {
                if let Some(face) = database.face(face_id) {
                    let font_bytes = match &face.source {
                        Source::File(path) => fs::read(path).ok(),
                        Source::Binary(data) => Some(data.as_ref().as_ref().to_vec()),
                        _ => None,
                    };

                    if let Some(bytes) = font_bytes {
                        let key = family_name.replace(' ', "_");
                        if seen.insert(key.clone()) {
                            loaded.push((key, bytes));
                        }
                    }
                }
            }
        }

        loaded
    }

    pub fn new(cc: &CreationContext<'_>) -> Self {
        Self::setup_fonts(cc);
        let mut device_names = Vec::new();
        if let Ok(devs) = cpal::default_host().output_devices() {
            for d in devs {
                if let Ok(name) = d.name() {
                    device_names.push(name);
                }
            }
        }
        let cfg = Config::load();
        let selected = cfg
            .preferred_output_device
            .as_ref()
            .and_then(|n| device_names.iter().position(|s| s == n));
        let state = Self::load_state();
        let active_tab = state
            .last_tab
            .as_deref()
            .and_then(Tab::from_label)
            .unwrap_or(Tab::Library);

        let playlists = Self::load_playlists();
        let queue = QueueVm::new(state.queue_tracks.clone());
        let selected_queue_index = state.selected_queue_index.filter(|&i| i < queue.tracks.len());

        let mut gui = GuiApp {
            app_vm: Self::default_app_vm(
                active_tab,
                &cfg,
                state.last_scan_status.clone(),
                state.scan_targets.clone(),
                state.scan_excludes.clone(),
                LibraryDto {
                    album_count: 0,
                    track_count: 0,
                    recent_locations: Vec::new(),
                    recent_directories: Vec::new(),
                    directory_tree: Vec::new(),
                    favorite_directories: state.favorite_directories.clone(),
                    albums: Vec::new(),
                    selected_album: None,
                    selected_album_tracks: Vec::new(),
                    artists: Vec::new(),
                    selected_artist: None,
                    selected_artist_tracks: Vec::new(),
                    selected_directory: None,
                    selected_directory_tracks: Vec::new(),
                },
                &playlists,
                queue.clone(),
            ),
            active_tab,
            device_names,
            selected,
            device_filter: String::new(),
            play_path: state.last_play_path.clone().unwrap_or_default(),
            lyrics_files: Vec::new(),
            selected_lyrics_index: None,
            loaded_lyrics: None,
            play_position_ms: state.last_play_position_ms.unwrap_or(0),
            play_duration_ms: None,
            play_title: None,
            play_artist: None,
            play_album: None,
            play_artwork_handle: None,
            play_artwork_source: None,
            lyrics_offset_ms: 0,
            play_status: None,
            playback_status_receiver: None,
            scan_target_input: String::new(),
            scan_exclude_input: String::new(),
            scan_targets: state.scan_targets,
            scan_excludes: state.scan_excludes,
            selected_album: None,
            selected_artist: None,
            selected_directory: None,
            favorite_directories: state.favorite_directories,
            favorite_directory_input: String::new(),
            playlists,
            new_playlist_name: String::new(),
            selected_playlist: None,
            playlist_track_input: String::new(),
            queue,
            queue_item_input: String::new(),
            selected_queue_index,
            taps: cfg.taps_for_device(cfg.preferred_output_device.as_ref()),
            theme: cfg.theme,
            log_level: cfg.log_level,
            language: cfg.language,
            last_scan_status: state.last_scan_status,
        };

        gui.discover_lyrics_for_current_play_path();
        gui.refresh_play_metadata();
        gui.update_app_vm();
        gui
    }
}

impl App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.active_tab == Tab::Scan {
            self.refresh_scan_status();
        }
        self.poll_playback_status();
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.heading(translate("title", self.language));
            ui.horizontal(|ui| {
                for tab in [
                    Tab::Library,
                    Tab::Play,
                    Tab::Playlist,
                    Tab::Album,
                    Tab::Artist,
                    Tab::Queue,
                    Tab::Scan,
                    Tab::Settings,
                ] {
                    if ui
                        .selectable_label(self.active_tab == tab, tab.label())
                        .clicked()
                    {
                        self.active_tab = tab;
                        self.update_app_vm();
                    }
                }
            });
        });

        egui::SidePanel::left("side_panel").resizable(false).show(ctx, |ui| {
            ui.heading(translate("output_device", self.language));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label(translate("filter", self.language));
                ui.text_edit_singleline(&mut self.device_filter);
            });

            let filter = self.device_filter.to_lowercase();
            let filtered: Vec<(usize, &String)> = if filter.is_empty() {
                self.device_names.iter().enumerate().collect()
            } else {
                self.device_names
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| n.to_lowercase().contains(&filter))
                    .collect()
            };

            if filtered.is_empty() {
                ui.label(translate("no_matching_devices", self.language));
            } else {
                let default_sel = self.selected.unwrap_or(0);
                let mut sel_in_filtered = 0usize;
                for (i, (orig_idx, _)) in filtered.iter().enumerate() {
                    if *orig_idx == default_sel {
                        sel_in_filtered = i;
                        break;
                    }
                }

                egui::ComboBox::from_id_source("device_combo")
                    .selected_text(filtered.get(sel_in_filtered).map(|(_, s)| s.as_str()).unwrap_or(""))
                    .show_ui(ui, |ui| {
                        for (i, (_orig_idx, name)) in filtered.iter().enumerate() {
                            ui.selectable_value(&mut sel_in_filtered, i, name.as_str());
                        }
                    });

                if let Some((orig_idx, _)) = filtered.get(sel_in_filtered) {
                    let new_selected = Some(*orig_idx);
                    if self.selected != new_selected {
                        self.selected = new_selected;
                        if let Some(name) = self.device_names.get(*orig_idx) {
                            let cfg = Config::load();
                            self.taps = cfg.taps_for_device(Some(name));
                        }
                    }
                }
            }

            if ui.button(translate("save_device", self.language)).clicked() {
                if let Some(idx) = self.selected {
                    let name = self.device_names[idx].clone();
                    let mut cfg = Config::load();
                    cfg.preferred_output_device = Some(name.clone());
                    if let Some(settings) = cfg.device_settings.get(&name) {
                        self.taps = settings.resampler_taps;
                    }
                    let _ = cfg.save();
                    self.update_app_vm();
                }
            }

            ui.add_space(8.0);
            ui.label(translate("resampler_taps", self.language));
            ui.add(egui::DragValue::new(&mut self.taps).clamp_range(4..=256));
            if ui.button(translate("save_taps", self.language)).clicked() {
                let mut cfg = Config::load();
                if let Some(idx) = self.selected {
                    let name = self.device_names[idx].clone();
                    cfg.set_taps_for_device(name, self.taps);
                } else {
                    cfg.resampler_taps = self.taps;
                }
                let _ = cfg.save();
                self.update_app_vm();
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(translate("language", self.language));
                let mut selected_lang = self.language;
                egui::ComboBox::from_id_source("language_combo")
                    .selected_text(language_name(self.language))
                    .show_ui(ui, |ui| {
                        for lang in supported_languages() {
                            ui.selectable_value(&mut selected_lang, *lang, language_name(*lang));
                        }
                    });

                if selected_lang != self.language {
                    self.language = selected_lang;
                }
            });

            if ui.button("Save language").clicked() {
                let mut cfg = Config::load();
                cfg.language = self.language;
                let _ = cfg.save();
                self.update_app_vm();
            }

            ui.separator();
            ui.label(translate("play_demo", self.language));
            ui.text_edit_singleline(&mut self.play_path);
            if ui.button(translate("play", self.language)).clicked() {
                self.request_play();
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.active_tab {
                    Tab::Library => tabs::library_tab::ui(ui, self),
                Tab::Play => tabs::play_tab::ui(ui, self),
                Tab::Playlist => tabs::playlist_tab::ui(ui, self),
                Tab::Album => tabs::album_tab::ui(ui, self),
                Tab::Artist => tabs::artist_tab::ui(ui, self),
                Tab::Queue => tabs::queue_tab::ui(ui, self),
                Tab::Scan => tabs::scan_tab::ui(ui, &self.app_vm),
                Tab::Settings => tabs::settings_tab::ui(ui, self),
            }

            if self.active_tab == Tab::Scan {
                ui.separator();
                ui.heading("Scan Controls");
                ui.label("Configure scan targets and exclude paths, then press Request scan.");

                ui.group(|ui| {
                    ui.label("Scan target path:");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.scan_target_input);
                        if ui.button("Add target").clicked() {
                            self.add_scan_target();
                        }
                    });

                    if !self.scan_targets.is_empty() {
                        let targets = self.scan_targets.clone();
                        for (index, target) in targets.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(format!("• {}", target));
                                if ui.button("Remove").clicked() {
                                    self.remove_scan_target(index);
                                }
                            });
                        }
                    }
                });

                ui.group(|ui| {
                    ui.label("Exclude path:");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.scan_exclude_input);
                        if ui.button("Add exclude").clicked() {
                            self.add_scan_exclude();
                        }
                    });

                    if !self.scan_excludes.is_empty() {
                        let excludes = self.scan_excludes.clone();
                        for (index, exclude) in excludes.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(format!("• {}", exclude));
                                if ui.button("Remove").clicked() {
                                    self.remove_scan_exclude(index);
                                }
                            });
                        }
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Start scan").clicked() {
                        self.request_scan();
                    }
                    if ui.button("Cancel scan").clicked() {
                        self.request_cancel();
                    }
                });
            }

            if self.active_tab == Tab::Playlist {
                ui.separator();
                ui.heading("Playlist Controls");
                ui.label("Create and manage playlists.");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.new_playlist_name);
                    if ui.button("Create playlist").clicked() {
                        self.add_playlist();
                    }
                });

                if self.playlists.is_empty() {
                    ui.label("No playlists saved.");
                } else {
                    let playlists = self.playlists.clone();
                    for (index, playlist) in playlists.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(self.selected_playlist == Some(index),
                                    &playlist.name)
                                .clicked()
                            {
                                self.selected_playlist = Some(index);
                            }
                            if ui.button("Delete").clicked() {
                                self.remove_playlist(index);
                            }
                        });
                    }
                }
            }

            if self.active_tab == Tab::Queue {
                ui.separator();
                ui.heading("Queue Controls");
                ui.label("Add a track path to the queue and play selected items.");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.queue_item_input);
                    if ui.button("Add to queue").clicked() {
                        self.add_queue_item();
                    }
                });

                if self.queue.tracks.is_empty() {
                    ui.label("Queue is empty.");
                } else {
                    let tracks = self.queue.tracks.clone();
                    for (index, track) in tracks.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(self.selected_queue_index == Some(index),
                                    track)
                                .clicked()
                            {
                                self.selected_queue_index = Some(index);
                            }
                            if ui.button("Remove").clicked() {
                                self.remove_queue_item(index);
                            }
                        });
                    }
                }

                ui.horizontal(|ui| {
                    if ui.button("Play selected").clicked() {
                        self.play_selected_queue_item();
                    }
                    if ui.button("Remove selected").clicked() {
                        if let Some(index) = self.selected_queue_index {
                            self.remove_queue_item(index);
                        }
                    }
                });
            }

            if self.active_tab == Tab::Library {
                ui.separator();
                ui.heading("Favorite directories");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.favorite_directory_input);
                    if ui.button("Add favorite").clicked() {
                        self.add_favorite_directory();
                    }
                });

                if self.favorite_directories.is_empty() {
                    ui.label("No favorite directories configured.");
                } else {
                    let mut remove_index = None;
                    for (index, favorite) in self.favorite_directories.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("• {}", favorite));
                            if ui.button("Remove").clicked() {
                                remove_index = Some(index);
                            }
                        });
                    }
                    if let Some(index) = remove_index {
                        self.remove_favorite_directory(index);
                    }
                }
            }
        });
    }
}
