//! Album タブ用の軽量型
//!
//! `AlbumTab` は UI が利用する表示専用データ構造で、アルバム一覧の要約と
//! 選択されたアルバムのトラック行を保持します。
use crate::app::view_models::TrackRowVm;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumTab {
    /// アルバム総数
    pub album_count: usize,
    /// 選択中アルバムの表示用トラック行
    /// Invariant: `visible_tracks` is ordered as they should appear in the UI and
    /// corresponds to the track list for the currently selected album. Indices into
    /// this vector are stable for the duration of a single UI frame.
    pub visible_tracks: Vec<TrackRowVm>,
}
