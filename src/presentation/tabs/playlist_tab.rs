use crate::presentation::egui_app::GuiApp;
use eframe::egui;
use image::io::Reader as ImageReader;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn discover_artwork_for_track(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let stem = path.file_stem()?.to_string_lossy();
    let candidates = [
        format!("{}.jpg", stem),
        format!("{}.jpeg", stem),
        format!("{}.png", stem),
        format!("{}.webp", stem),
        "folder.jpg".to_string(),
        "cover.jpg".to_string(),
        "front.jpg".to_string(),
        "folder.png".to_string(),
        "cover.png".to_string(),
        "front.png".to_string(),
    ];
    for candidate in candidates.iter() {
        let candidate_path = parent.join(candidate);
        if candidate_path.exists() {
            return Some(candidate_path);
        }
    }
    None
}

fn load_artwork_texture(ctx: &egui::Context, artwork_path: &Path, id: &str) -> Option<egui::TextureHandle> {
    if let Ok(reader) = ImageReader::open(artwork_path) {
        if let Ok(reader) = reader.with_guessed_format() {
            if let Ok(image) = reader.decode() {
                let size = [image.width() as usize, image.height() as usize];
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    size,
                    &image.to_rgba8().into_vec(),
                );
                return Some(ctx.load_texture(id, color_image, egui::TextureOptions::default()));
            }
        }
    }
    None
}

