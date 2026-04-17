use serde::{Deserialize, Deserializer, Serialize};

fn deserialize_last_scan_status<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)
        .map_err(serde::de::Error::custom)?;

    match value {
        None => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s)),
        Some(other) => {
            let s = other.to_string();
            if s.chars().all(|c| c.is_ascii_digit()) {
                Ok(None)
            } else {
                Ok(Some(s))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDto {
    pub last_tab: Option<String>,

    #[serde(default)]
    pub last_play_path: Option<String>,

    #[serde(default)]
    pub last_play_position_ms: Option<u64>,

    #[serde(default, deserialize_with = "deserialize_last_scan_status")]
    pub last_scan_status: Option<String>,

    #[serde(default)]
    pub scan_scanned: Option<u64>,
    #[serde(default)]
    pub scan_parsed: Option<u64>,
    #[serde(default)]
    pub scan_registered: Option<u64>,

    #[serde(default)]
    pub scan_targets: Vec<String>,
    #[serde(default)]
    pub scan_excludes: Vec<String>,
    #[serde(default)]
    pub queue_tracks: Vec<String>,
    #[serde(default)]
    pub selected_queue_index: Option<usize>,
    #[serde(default)]
    pub favorite_directories: Vec<String>,
}
