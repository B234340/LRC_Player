use crate::infrastructure::persistence::MetaCache;
use eframe::egui;
use image::io::Reader as ImageReader;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct TrackDisplayInfo {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
}

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
    let cache_id = egui::Id::new("track_display_artwork_texture_cache");
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

pub fn resolve_track_display(
    track_path: &str,
    meta_cache: Option<&MetaCache>,
    metadata_cache: &mut HashMap<String, (String, String, String)>,
) -> TrackDisplayInfo {
    let fallback_title = Path::new(track_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(track_path)
        .to_string();

    let (title, artist, album) = metadata_cache
        .entry(track_path.to_string())
        .or_insert_with(|| {
            meta_cache
                .and_then(|cache| cache.track_metadata_by_location(track_path).ok().flatten())
                .map(|(title, artist, album)| {
                    let title = if title.trim().is_empty() {
                        fallback_title.clone()
                    } else {
                        title
                    };
                    let artist = if artist.trim().is_empty() {
                        "Unknown artist".to_string()
                    } else {
                        artist
                    };
                    let album = if album.trim().is_empty() {
                        "Unknown album".to_string()
                    } else {
                        album
                    };
                    (title, artist, album)
                })
                .unwrap_or_else(|| {
                    (
                        fallback_title.clone(),
                        "Unknown artist".to_string(),
                        "Unknown album".to_string(),
                    )
                })
        })
        .clone();

    TrackDisplayInfo {
        path: track_path.to_string(),
        title,
        artist,
        album,
    }
}

pub fn render_track_artwork(
    ui: &mut egui::Ui,
    track_path: &str,
    texture_id: &str,
    art_size: egui::Vec2,
    is_playing: bool,
) {
    let art_bg = if is_playing {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().faint_bg_color
    };

    if let Some(art_path) = discover_artwork_for_track(Path::new(track_path)) {
        if let Some(texture) = get_cached_artwork_texture(ui.ctx(), &art_path, texture_id) {
            ui.image(&texture, art_size);
            return;
        }
    }

    let (art_rect, _) = ui.allocate_exact_size(art_size, egui::Sense::hover());
    ui.painter().rect_filled(art_rect, 4.0, art_bg);
    ui.painter().text(
        art_rect.center(),
        egui::Align2::CENTER_CENTER,
        "ART",
        egui::TextStyle::Small.resolve(ui.style()),
        ui.visuals().text_color(),
    );
}
