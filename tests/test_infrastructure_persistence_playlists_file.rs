#[cfg(test)]
mod tests {
    use lrc_player::application::dto::PlaylistDto;
    use lrc_player::infrastructure::persistence::json_store::JsonStore;
    use anyhow::{Context, Result};
    use directories::ProjectDirs;
    use std::path::{Path, PathBuf};
    use lrc_player::infrastructure::persistence::playlists_file::*;
    use tempfile::tempdir;

    #[test]
    fn playlists_file_roundtrips_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("playlists.json");
        let file = PlaylistsFile::open_with_path(path).unwrap();

        let original = vec![PlaylistDto {
            id: "playlist-1".into(),
            name: "Favorites".into(),
            tracks: Vec::new(),
        }];
        file.save(&original).unwrap();

        let loaded = file.load().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, original[0].id);
        assert_eq!(loaded[0].name, original[0].name);
    }
}
