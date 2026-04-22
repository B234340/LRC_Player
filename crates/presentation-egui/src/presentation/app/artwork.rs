//! アートワーク画像の管理を担当
//! - 画像ファイルの検索と読み込み
//! - 埋め込みアートワークの抽出
//! - テクスチャ生成（通常/ぼかし）
//! - クロップ処理

use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui;
use image::{self, GenericImageView, io::Reader as ImageReader};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardVisualKey};
use symphonia::default::get_probe;

use crate::presentation::app::GuiApp;

impl GuiApp {
    pub(crate) fn ensure_play_artwork(&mut self, ctx: &egui::Context) {
        let path = Path::new(&self.play_path);
        if path.as_os_str().is_empty() || !path.exists() {
            self.play_artwork_handle = None;
            self.play_artwork_source = None;
            self.play_artwork_blur_handle = None;
            self.play_artwork_blur_source = None;
            return;
        }

        let artwork_path = self.discover_artwork_for_track(path);
        let artwork_source = artwork_path
            .as_ref()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| path.to_path_buf());

        let need_regular_texture = self.play_artwork_source.as_ref() != Some(&artwork_source)
            || self.play_artwork_handle.is_none();
        let need_blur_texture = self.lrc_overlay_enabled
            && (self.play_artwork_blur_source.as_ref() != Some(&artwork_source)
                || self.play_artwork_blur_handle.is_none());

        if need_regular_texture || need_blur_texture {
            let artwork_image = if let Some(ref art_path) = artwork_path {
                self.load_image_from_path(art_path)
            } else {
                self.extract_embedded_artwork(path)
            };

            if artwork_image.is_none() {
                self.play_artwork_handle = None;
                self.play_artwork_source = None;
                self.play_artwork_blur_handle = None;
                self.play_artwork_blur_source = None;
                return;
            }

            let artwork_image = artwork_image.unwrap();

            if need_regular_texture {
                self.play_artwork_handle =
                    self.load_artwork_texture_from_image(ctx, artwork_image.clone(), "play_artwork");
                self.play_artwork_source =
                    self.play_artwork_handle.as_ref().map(|_| artwork_source.clone());
            }

            if self.lrc_overlay_enabled {
                if need_blur_texture {
                    if need_regular_texture {
                        ctx.request_repaint_after(Duration::from_millis(16));
                    } else {
                        self.play_artwork_blur_handle = self.load_blurred_artwork_texture_from_image(
                            ctx,
                            artwork_image,
                            "play_artwork_blur",
                        );
                        self.play_artwork_blur_source =
                            self.play_artwork_blur_handle.as_ref().map(|_| artwork_source.clone());
                    }
                }
            } else {
                self.play_artwork_blur_handle = None;
                self.play_artwork_blur_source = None;
            }
        }
    }

    pub(crate) fn load_image_from_path(&self, artwork_path: &Path) -> Option<image::DynamicImage> {
        let reader = ImageReader::open(artwork_path).ok()?;
        let reader = reader.with_guessed_format().ok()?;
        reader.decode().ok()
    }

    pub(crate) fn crop_image_to_square(&self, image: image::DynamicImage) -> image::DynamicImage {
        let (width, height) = image.dimensions();
        if width == height {
            return image;
        }

        let side: u32 = width.min(height);
        let x = (width - side) / 2;
        let y = (height - side) / 2;
        let buffer = image.to_rgba8();
        let cropped = image::imageops::crop_imm(&buffer, x, y, side, side).to_image();
        image::DynamicImage::ImageRgba8(cropped)
    }

    pub(crate) fn load_artwork_texture_from_image(
        &self,
        ctx: &egui::Context,
        image: image::DynamicImage,
        id: &str,
    ) -> Option<egui::TextureHandle> {
        let image = self.crop_image_to_square(image);
        let size = [image.width() as usize, image.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &image.to_rgba8().into_vec());
        Some(ctx.load_texture(id, color_image, egui::TextureOptions::default()))
    }

    pub(crate) fn load_blurred_artwork_texture_from_image(
        &self,
        ctx: &egui::Context,
        image: image::DynamicImage,
        id: &str,
    ) -> Option<egui::TextureHandle> {
        let image = self.crop_image_to_square(image);
        let image = image.thumbnail(512, 512);
        let blurred = image::imageops::blur(&image, 10.0);
        self.load_artwork_texture_from_image(ctx, image::DynamicImage::ImageRgba8(blurred), id)
    }

    pub(crate) fn extract_embedded_artwork(&self, path: &Path) -> Option<image::DynamicImage> {
        let src = File::open(path).ok()?;
        let mss = MediaSourceStream::new(Box::new(src), Default::default());
        let probe = get_probe()
            .format(
                &Default::default(),
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .ok()?;
        let mut format = probe.format;
        let metadata_source = format.metadata();
        let metadata = metadata_source.current()?;

        let artwork_visual = metadata
            .visuals()
            .iter()
            .find(|visual| visual.usage == Some(StandardVisualKey::FrontCover))
            .or_else(|| {
                metadata.visuals().iter().max_by_key(|visual| {
                    visual
                        .dimensions
                        .map(|dimensions| dimensions.width.saturating_mul(dimensions.height))
                        .unwrap_or(0)
                })
            })?;

        image::load_from_memory(&artwork_visual.data).ok()
    }

    pub(crate) fn discover_artwork_for_track(&self, path: &Path) -> Option<PathBuf> {
        let parent = path.parent()?;
        let stem = path.file_stem()?.to_string_lossy().to_string();

        let stem_patterns = [
            format!("{}.jpg", stem),
            format!("{}.jpeg", stem),
            format!("{}.png", stem),
            format!("{}.webp", stem),
        ];

        let preferred_patterns = [
            "cover.jpg",
            "cover.jpeg",
            "cover.png",
            "cover.webp",
            "folder.jpg",
            "folder.jpeg",
            "folder.png",
            "folder.webp",
            "front.jpg",
            "front.jpeg",
            "front.png",
            "front.webp",
        ];

        let mut best_match: Option<(u8, u64, PathBuf)> = None;

        let mut check_pattern = |name: &str, path: PathBuf| {
            let lower = name.to_lowercase();
            let priority = if preferred_patterns.contains(&lower.as_str()) {
                1
            } else if stem_patterns
                .iter()
                .any(|pattern| pattern.eq_ignore_ascii_case(&lower))
            {
                2
            } else {
                return;
            };

            if let Ok(metadata) = fs::metadata(&path) {
                let size = metadata.len();
                match &best_match {
                    Some((best_priority, _, _)) if *best_priority < priority => {}
                    Some((best_priority, best_size, _))
                        if *best_priority == priority && *best_size >= size => {}
                    _ => {
                        best_match = Some((priority, size, path));
                    }
                }
            }
        };

        if let Ok(entries) = fs::read_dir(parent) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    check_pattern(name, path.clone());
                }
            }
        }

        best_match.map(|(_, _, path)| path)
    }
}
