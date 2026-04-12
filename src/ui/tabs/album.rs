//! Album タブ用の軽量型
//!
//! `AlbumTab` は UI が利用する表示専用データ構造で、アルバム一覧の要約と
//! 選択されたアルバムのトラック行を保持します。
use crate::app::view_models::TrackRowVm;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumTab {
    /// アルバム総数
    pub album_count: usize,
    /// 選択中アルバムの表示用トラック行（UI 用スナップショット）
    /// - このベクタは UI 表示専用で、内部モデルとは独立している。
    /// - 不変条件: `visible_tracks` は UI に表示される順序で整列されており、
    ///   単一フレーム中はインデックスが安定している（クリックハンドラはこれを前提にする）。
    pub visible_tracks: Vec<TrackRowVm>,
}