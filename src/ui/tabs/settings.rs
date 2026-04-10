#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingItemVm {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsTab {
    pub items: Vec<SettingItemVm>,
}