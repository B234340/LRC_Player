use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::ids::{PlaylistId, TrackId, TrackKey};
use crate::core::library::Library;
use crate::core::playback::PlaybackModel;
use crate::core::playlists::{Playlist, PlaylistKind, PlaylistStore};
use crate::core::queue::Queue;
use crate::core::queue::RepeatMode;
use crate::core::scan::ScanModel;
use crate::core::strings::StringInterner;
use crate::core::artwork::ArtworkCache;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TabId {
    #[default]
    Queue,
    Play,
    Album,
    Artist,
    Playlist,
    Library,
    Scan,
    Settings,
}

#[derive(Debug, Clone)]
pub struct PersistencePaths {
    pub config_json: PathBuf,
    pub state_json: PathBuf,
    pub playlists_json: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceConfig {
    pub sample_rate: Option<u32>,
    pub frames_per_buffer: Option<u32>,
    pub target_buffer_ms: Option<u32>,
    pub exclusive_mode: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfigFile {
    pub accent_color: String,
    pub theme: String,
    pub seek_step_ms: u64,
    pub restore_position_policy: String,
    pub full_auto_scan: bool,
    pub fallback_artwork_path: Option<PathBuf>,
    pub play_empty_state: String,
    pub include_dirs: Vec<PathBuf>,
    pub exclude_dirs: Vec<PathBuf>,
    pub preferred_output_device: Option<String>,
    pub per_device_settings: HashMap<String, DeviceConfig>,
}

impl Default for AppConfigFile {
    fn default() -> Self {
        Self {
            accent_color: "#2C6E49".to_string(),
            theme: "system".to_string(),
            seek_step_ms: 10_000,
            restore_position_policy: "resume".to_string(),
            full_auto_scan: false,
            fallback_artwork_path: None,
            play_empty_state: "none".to_string(),
            include_dirs: Vec::new(),
            exclude_dirs: Vec::new(),
            preferred_output_device: None,
            per_device_settings: HashMap::new(),
        }
    }
}

impl AppConfigFile {
    pub fn from_scan_model(scan: &ScanModel) -> Self {
        let mut config = Self::default();
        config.include_dirs = scan.config.include_dirs.clone();
        config.exclude_dirs = scan.config.exclude_dirs.clone();
        config
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateFile {
    pub last_tab: TabId,
    pub queue: Vec<TrackKey>,
    pub current_index: Option<u32>,
    pub repeat_mode: RepeatMode,
    pub playback_position_ms: u64,
}

impl Default for AppStateFile {
    fn default() -> Self {
        Self {
            last_tab: TabId::Queue,
            queue: Vec::new(),
            current_index: None,
            repeat_mode: RepeatMode::RepeatQueue,
            playback_position_ms: 0,
        }
    }
}

impl AppStateFile {
    pub fn capture(
        last_tab: TabId,
        queue: &Queue,
        playback: &PlaybackModel,
        library: &Library,
    ) -> Self {
        let restored_queue = queue
            .entries
            .iter()
            .filter_map(|entry| library.track_key(entry.track_id).cloned())
            .collect();

        Self {
            last_tab,
            queue: restored_queue,
            current_index: queue.current_index,
            repeat_mode: queue.repeat_mode,
            playback_position_ms: playback.position_ms,
        }
    }

    pub fn resolve_queue_track_ids(&self, library: &Library) -> Vec<TrackId> {
        self.queue
            .iter()
            .filter_map(|track_key| library.resolve_track_key(track_key))
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaylistFile {
    pub id: u32,
    pub name: String,
    pub kind: String,
    pub tracks: Vec<TrackKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaylistsFile {
    pub playlists: Vec<PlaylistFile>,
}

impl PlaylistsFile {
    pub fn capture(playlists: &PlaylistStore, library: &Library, strings: &StringInterner) -> Self {
        let playlists = playlists
            .playlists
            .iter()
            .map(|playlist| PlaylistFile {
                id: playlist.id.0,
                name: strings
                    .resolve(playlist.name)
                    .unwrap_or_else(|| format!("playlist-{}", playlist.id.0)),
                kind: match playlist.kind {
                    PlaylistKind::SystemFavorite => "system_favorite".to_string(),
                    PlaylistKind::UserDefined => "user_defined".to_string(),
                },
                tracks: playlist
                    .tracks
                    .iter()
                    .filter_map(|track_id| library.track_key(*track_id).cloned())
                    .collect(),
            })
            .collect();

        Self { playlists }
    }

    pub fn restore_playlists(
        &self,
        library: &Library,
        strings: &mut StringInterner,
    ) -> (Vec<Playlist>, PlaylistId) {
        let mut favorite_playlist = None;
        let mut restored = Vec::new();

        for playlist_file in &self.playlists {
            let kind = match playlist_file.kind.as_str() {
                "system_favorite" => PlaylistKind::SystemFavorite,
                _ => PlaylistKind::UserDefined,
            };
            let playlist = Playlist {
                id: PlaylistId(playlist_file.id),
                name: strings.intern(playlist_file.name.clone()),
                tracks: playlist_file
                    .tracks
                    .iter()
                    .filter_map(|track_key| library.resolve_track_key(track_key))
                    .collect(),
                kind,
            };

            if kind == PlaylistKind::SystemFavorite {
                favorite_playlist = Some(playlist.id);
            }

            restored.push(playlist);
        }

        let favorite_playlist = favorite_playlist.unwrap_or_else(|| {
            let generated_id = restored
                .iter()
                .map(|playlist| playlist.id.0)
                .max()
                .map(|max_id| max_id + 1)
                .unwrap_or(0);
            let favorite = Playlist {
                id: PlaylistId(generated_id),
                name: strings.intern("favorite"),
                tracks: Vec::new(),
                kind: PlaylistKind::SystemFavorite,
            };
            let favorite_id = favorite.id;
            restored.insert(0, favorite);
            favorite_id
        });

        (restored, favorite_playlist)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlbumIndexEntry {
    pub title: String,
    pub album_artist: String,
    pub track_count: usize,
    pub tracks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtistIndexEntry {
    pub name: String,
    pub track_count: usize,
    pub tracks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtistIndexFile {
    pub track_artists: Vec<ArtistIndexEntry>,
    pub album_artists: Vec<ArtistIndexEntry>,
    pub composers: Vec<ArtistIndexEntry>,
}

#[derive(Debug, Clone)]
pub struct JsonStore {
    pub paths: PersistencePaths,
}

impl JsonStore {
    pub fn new(paths: PersistencePaths) -> Self {
        Self { paths }
    }

    pub fn base_dir(&self) -> std::path::PathBuf {
        // config_json is data/<app>/config/config.json -> parent().parent() => data/<app>
        self.paths
            .config_json
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")))
    }

    /// Compute the shared data root (the `data` directory) under which
    /// application-specific folders live. For repo-local layout this will
    /// return `.../data`, for platform layout it will return the platform
    /// data dir (parent of the app directory).
    pub fn data_root(&self) -> std::path::PathBuf {
        // Honor an explicit override first.
        if let Ok(dir) = std::env::var("LRC_PLAYER_DATA_DIR") {
            return std::path::PathBuf::from(dir);
        }

        // If the configured json paths live under a directory named `data`, prefer that
        // as the shared data root. Walk upwards from the configured config path to find
        // a `data` component.
        if let Some(mut p) = self.paths.config_json.parent() {
            loop {
                if p.file_name().map(|n| n == "data").unwrap_or(false) {
                    return p.to_path_buf();
                }
                if let Some(parent) = p.parent() {
                    p = parent;
                    continue;
                }
                break;
            }
        }

        // Prefer a repo-local `./data` directory when possible (create if necessary).
        if let Ok(cwd) = std::env::current_dir() {
            let local_base = cwd.join("data");
            if fs::create_dir_all(&local_base).is_ok() {
                return local_base;
            }
        }

        // Fall back to platform data dirs.
        dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .unwrap_or(std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")))
    }

    pub fn cache_dir(&self) -> std::path::PathBuf {
        self.data_root().join("cache")
    }

    pub fn logs_dir(&self) -> std::path::PathBuf {
        self.data_root().join("logs")
    }

    pub fn album_index_path(&self) -> std::path::PathBuf {
        self.data_root().join("index").join("albums.json")
    }

    pub fn artist_index_path(&self) -> std::path::PathBuf {
        self.data_root().join("index").join("artists.json")
    }

    pub fn save_album_index(&self, index: &Vec<AlbumIndexEntry>) -> io::Result<()> {
        let path = self.album_index_path();
        self.save_json(&path, index)
    }

    pub fn load_album_index(&self) -> io::Result<Vec<AlbumIndexEntry>> {
        let path = self.album_index_path();
        self.load_json(&path)
    }

    pub fn save_artist_index(&self, index: &ArtistIndexFile) -> io::Result<()> {
        let path = self.artist_index_path();
        self.save_json(&path, index)
    }

    pub fn load_artist_index(&self) -> io::Result<ArtistIndexFile> {
        let path = self.artist_index_path();
        self.load_json(&path)
    }

    pub fn save_artwork_cache(&self, cache: &ArtworkCache) -> io::Result<()> {
        let dir = self.cache_dir();
        fs::create_dir_all(&dir)?;
        cache.save_to_dir(dir)
    }

    pub fn load_artwork_cache(&self, capacity: usize) -> io::Result<ArtworkCache> {
        let dir = self.cache_dir();
        ArtworkCache::load_from_dir(dir, capacity)
    }

    pub fn load_config(&self) -> io::Result<AppConfigFile> {
        self.load_json(&self.paths.config_json)
    }

    pub fn save_config(&self, config: &AppConfigFile) -> io::Result<()> {
        self.save_json(&self.paths.config_json, config)
    }

    pub fn load_state(&self) -> io::Result<AppStateFile> {
        self.load_json(&self.paths.state_json)
    }

    pub fn save_state(&self, state: &AppStateFile) -> io::Result<()> {
        self.save_json(&self.paths.state_json, state)
    }

    pub fn load_playlists(&self) -> io::Result<PlaylistsFile> {
        self.load_json(&self.paths.playlists_json)
    }

    pub fn save_playlists(&self, playlists: &PlaylistsFile) -> io::Result<()> {
        self.save_json(&self.paths.playlists_json, playlists)
    }

    fn load_json<T>(&self, path: &Path) -> io::Result<T>
    where
        T: for<'de> Deserialize<'de> + Default,
    {
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
            Err(error) => Err(error),
        }
    }

    fn save_json<T>(&self, path: &Path, value: &T) -> io::Result<()>
    where
        T: Serialize,
    {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let tmp_path = path.with_extension("tmp");
        let contents = serde_json::to_vec_pretty(value)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::write(&tmp_path, contents)?;
        fs::rename(tmp_path, path)?;
        Ok(())
    }
}

impl PersistencePaths {
    pub fn for_app(app_name: &str) -> io::Result<Self> {
        // Prefer explicit override via environment variable `LRC_PLAYER_DATA_DIR`.
        if let Ok(dir) = std::env::var("LRC_PLAYER_DATA_DIR") {
            let base = PathBuf::from(dir);
            fs::create_dir_all(&base)?;
            return Ok(Self {
                config_json: base.join("config").join("config.json"),
                state_json: base.join("state").join("state.json"),
                playlists_json: base.join("playlists").join("playlists.json"),
            });
        }

        // Prefer a repo-local `./data/...` directory when possible.
        if let Ok(cwd) = std::env::current_dir() {
            let local_base = cwd.join("data");
            if fs::create_dir_all(&local_base).is_ok() {
                return Ok(Self {
                    config_json: local_base.join("config").join("config.json"),
                    state_json: local_base.join("state").join("state.json"),
                    playlists_json: local_base.join("playlists").join("playlists.json"),
                });
            }
        }

        // Fallback to the platform data directory (existing behavior).
        let data_root = dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .unwrap_or(std::env::current_dir()?);
        let app_root = data_root.join(app_name);
        fs::create_dir_all(&app_root)?;

        Ok(Self {
            config_json: app_root.join("config.json"),
            state_json: app_root.join("state.json"),
            playlists_json: app_root.join("playlists.json"),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        AppConfigFile,
        AppStateFile,
        JsonStore,
        PersistencePaths,
        PlaylistsFile,
        TabId,
        AlbumIndexEntry,
        ArtistIndexEntry,
        ArtistIndexFile,
    };
    use crate::core::ids::TrackId;
    use crate::core::library::{Library, TrackInput};
    use crate::core::playback::PlaybackModel;
    use crate::core::playlists::PlaylistStore;
    use crate::core::queue::Queue;
    use crate::core::scan::ScanModel;
    use crate::core::strings::StringInterner;

    fn temp_paths() -> PersistencePaths {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("lrc-player-test-{unique}"));
        PersistencePaths {
            config_json: root.join("config.json"),
            state_json: root.join("state.json"),
            playlists_json: root.join("playlists.json"),
        }
    }

    fn sample_library(strings: &mut StringInterner) -> Library {
        let mut library = Library::default();
        library.upsert_track(
            strings,
            TrackInput {
                title: "Alpha".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: "/music/album/01-alpha.flac".to_string(),
                file_size: 1,
                modified_unix_secs: 10,
                file_name: "01-alpha.flac".to_string(),
                track_number: None,
                duration_ms: 1_000,
                artwork_key: None,
            },
        );
        library.upsert_track(
            strings,
            TrackInput {
                title: "Beta".to_string(),
                album: Some("Album".to_string()),
                artist: Some("Artist".to_string()),
                album_artist: Some("Artist".to_string()),
                composer: None,
                normalized_path: "/music/album/02-beta.flac".to_string(),
                file_size: 2,
                modified_unix_secs: 10,
                file_name: "02-beta.flac".to_string(),
                track_number: None,
                duration_ms: 2_000,
                artwork_key: None,
            },
        );
        library
    }

    #[test]
    fn state_and_playlists_round_trip_with_track_keys() {
        let mut strings = StringInterner::default();
        let library = sample_library(&mut strings);

        let mut queue = Queue::default();
        queue.replace_with_tracks(&[TrackId(0), TrackId(1)]);
        queue.set_current_index(1);

        let mut playback = PlaybackModel::default();
        playback.restore_track(TrackId(1), 4_321);

        let state = AppStateFile::capture(TabId::Play, &queue, &playback, &library);
        assert_eq!(
            state.resolve_queue_track_ids(&library),
            vec![TrackId(0), TrackId(1)]
        );

        let favorite_name = strings.intern("favorite");
        let playlist_name = strings.intern("Roadtrip");
        let mut playlists = PlaylistStore::new(favorite_name);
        let playlist_id = playlists
            .create_user_playlist(playlist_name)
            .expect("playlist creation should succeed");
        playlists.add_track(playlist_id, TrackId(1));
        playlists.toggle_favorite(TrackId(0));

        let file = PlaylistsFile::capture(&playlists, &library, &strings);
        let (restored, favorite_id) = file.restore_playlists(&library, &mut strings);

        assert_eq!(favorite_id.0, 0);
        assert_eq!(restored.len(), 2);
        assert!(
            restored
                .iter()
                .any(|playlist| playlist.tracks.contains(&TrackId(0)))
        );
        assert!(
            restored
                .iter()
                .any(|playlist| playlist.tracks.contains(&TrackId(1)))
        );
    }

    #[test]
    fn json_store_persists_split_files() {
        let paths = temp_paths();
        let store = JsonStore::new(paths.clone());
        let mut scan = ScanModel::default();
        scan.config.include_dirs.push("/music".into());
        scan.config.exclude_dirs.push("/music/tmp".into());

        store
            .save_config(&AppConfigFile::from_scan_model(&scan))
            .expect("config save should succeed");
        store
            .save_state(&AppStateFile::default())
            .expect("state save should succeed");
        store
            .save_playlists(&PlaylistsFile::default())
            .expect("playlist save should succeed");

        assert!(paths.config_json.exists());
        assert!(paths.state_json.exists());
        assert!(paths.playlists_json.exists());

        let loaded = store.load_config().expect("config load should succeed");
        assert_eq!(
            loaded.include_dirs,
            vec![std::path::PathBuf::from("/music")]
        );
        assert_eq!(
            loaded.exclude_dirs,
            vec![std::path::PathBuf::from("/music/tmp")]
        );

        let root = paths
            .config_json
            .parent()
            .expect("config parent should exist")
            .to_path_buf();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn persistence_paths_for_app_use_app_data_directory() {
        let paths = PersistencePaths::for_app("lrc_player_test").expect("paths should resolve");

        assert!(paths.config_json.ends_with("data/config/config.json"));
        assert!(paths.state_json.ends_with("data/state/state.json"));
        assert!(paths.playlists_json.ends_with("data/playlists/playlists.json"));
    }

    #[test]
    fn album_and_artist_index_round_trip() {
        let paths = temp_paths();

        // create store paths under a `data/` subdirectory of our temp root so
        // JsonStore::data_root() will pick that up without touching global env
        let base = paths
            .config_json
            .parent()
            .expect("config parent should exist")
            .to_path_buf();
        let store_paths = PersistencePaths {
            config_json: base.join("data").join("config").join("config.json"),
            state_json: base.join("data").join("state").join("state.json"),
            playlists_json: base.join("data").join("playlists").join("playlists.json"),
        };

        let store = JsonStore::new(store_paths.clone());

        let album_entry = AlbumIndexEntry {
            title: "Album".to_string(),
            album_artist: "Artist".to_string(),
            track_count: 2,
            tracks: vec![
                "/music/artist/album/01.flac".to_string(),
                "/music/artist/album/02.flac".to_string(),
            ],
        };

        let artist_entry = ArtistIndexEntry {
            name: "Artist".to_string(),
            track_count: 2,
            tracks: vec![
                "/music/artist/album/01.flac".to_string(),
                "/music/artist/album/02.flac".to_string(),
            ],
        };

        let artist_file = ArtistIndexFile {
            track_artists: vec![artist_entry.clone()],
            album_artists: Vec::new(),
            composers: Vec::new(),
        };

        store
            .save_album_index(&vec![album_entry.clone()])
            .expect("save album index should succeed");
        store
            .save_artist_index(&artist_file)
            .expect("save artist index should succeed");

        let loaded_albums = store.load_album_index().expect("load album index should succeed");
        let loaded_artists = store.load_artist_index().expect("load artist index should succeed");

        assert_eq!(loaded_albums.len(), 1);
        assert_eq!(loaded_albums[0].title, album_entry.title);
        assert_eq!(loaded_albums[0].album_artist, album_entry.album_artist);
        assert_eq!(loaded_albums[0].tracks, album_entry.tracks);

        assert_eq!(loaded_artists.track_artists.len(), 1);
        assert_eq!(loaded_artists.track_artists[0].name, artist_entry.name);
        assert_eq!(loaded_artists.track_artists[0].tracks, artist_entry.tracks);

        let root = store_paths
            .config_json
            .parent()
            .expect("config parent should exist")
            .to_path_buf();
        let _ = fs::remove_dir_all(root);
    }
}