fn get_cached_artwork_texture(
    ctx: &egui::Context,
    artwork_path: &Path,
    id: &str,
) -> Option<egui::TextureHandle> {
    let cache_id = egui::Id::new("playlist_artwork_texture_cache");
    let cache_key = artwork_path.to_string_lossy().to_string();

    let existing = ctx.data(|data| {
        data.get_temp::<HashMap<String, egui::TextureHandle>>(cache_id)
            .and_then(|cache| cache.get(&cache_key).cloned())
    });
    if existing.is_some() {
        return existing;
    }

    let texture = load_artwork_texture(ctx, artwork_path, id)?;
    ctx.data_mut(|data| {
        let cache = data.get_temp_mut_or_default::<HashMap<String, egui::TextureHandle>>(cache_id);
        cache.insert(cache_key, texture.clone());
    });
    Some(texture)
}

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    ui.heading("Playlists");
    ui.label(format!("Saved playlists: {}", app.playlists.len()));
    ui.separator();

    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.new_playlist_name);
        if ui.button("Create playlist").clicked() {
            app.add_playlist();
        }
    });

    if app.playlists.is_empty() {
        ui.label("No playlists available.");
        return;
    }

    ui.separator();
    let mut pending_select_playlist: Option<Option<usize>> = None;
    let mut pending_delete_playlist: Option<usize> = None;
    for (index, playlist) in app.playlists.iter().enumerate() {
        let selected = app.selected_playlist == Some(index);
        let is_favorite = playlist.name.eq_ignore_ascii_case("favorite");
        let mut frame = egui::Frame::group(ui.style());
        if selected {
            frame = frame
                .fill(ui.visuals().selection.bg_fill)
                .stroke(egui::Stroke::new(2.0, ui.visuals().selection.stroke.color));
        }

        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                let label = if is_favorite {
                    format!("★ {}", playlist.name)
                } else {
                    playlist.name.clone()
                };
                let response = ui.add(egui::SelectableLabel::new(selected, label));
                if response.clicked() {
                    if selected {
                        pending_select_playlist = Some(None);
                    } else {
                        pending_select_playlist = Some(Some(index));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!is_favorite, egui::Button::new("Delete")).clicked() {
                        pending_delete_playlist = Some(index);
                    }
                });
            });
        });
        ui.add_space(4.0);
    }

    if let Some(index) = pending_delete_playlist {
        app.remove_playlist(index);
        return;
    }

    if let Some(selection) = pending_select_playlist {
        app.selected_playlist = selection;
        app.update_app_vm();
    }

    if let Some(selected_index) = app.selected_playlist {
        let mut selected_name: Option<String> = None;
        let mut selected_len: usize = 0;
        let mut pending_remove_track: Option<usize> = None;
        let mut pending_play_index: Option<usize> = None;
        let mut pending_play_list: Vec<String> = Vec::new();
        let mut pending_play_now_path: Option<String> = None;
        let mut pending_next_path: Option<String> = None;
        let mut pending_enqueue_path: Option<String> = None;
        let mut pending_add_to_playlist: Option<(usize, String)> = None;
        let mut pending_show_details_path: Option<String> = None;
        let meta_cache = crate::infrastructure::persistence::MetaCache::open_default().ok();
        let playlist_targets: Vec<(usize, String)> = app
            .playlists
            .iter()
            .enumerate()
            .map(|(idx, p)| (idx, p.name.clone()))
            .collect();
        if let Some(playlist) = app.playlists.get(selected_index) {
            selected_name = Some(playlist.name.clone());
            selected_len = playlist.tracks.len();
            ui.separator();
            ui.heading(format!("Playlist: {}", playlist.name));
            ui.label(format!("{} tracks", playlist.tracks.len()));

            if playlist.tracks.is_empty() {
                ui.label("No tracks in this playlist.");
            } else {
                ui.label("Left click: create queue from this list and play. Right click: common actions.");
                for (track_index, track) in playlist.tracks.iter().enumerate() {
                    let path = std::path::Path::new(track);
                    let fallback_title = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(track);
                    let metadata = meta_cache
                        .as_ref()
                        .and_then(|cache| cache.track_metadata_by_location(track).ok().flatten());
                    let title = metadata
                        .as_ref()
                        .map(|(title, _, _)| title.as_str())
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or(fallback_title);
                    let artist = metadata
                        .as_ref()
                        .map(|(_, artist, _)| artist.as_str())
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or("Unknown artist");
                    let album = metadata
                        .as_ref()
                        .map(|(_, _, album)| album.as_str())
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or("Unknown album");
                    let is_playing = app.play_path == *track;

                    let mut frame = egui::Frame::group(ui.style());
                    if is_playing {
                        frame = frame
                            .fill(ui.visuals().selection.bg_fill)
                            .stroke(egui::Stroke::new(2.0, ui.visuals().selection.stroke.color));
                    }

                    frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let art_size = egui::vec2(44.0, 44.0);
                            if let Some(art_path) = discover_artwork_for_track(Path::new(track)) {
                                if let Some(texture) =
                                    get_cached_artwork_texture(ui.ctx(), &art_path, &format!("playlist_art_{}_{}", selected_index, track_index))
                                {
                                    ui.image(&texture, art_size);
                                } else {
                                    let (art_rect, _) = ui.allocate_exact_size(art_size, egui::Sense::hover());
                                    ui.painter()
                                        .rect_filled(art_rect, 4.0, ui.visuals().faint_bg_color);
                                    ui.painter().text(
                                        art_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "ART",
                                        egui::TextStyle::Small.resolve(ui.style()),
                                        ui.visuals().text_color(),
                                    );
                                }
                            } else {
                                let (art_rect, _) = ui.allocate_exact_size(art_size, egui::Sense::hover());
                                ui.painter()
                                    .rect_filled(art_rect, 4.0, ui.visuals().faint_bg_color);
                                ui.painter().text(
                                    art_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "ART",
                                    egui::TextStyle::Small.resolve(ui.style()),
                                    ui.visuals().text_color(),
                                );
                            }

                            ui.vertical(|ui| {
                                let title_text = if is_playing {
                                    format!("{}: {} [Playing]", track_index + 1, title)
                                } else {
                                    format!("{}: {}", track_index + 1, title)
                                };

                                let response = ui.selectable_label(false, title_text);
                                if response.clicked() {
                                    pending_play_index = Some(track_index);
                                    pending_play_list = playlist.tracks.clone();
                                }
                                response.context_menu(|ui| {
                                    if ui.button("Play now").clicked() {
                                        pending_play_now_path = Some(track.clone());
                                        ui.close_menu();
                                    }
                                    if ui.button("Play next").clicked() {
                                        pending_next_path = Some(track.clone());
                                        ui.close_menu();
                                    }
                                    if ui.button("Add to queue").clicked() {
                                        pending_enqueue_path = Some(track.clone());
                                        ui.close_menu();
                                    }
                                    ui.separator();
                                    ui.label("Add to playlist");
                                    for (target_index, target_name) in &playlist_targets {
                                        if ui.button(target_name).clicked() {
                                            pending_add_to_playlist = Some((*target_index, track.clone()));
                                            ui.close_menu();
                                        }
                                    }
                                    ui.separator();
                                    if ui.button("Show details").clicked() {
                                        pending_show_details_path = Some(track.clone());
                                        ui.close_menu();
                                    }
                                });

                                ui.label(format!("Artist: {}", artist));
                                ui.label(format!("Album: {}", album));
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("Remove").clicked() {
                                    pending_remove_track = Some(track_index);
                                }
                                if ui.button("Play").clicked() {
                                    pending_play_index = Some(track_index);
                                    pending_play_list = playlist.tracks.clone();
                                }
                            });
                        });
                    });
                    ui.add_space(4.0);
                }
            }
        }

        if selected_name.is_some() {
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut app.playlist_track_input);
                if ui.button("Add track").clicked() {
                    app.add_track_to_selected_playlist();
                }
                if selected_len == 0 {
                    ui.label("(empty)");
                }
            });
        }

        if let Some(track_index) = pending_remove_track {
            app.remove_track_from_selected_playlist(track_index);
        }
        if let Some(index) = pending_play_index {
            app.play_from_displayed_list(&pending_play_list, index);
        }
        if let Some(path) = pending_play_now_path {
            app.play_track_now(&path);
        }
        if let Some(path) = pending_next_path {
            app.add_track_next_in_queue(&path);
        }
        if let Some(path) = pending_enqueue_path {
            app.enqueue_track_path(&path);
        }
        if let Some((target_index, track)) = pending_add_to_playlist {
            app.add_track_to_playlist(target_index, &track);
        }
        if let Some(path) = pending_show_details_path {
            app.open_track_details(&path);
        }
    }
}
