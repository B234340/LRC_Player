//! Playlists: loading / normalizing / saving `playlists.json` and the edit operations used by
//! the Playlists page, the sidebar (drop target) and the track context menu.
//!
//! The first playlist is always the built-in "favorite" list (shown as お気に入り /
//! Favorites); it cannot be renamed or deleted.

use std::time::{SystemTime, UNIX_EPOCH};

use lrc_player_application::dto::PlaylistDto;
use lrc_player_application::infrastructure::persistence::PlaylistsFile;

use crate::presentation::app::GuiApp;
use crate::presentation::ui::{views::apply_reorder, Tr};

pub(crate) const FAVORITE_NAME: &str = "favorite";

pub(crate) fn is_favorite(p: &PlaylistDto) -> bool {
    p.name.eq_ignore_ascii_case(FAVORITE_NAME)
}

/// Paths in the favorites list.
pub(crate) fn favorite_set(playlists: &[PlaylistDto]) -> std::collections::HashSet<String> {
    playlists.iter().filter(|p| is_favorite(p)).flat_map(|p| p.tracks.iter().cloned()).collect()
}

/// `Path -> is favorite` lookup for track tables.
pub(crate) fn favorite_lookup(set: &std::collections::HashSet<String>) -> impl Fn(&std::path::Path) -> bool + '_ {
    move |path| match path.to_str() {
        Some(s) => set.contains(s),
        None => set.contains(path.to_string_lossy().as_ref()),
    }
}

/// Name shown in the UI (the favorite list is localized).
pub(crate) fn display_name<'a>(p: &'a PlaylistDto, tr: &Tr) -> &'a str {
    if is_favorite(p) {
        tr.t("playlist.favorite")
    } else {
        &p.name
    }
}

/// Merges duplicate "favorite" lists (deduplicated) and puts the favorite list first.
pub(crate) fn normalize_playlists(playlists: Vec<PlaylistDto>) -> Vec<PlaylistDto> {
    let mut favorite_tracks: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut favorite_id: Option<String> = None;
    let mut others = Vec::new();
    for playlist in playlists {
        if is_favorite(&playlist) {
            favorite_id.get_or_insert_with(|| playlist.id.clone());
            for track in playlist.tracks {
                if seen.insert(track.clone()) {
                    favorite_tracks.push(track);
                }
            }
        } else {
            others.push(playlist);
        }
    }
    let mut normalized = Vec::with_capacity(others.len() + 1);
    normalized.push(PlaylistDto {
        id: favorite_id.unwrap_or_else(|| "playlist-favorite".to_string()),
        name: FAVORITE_NAME.to_string(),
        tracks: favorite_tracks,
    });
    normalized.extend(others);
    normalized
}

pub(crate) fn load_playlists() -> Vec<PlaylistDto> {
    let loaded = PlaylistsFile::open().and_then(|f| f.load()).unwrap_or_default();
    normalize_playlists(loaded)
}

fn new_id(existing: &[PlaylistDto]) -> String {
    let base = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut n = 0u32;
    loop {
        let id = if n == 0 { format!("playlist-{base}") } else { format!("playlist-{base}-{n}") };
        if !existing.iter().any(|p| p.id == id) {
            return id;
        }
        n += 1;
    }
}

/// Why a playlist name was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameError {
    Empty,
    Reserved,
    Duplicate,
}

impl NameError {
    pub(crate) fn key(self) -> &'static str {
        match self {
            NameError::Empty => "playlist.name_empty",
            NameError::Reserved => "playlist.name_reserved",
            NameError::Duplicate => "playlist.name_duplicate",
        }
    }
}

/// Validates a new name (`except` = id of the playlist being renamed).
pub(crate) fn validate_name(playlists: &[PlaylistDto], name: &str, except: Option<&str>) -> Result<String, NameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.eq_ignore_ascii_case(FAVORITE_NAME) {
        return Err(NameError::Reserved);
    }
    if playlists
        .iter()
        .any(|p| Some(p.id.as_str()) != except && p.name.eq_ignore_ascii_case(name))
    {
        return Err(NameError::Duplicate);
    }
    Ok(name.to_string())
}

impl GuiApp {
    pub(crate) fn save_playlists(&mut self) {
        self.favorites = favorite_set(&self.playlists);
        if !self.persist {
            return;
        }
        let result = PlaylistsFile::open().and_then(|f| f.save(&self.playlists));
        if let Err(err) = result {
            tracing::warn!("saving playlists failed: {err:#}");
            let tr = self.tr();
            self.toast_error(tr.f("toast.playlists_save_failed", &[&format!("{err:#}")]));
        }
    }

    pub(crate) fn playlist_index(&self, id: &str) -> Option<usize> {
        self.playlists.iter().position(|p| p.id == id)
    }

    /// Creates a playlist; returns its id.
    pub(crate) fn create_playlist(&mut self, name: &str, tracks: Vec<String>) -> Result<String, NameError> {
        let name = validate_name(&self.playlists, name, None)?;
        let id = new_id(&self.playlists);
        let count = tracks.len();
        self.playlists.push(PlaylistDto { id: id.clone(), name: name.clone(), tracks });
        self.save_playlists();
        let tr = self.tr();
        let msg = if count > 0 {
            tr.f("toast.playlist_created_n", &[&name, &count])
        } else {
            tr.f("toast.playlist_created", &[&name])
        };
        self.toast_success(msg);
        Ok(id)
    }

