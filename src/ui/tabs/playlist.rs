//! Playlist タブ用の表示型
//!
//! `PlaylistTab` はプレイリスト一覧のメタ情報と、選択プレイリストのトラック行を提供します。
use crate::app::view_models::TrackRowVm;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistTab {
    /// プレイリスト総数
    pub playlist_count: usize,
    /// 選択プレイリストの表示用トラック行
    pub visible_tracks: Vec<TrackRowVm>,
}
