//! Artist タブ用の軽量表示型
//!
//! `ArtistTab` は UI が利用する表示専用データ構造で、アーティスト一覧の要約と
//! 選択されたアーティストのトラック行を保持します。
use crate::app::view_models::TrackRowVm;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistTab {
    /// アーティスト総数
    pub artist_count: usize,
    /// 選択中アーティストの表示用トラック行（UI 用スナップショット）
    /// - このベクタは UI 表示専用で、内部モデルとは独立している。
    /// - 不変条件: `visible_tracks` は UI に表示される順序で整列されており、
    ///   単一フレーム中はインデックスが安定している（クリックハンドラはこれを前提にする）。
    pub visible_tracks: Vec<TrackRowVm>,
}