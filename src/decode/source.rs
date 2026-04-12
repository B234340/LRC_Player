//! デコード対象ソースを表す小さな型
//!
//! `DecodedSource` は `TrackId` とファイルパスを保持し、デコーダに渡されます。
use std::path::PathBuf;

use crate::core::ids::TrackId;
use crate::core::library::Library;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedSource {
    pub track_id: TrackId,
    pub path: PathBuf,
}

impl DecodedSource {
    /// ライブラリからトラックパスを取得して `DecodedSource` を構築するヘルパー
    pub fn from_library(track_id: TrackId, library: &Library) -> Option<Self> {
        let path = library.track_path(track_id)?;
        Some(Self {
            track_id,
            path: PathBuf::from(path),
        })
    }
}
