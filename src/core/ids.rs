//! core::ids — 識別子型 (ID types)
//!
//! このモジュールはアプリケーション全体で使用する ID の newtype を定義します。
//! 各型は内部に `u32` を保持する薄いラッパーであり、型安全性を高めて
//! 異なる ID を誤って混用することを防ぎます。Serde の `Serialize`/`Deserialize`
//! を導出しているため、永続化や通信にそのまま利用できます。
//!
//! また、`PartialOrd`/`Ord` を導出している型は内部の整数値で順序付けされます。
//! `StrId` と `PathId` は `Default` を実装しており、デフォルト値は `0` です。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
/// `TrackId` — トラック（曲）を一意に識別する newtype。
/// 内部は `u32`（インデックスや DB のキー等に対応）。
pub struct TrackId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
/// `AlbumId` — アルバムを一意に識別する newtype。
/// TrackId と同様に誤用を防ぐための薄いラッパーです。
pub struct AlbumId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
/// `ArtistId` — アーティストを一意に識別する newtype。
pub struct ArtistId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
/// `PlaylistId` — プレイリストを一意に識別する newtype。
pub struct PlaylistId(pub u32);

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
/// `StrId` — 文字列テーブル等の汎用的な ID。`Default` を実装しており
/// デフォルトは `StrId(0)` になります。
pub struct StrId(pub u32);

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
/// `PathId` — パス（ファイル／ディレクトリ）を識別する ID。`Default` を実装。
pub struct PathId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
/// `QueueEntryId` — 再生キュー内のエントリを識別する newtype。
pub struct QueueEntryId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// `TrackKey` — ファイルシステム上のトラックを照合・一意化するためのキー。
///
/// 通常はノーマライズされたパスとファイルメタデータ（サイズ・更新時刻）の組
/// 合せで、一意なトラックを特定します。比較やハッシュ用に使います。
pub struct TrackKey {
    /// 比較用に正規化されたパス（例: 区切り文字や大文字小文字を統一した文字列）
    pub normalized_path: String,
    /// ファイルサイズ（バイト）
    pub file_size: u64,
    /// 最終更新時刻（Unix 時間、秒）
    pub modified_unix_secs: u64,
}
