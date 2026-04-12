//! Scan タブ用のラッパ型
//!
//! `ScanTab` はスキャン設定と進捗を表す `ScanTabVm` を単一フィールドとしてラップします。
use crate::app::view_models::ScanTabVm;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanTab {
    /// スキャン用 ViewModel
    pub view_model: ScanTabVm,
}
