//! Settings タブの表示用型
//!
//! `SettingItemVm` はキーと値のペアを表し、`SettingsTab` はそれらのリストを保持します。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingItemVm {
    /// 設定キー
    pub key: String,
    /// 表示用の値（文字列化済み）
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsTab {
    /// 表示する設定項目のリスト
    pub items: Vec<SettingItemVm>,
}
