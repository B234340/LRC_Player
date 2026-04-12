//! Artist タブ用の軽量表示型
//!
//! `ArtistTab` は UI が利用する表示専用データ構造で、アーティスト一覧の要約と
//! 選択されたアーティストのトラック行を保持します。
use crate::app::view_models::TrackRowVm;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistTab {
    /// アーティスト総数
    pub artist_count: usize,
    /// 選択中アーティストの表示用トラック行
    pub visible_tracks: Vec<TrackRowVm>,
}
