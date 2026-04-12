//! Library タブ用の表示型
//!
//! `LibraryTab` はディレクトリツリーのルート要約と選択されたディレクトリのトラック行を提供します。
use crate::app::view_models::TrackRowVm;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryTab {
    /// ルートノードの数（UI の簡易統計表示向け）
    pub root_node_count: usize,
    /// 選択中ディレクトリの表示用トラック行
    pub visible_tracks: Vec<TrackRowVm>,
}
