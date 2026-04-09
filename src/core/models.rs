#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackId(pub String);

impl TrackId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackLocation {
    Relative(String),
    Absolute(String),
}

impl TrackLocation {
    pub fn as_path_str(&self) -> &str {
        match self {
            Self::Relative(path) | Self::Absolute(path) => path,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Track {
    pub id: TrackId,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub composer: Option<String>,
    pub duration_ms: Option<u64>,
    pub track_number: Option<u32>,
    pub file_name: String,
    pub location: TrackLocation,
    pub lyric_path: Option<String>,
    pub artwork: Option<Vec<u8>>,
}

impl Track {
    pub fn file_path(&self) -> &str {
        self.location.as_path_str()
    }
}

#[derive(Debug, Clone)]
pub struct Playlist {
    pub name: String,
    pub track_ids: Vec<TrackId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    Off,
    One,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMode {
    Sequential,
    Shuffle,
}

#[derive(Debug, Clone)]
pub struct QueueState {
    pub track_ids: Vec<TrackId>,
    pub current_index: Option<usize>,
    pub repeat_mode: RepeatMode,
    pub queue_mode: QueueMode,
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            track_ids: Vec::new(),
            current_index: None,
            repeat_mode: RepeatMode::Off,
            queue_mode: QueueMode::Sequential,
        }
    }
}

impl QueueState {
    pub fn current_track_id(&self) -> Option<&TrackId> {
        let index = self.current_index?;
        self.track_ids.get(index)
    }
}