use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistDto {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tracks: Vec<String>,
}
