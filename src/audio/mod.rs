//! Audio サブモジュールの再エクスポート
//!
//! `audio` モジュールは出力ストリーム周りやデバイス監視など、OS のオーディオ機能に
//! 依存する実装をまとめます。

pub mod device_watcher;
pub mod output;
