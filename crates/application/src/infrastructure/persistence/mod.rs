pub mod config_file;pub mod json_store;
pub mod meta_cache;
pub mod playlists_file;
pub mod sqlite_store;
pub mod state_file;
use crate::dto::{PlaylistDto, StateDto};

pub use lrc_player_adapters_persistence::persistence::{JsonStore, MetaCache, SqliteStore};
pub type PlaylistsFile = lrc_player_adapters_persistence::persistence::PlaylistsFile<PlaylistDto>;
pub type StateFile = lrc_player_adapters_persistence::persistence::StateFile<StateDto>;
pub use config_file::ConfigFile;
