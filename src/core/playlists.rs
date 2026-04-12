//! Core::playlists — プレイリスト管理
//!
//! ユーザー定義プレイリストやシステム側のお気に入りプレイリストを管理する簡易ストア。
//! UI 層や永続化層がこれを利用してプレイリストの CRUD を行います。

use serde::{Deserialize, Serialize};

use crate::core::ids::{PlaylistId, StrId, TrackId};

/// プレイリストの種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaylistKind {
    /// システムで用意されたお気に入りプレイリスト
    SystemFavorite,
    /// ユーザーが作成したプレイリスト
    UserDefined,
}

/// プレイリストのデータ本体
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    /// プレイリスト固有の識別子
    pub id: PlaylistId,
    /// 表示名（`StrId` を用いてローカライズ可能）
    pub name: StrId,
    /// プレイリスト内のトラック ID（順序は再生順に対応）
    pub tracks: Vec<TrackId>,
    /// プレイリストの種別（お気に入り／ユーザー定義）
    pub kind: PlaylistKind,
}

/// メモリ上のプレイリストストア（軽量な CRUD を提供）
#[derive(Debug, Clone)]
pub struct PlaylistStore {
    /// 登録されている全プレイリストの配列。順序は UI 表示順に使われることがある。
    pub playlists: Vec<Playlist>,
    /// 現在選択されているプレイリスト ID。選択なしの可能性あり。
    pub selected: Option<PlaylistId>,
    /// お気に入りプレイリストの ID（常に存在することを想定）
    pub favorite_playlist: PlaylistId,
    /// 次に割り当てるユーザープレイリスト用の単純なインクリメント ID
    next_playlist_id: u32,
}

impl PlaylistStore {
    /// 初期化。`favorite_name` でお気に入りプレイリストの名前を指定する。
    pub fn new(favorite_name: StrId) -> Self {
        Self {
            playlists: vec![Playlist {
                id: PlaylistId(0),
                name: favorite_name,
                tracks: Vec::new(),
                kind: PlaylistKind::SystemFavorite,
            }],
            selected: Some(PlaylistId(0)),
            favorite_playlist: PlaylistId(0),
            next_playlist_id: 1,
        }
    }

    /// ユーザープレイリストを作成する。名前の重複はエラー。
    /// 成功すると新しく作成した `PlaylistId` を返し、作成したプレイリストを選択状態にします。
    /// 名前が既に存在する場合は `Err("playlist name must be unique")` を返します。
    pub fn create_user_playlist(&mut self, name: StrId) -> Result<PlaylistId, &'static str> {
        if self.playlists.iter().any(|playlist| playlist.name == name) {
            return Err("playlist name must be unique");
        }

        let id = PlaylistId(self.next_playlist_id);
        self.next_playlist_id += 1;
        self.playlists.push(Playlist {
            id,
            name,
            tracks: Vec::new(),
            kind: PlaylistKind::UserDefined,
        });
        self.selected = Some(id);
        Ok(id)
    }

    /// 指定プレイリストにトラックを追加する
    /// 重複チェックは行わないため、同一トラックを複数回追加することが可能です。
    pub fn add_track(&mut self, playlist_id: PlaylistId, track_id: TrackId) {
        if let Some(playlist) = self
            .playlists
            .iter_mut()
            .find(|playlist| playlist.id == playlist_id)
        {
            playlist.tracks.push(track_id);
        }
    }

    /// お気に入りプレイリストにトグルで登録/解除する
    /// トラックが既にお気に入りに含まれていれば削除、含まれていなければ追加する。
    /// `favorite_playlist` が見つからない場合は何も行いません。
    pub fn toggle_favorite(&mut self, track_id: TrackId) {
        if let Some(playlist) = self
            .playlists
            .iter_mut()
            .find(|playlist| playlist.id == self.favorite_playlist)
        {
            // 既に存在すれば削除
            if let Some(index) = playlist
                .tracks
                .iter()
                .position(|existing| *existing == track_id)
            {
                playlist.tracks.remove(index);
            } else {
                // 存在しなければ末尾に追加
                playlist.tracks.push(track_id);
            }
        }
    }

    /// 指定トラックがお気に入りに含まれているか判定する
    pub fn contains_in_favorite(&self, track_id: TrackId) -> bool {
        self.playlists
            .iter()
            .find(|playlist| playlist.id == self.favorite_playlist)
            .map(|playlist| playlist.tracks.contains(&track_id))
            .unwrap_or(false)
    }

    /// 全体を置換する（ロード時に使用）
    /// 指定されたプレイリスト群で内部状態を置換する（通常はロード時に使用）。
    /// `next_playlist_id` は与えられたプレイリスト内の最大 ID の次の値に設定されます。空の場合は `0` になります。
    pub fn replace_all(
        &mut self,
        playlists: Vec<Playlist>,
        selected: Option<PlaylistId>,
        favorite_playlist: PlaylistId,
    ) {
        // 次回に割り当てる ID を、既存プレイリストの最大 ID + 1 に設定
        // プレイリストが空なら 0 を使用
        let next_playlist_id = playlists
            .iter()
            .map(|playlist| playlist.id.0)
            .max()
            .map(|max_id| max_id + 1)
            .unwrap_or(0);

        self.playlists = playlists;
        self.selected = selected;
        self.favorite_playlist = favorite_playlist;
        self.next_playlist_id = next_playlist_id;
    }
}
