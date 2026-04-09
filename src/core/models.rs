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
    pub duration_ms: Option<u64>,
    pub location: TrackLocation,
    pub lyric_path: Option<String>,
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
    pub current_track: Option<TrackId>,
    pub repeat_mode: RepeatMode,
    pub queue_mode: QueueMode,
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            current_track: None,
            repeat_mode: RepeatMode::Off,
            queue_mode: QueueMode::Sequential,
        }
    }
}