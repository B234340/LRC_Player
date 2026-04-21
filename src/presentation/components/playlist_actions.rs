use crate::application::dto::PlaylistDto;
use crate::infrastructure::persistence::PlaylistsFile;
use crate::presentation::egui_app::GuiApp;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

pub fn normalize_playlists(playlists: Vec<PlaylistDto>) -> Vec<PlaylistDto> {
    let mut favorite_tracks: Vec<String> = Vec::new();
    let mut seen_tracks: Vec<String> = Vec::new();
    let mut favorite_id: Option<String> = None;
    let mut others: Vec<PlaylistDto> = Vec::new();

    for playlist in playlists {
        if playlist.name.eq_ignore_ascii_case("favorite") {
            if favorite_id.is_none() {
                favorite_id = Some(playlist.id.clone());
            }
            for track in playlist.tracks {
                if seen_tracks.contains(&track) {
                    continue;
                }
                seen_tracks.push(track.clone());
                favorite_tracks.push(track);
            }
        } else {
            others.push(playlist);
        }
    }

    let favorite = PlaylistDto {
        id: favorite_id.unwrap_or_else(|| "playlist-favorite".to_string()),
        name: "favorite".to_string(),
        tracks: favorite_tracks,
    };

    let mut normalized = Vec::with_capacity(others.len() + 1);
    normalized.push(favorite);
    normalized.extend(others);
    normalized
}

pub fn save_playlists(app: &GuiApp) {
    if let Ok(file) = PlaylistsFile::open() {
        let _ = file.save(&app.playlists);
    }
}

pub fn add_playlist(app: &mut GuiApp) {
    let name = app.new_playlist_name.trim().to_string();
    if name.is_empty() {
        return;
    }
    if name.eq_ignore_ascii_case("favorite") {
        return;
    }
    if app
        .playlists
        .iter()
        .any(|playlist| playlist.name.eq_ignore_ascii_case(&name))
    {
        return;
    }

    let id = format!(
        "playlist-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    app.playlists.push(PlaylistDto {
        id,
        name,
        tracks: Vec::new(),
    });
    app.new_playlist_name.clear();
    save_playlists(app);
    app.update_app_vm();
}

pub fn remove_playlist(app: &mut GuiApp, index: usize) {
    if index < app.playlists.len()
        && !app.playlists[index].name.eq_ignore_ascii_case("favorite")
    {
        app.playlists.remove(index);
        if app.selected_playlist == Some(index) {
            app.selected_playlist = None;
        }
        save_playlists(app);
        app.update_app_vm();
    }
}

pub fn move_playlist_up(app: &mut GuiApp, index: usize) {
    if index <= 1 || index >= app.playlists.len() {
        return;
    }
    if app.playlists[index - 1].name.eq_ignore_ascii_case("favorite") {
        return;
    }

    app.playlists.swap(index - 1, index);
    if let Some(selected) = app.selected_playlist {
        if selected == index {
            app.selected_playlist = Some(index - 1);
        } else if selected == index - 1 {
            app.selected_playlist = Some(index);
        }
    }
    save_playlists(app);
    app.update_app_vm();
}

pub fn move_playlist_down(app: &mut GuiApp, index: usize) {
    if index == 0 || index + 1 >= app.playlists.len() {
        return;
    }
    if app.playlists[index].name.eq_ignore_ascii_case("favorite") {
        return;
    }

    app.playlists.swap(index, index + 1);
    if let Some(selected) = app.selected_playlist {
        if selected == index {
            app.selected_playlist = Some(index + 1);
        } else if selected == index + 1 {
            app.selected_playlist = Some(index);
        }
    }
    save_playlists(app);
    app.update_app_vm();
}
