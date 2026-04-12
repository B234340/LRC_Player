//! Core::strings — 文字列のインターン（重複排除）ユーティリティ
//!
//! 再利用される文字列（アーティスト名、アルバム名、ファイル名など）を一元管理し、
//! メモリ使用量を抑えつつ高速な比較を可能にするための簡易インターンです。

use std::collections::HashMap;

use crate::core::ids::StrId;

/// 文字列を一意 ID (`StrId`) に変換して管理するインターンテーブル
#[derive(Debug, Default, Clone)]
pub struct StringInterner {
    /// 値 -> ID のマッピング（高速検索用）
    ids_by_value: HashMap<String, StrId>,
    /// ID に対応する文字列配列（ID の整数部分がインデックス）
    values: Vec<String>,
}

impl StringInterner {
    /// 文字列を登録し、対応する `StrId` を返す。既存であれば再利用する。
    pub fn intern<S: Into<String>>(&mut self, value: S) -> StrId {
        let value = value.into();
        if let Some(existing) = self.ids_by_value.get(&value) {
            return *existing;
        }
        // 新しい文字列は配列末尾に追加し、そのインデックスを StrId として返す
        let id = StrId(self.values.len() as u32);
        self.values.push(value.clone());
        self.ids_by_value.insert(value, id);
        id
    }

    /// `StrId` を解決して元の `String` を返す（存在しない場合は `None`）。
    pub fn resolve(&self, id: StrId) -> Option<String> {
        // StrId を元に文字列を取り出す。存在しない場合は None を返す。
        self.values.get(id.0 as usize).cloned()
    }

    /// 登録されている文字列の数
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// インターンが空かどうか
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}
