use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryDirectoryDto {
    pub path: String,
    pub children: Vec<LibraryDirectoryDto>,
}