    pub(crate) fn rename_playlist(&mut self, id: &str, name: &str) -> Result<(), NameError> {
        let Some(i) = self.playlist_index(id) else { return Ok(()) };
        if is_favorite(&self.playlists[i]) {
            return Err(NameError::Reserved);
        }
        let name = validate_name(&self.playlists, name, Some(id))?;
        self.playlists[i].name = name;
        self.save_playlists();
        Ok(())
    }

    pub(crate) fn delete_playlist(&mut self, id: &str) {
        let Some(i) = self.playlist_index(id) else { return };
        if is_favorite(&self.playlists[i]) {
            return;
        }
        let removed = self.playlists.remove(i);
        if self.ui.playlist_detail.as_deref() == Some(id) {
            self.ui.playlist_detail = None;
        }
        self.save_playlists();
        let tr = self.tr();
        self.toast_info(tr.f("toast.playlist_deleted", &[&removed.name]));
    }

    /// Appends tracks (skipping ones already in the list).
    pub(crate) fn add_to_playlist(&mut self, id: &str, paths: Vec<String>) {
        let Some(i) = self.playlist_index(id) else { return };
        let tr = self.tr();
        let list = &mut self.playlists[i];
        let before = list.tracks.len();
        for p in paths {
            if !list.tracks.contains(&p) {
                list.tracks.push(p);
            }
        }
        let added = list.tracks.len() - before;
        let name = display_name(list, &tr).to_string();
        self.save_playlists();
        if added > 0 {
            self.toast_success(tr.f("toast.added_to_playlist", &[&added, &name]));
        } else {
            self.toast_info(tr.f("toast.already_in_playlist", &[&name]));
        }
    }

    /// Removes rows (indices into the playlist's track list).
    pub(crate) fn remove_from_playlist(&mut self, id: &str, mut rows: Vec<usize>) {
        let Some(i) = self.playlist_index(id) else { return };
        rows.sort_unstable();
        rows.dedup();
        let list = &mut self.playlists[i];
        for r in rows.into_iter().rev() {
            if r < list.tracks.len() {
                list.tracks.remove(r);
            }
        }
        self.save_playlists();
    }

    pub(crate) fn reorder_playlist(&mut self, id: &str, from: usize, to: usize) {
        let Some(i) = self.playlist_index(id) else { return };
        apply_reorder(&mut self.playlists[i].tracks, from, to);
        self.save_playlists();
    }

    /// Whether `path` is in the favorites list.
    pub(crate) fn is_favorite_path(&self, path: &str) -> bool {
        self.favorites.contains(path)
    }

    /// Adds the tracks to (or removes them from) the favorites list. A single track toggles
    /// silently (the heart is the feedback); several tracks show a toast.
    pub(crate) fn set_favorite(&mut self, paths: Vec<String>, favorite: bool) {
        let Some(i) = self.playlists.iter().position(is_favorite) else { return };
        let list = &mut self.playlists[i].tracks;
        let mut changed = 0usize;
        if favorite {
            for p in &paths {
                if !list.contains(p) {
                    list.push(p.clone());
                    changed += 1;
                }
            }
        } else {
            let before = list.len();
            list.retain(|t| !paths.contains(t));
            changed = before - list.len();
        }
        if changed == 0 {
            return;
        }
        self.save_playlists();
        if paths.len() > 1 {
            let tr = self.tr();
            let key = if favorite { "toast.favorites_added" } else { "toast.favorites_removed" };
            self.toast_success(tr.f(key, &[&changed]));
        }
    }

    /// Toggles the favorite state of the playing track.
    pub(crate) fn toggle_favorite_current(&mut self) {
        if self.play_path.is_empty() {
            return;
        }
        let path = self.play_path.clone();
        let favorite = !self.is_favorite_path(&path);
        self.set_favorite(vec![path], favorite);
    }

    /// A unique default name for a new playlist ("プレイリスト 3").
    pub(crate) fn suggest_playlist_name(&self, base: &str) -> String {
        let mut n = self.playlists.len().max(1);
        loop {
            let name = format!("{base} {n}");
            if validate_name(&self.playlists, &name, None).is_ok() {
                return name;
            }
            n += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl(id: &str, name: &str, tracks: &[&str]) -> PlaylistDto {
        PlaylistDto { id: id.into(), name: name.into(), tracks: tracks.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn favorites_are_merged_and_first() {
        let out = normalize_playlists(vec![
            pl("a", "Road", &["1"]),
            pl("f1", "favorite", &["x", "y"]),
            pl("f2", "Favorite", &["y", "z"]),
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "f1");
        assert_eq!(out[0].tracks, vec!["x", "y", "z"]);
        assert_eq!(out[1].name, "Road");
        assert_eq!(normalize_playlists(Vec::new())[0].name, FAVORITE_NAME);
    }

    #[test]
    fn names_are_validated() {
        let lists = vec![pl("f", "favorite", &[]), pl("a", "Road", &[])];
        assert_eq!(validate_name(&lists, "  ", None), Err(NameError::Empty));
        assert_eq!(validate_name(&lists, "FAVORITE", None), Err(NameError::Reserved));
        assert_eq!(validate_name(&lists, "road", None), Err(NameError::Duplicate));
        assert_eq!(validate_name(&lists, "road", Some("a")), Ok("road".to_string()));
        assert_eq!(validate_name(&lists, " New ", None), Ok("New".to_string()));
    }
}
