//! ライブラリのキャッシュ管理を担当
//! - アルバムアートワークのキャッシュ
//! - 遅延読み込みキュー処理

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui;
use image::io::Reader as ImageReader;
use lrc_player_application::infrastructure::persistence::MetaCache;

use crate::presentation::app::GuiApp;

impl GuiApp {
    pub(crate) fn clear_album_artwork_cache(&mut self) {
        self.album_artwork_path_cache.clear();
        self.album_artwork_texture_cache.clear();
        self.album_artwork_pending.clear();
        self.album_artwork_queued.clear();
        self.album_artwork_album_titles.clear();
        self.album_meta_cache = None;
    }

    pub(crate) fn sync_album_artwork_cache_with_library(&mut self) {
        let current_titles: Vec<String> = self
            .app_vm
            .library
            .albums
            .iter()
            .map(|album| album.title.clone())
            .collect();

        if current_titles == self.album_artwork_album_titles {
            return;
        }

        let keep: HashSet<String> = current_titles.iter().cloned().collect();
        self.album_artwork_path_cache
            .retain(|title, _| keep.contains(title));
        self.album_artwork_texture_cache
            .retain(|title, _| keep.contains(title));
        self.album_artwork_queued.retain(|title| keep.contains(title));
        self.album_artwork_pending.retain(|title| keep.contains(title));
        self.album_artwork_album_titles = current_titles;
    }

    pub(crate) fn resolve_album_artwork_path(&mut self, album_title: &str) -> Option<PathBuf> {
        if self.album_meta_cache.is_none() {
            self.album_meta_cache = MetaCache::open_default().ok();
        }

        let cache = self.album_meta_cache.as_ref()?;
        let candidates = cache.album_location_paths(album_title, 32).ok()?;
        for location in candidates {
            if let Some(path) = self.discover_artwork_for_track(Path::new(&location)) {
                return Some(path);
            }
        }
        None
    }

    pub(crate) fn queue_album_artwork_for_visible(&mut self, visible_album_titles: &[String]) {
        for title in visible_album_titles {
            if self.album_artwork_texture_cache.contains_key(title) {
                continue;
            }
            if self.album_artwork_queued.insert(title.clone()) {
                self.album_artwork_pending.push_back(title.clone());
            }
        }
    }

    pub(crate) fn process_album_artwork_queue(&mut self, ctx: &egui::Context, budget: usize) {
        if budget == 0 {
            return;
        }

        for _ in 0..budget {
            let Some(title) = self.album_artwork_pending.pop_front() else {
                break;
            };
            self.album_artwork_queued.remove(&title);

            if self.album_artwork_texture_cache.contains_key(&title) {
                continue;
            }

            let artwork_path = if let Some(cached) = self.album_artwork_path_cache.get(&title).cloned()
            {
                cached
            } else {
                let resolved = self.resolve_album_artwork_path(&title);
                self.album_artwork_path_cache
                    .insert(title.clone(), resolved.clone());
                resolved
            };

            let Some(path) = artwork_path else {
                continue;
            };

            let image_result: Result<image::DynamicImage, image::ImageError> = (|| {
                let reader = ImageReader::open(&path).map_err(image::ImageError::IoError)?;
                let reader = reader
                    .with_guessed_format()
                    .map_err(image::ImageError::IoError)?;
                reader.decode()
            })();

            let image = match image_result {
                Ok(image) => Some(image),
                Err(_) => self.extract_embedded_artwork(&path),
            };

            if let Some(image) = image {
                let size = [image.width() as usize, image.height() as usize];
                let color_image =
                    egui::ColorImage::from_rgba_unmultiplied(size, &image.to_rgba8().into_vec());
                let texture_id = format!("album_artwork:{}", title);
                let texture =
                    ctx.load_texture(texture_id, color_image, egui::TextureOptions::default());
                self.album_artwork_texture_cache.insert(title, texture);
            }
        }

        if !self.album_artwork_pending.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    pub(crate) fn album_artwork_texture(&self, album_title: &str) -> Option<egui::TextureHandle> {
        self.album_artwork_texture_cache.get(album_title).cloned()
    }
}