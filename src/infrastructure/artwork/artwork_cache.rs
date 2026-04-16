use anyhow::{Context, Result};
use image::{io::Reader as ImageReader, DynamicImage};
use std::collections::{HashMap, VecDeque};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ArtworkImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub format: image::ImageFormat,
}

impl ArtworkImage {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let reader = ImageReader::open(path)
            .with_context(|| format!("opening artwork file {}", path.display()))?
            .with_guessed_format()
            .with_context(|| format!("guessing artwork image format for {}", path.display()))?;

        let format = reader
            .format()
            .context("unable to determine image format")?;
        let image = reader
            .decode()
            .with_context(|| format!("decoding artwork image {}", path.display()))?;
        Ok(Self::from_dynamic(image, format))
    }

    fn from_dynamic(image: DynamicImage, format: image::ImageFormat) -> Self {
        let rgba = image.to_rgba8();
        Self {
            width: rgba.width(),
            height: rgba.height(),
            pixels: rgba.into_vec(),
            format,
        }
    }
}

#[derive(Debug)]
pub struct ArtworkCache {
    capacity: usize,
    order: VecDeque<String>,
    entries: HashMap<String, ArtworkImage>,
}

impl ArtworkCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::new(),
            entries: HashMap::new(),
        }
    }

    pub fn get(&mut self, key: &str) -> Option<&ArtworkImage> {
        if let Some(image) = self.entries.get(key) {
            if let Some(pos) = self.order.iter().position(|k| k == key) {
                self.order.remove(pos);
                self.order.push_back(key.to_string());
            }
            Some(image)
        } else {
            None
        }
    }

    pub fn put(&mut self, key: String, image: ArtworkImage) {
        if self.entries.contains_key(&key) {
            if let Some(pos) = self.order.iter().position(|k| k == &key) {
                self.order.remove(pos);
            }
        }

        self.order.push_back(key.clone());
        self.entries.insert(key, image);

        while self.entries.len() > self.capacity {
            if let Some(old_key) = self.order.pop_front() {
                self.entries.remove(&old_key);
            }
        }
    }

    pub fn get_or_load<P: AsRef<Path>>(&mut self, key: String, path: P) -> Result<&ArtworkImage> {
        if self.entries.contains_key(&key) {
            return Ok(self.get(&key).unwrap());
        }

        let image = ArtworkImage::load_from_file(path)?;
        self.put(key.clone(), image);
        Ok(self.get(&key).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageBuffer;
    use tempfile::tempdir;

    #[test]
    fn artwork_cache_evicts_least_recently_used_entry() {
        let mut cache = ArtworkCache::new(2);
        let image_a = ArtworkImage {
            width: 1,
            height: 1,
            pixels: vec![0, 0, 0, 0],
            format: image::ImageFormat::Png,
        };
        let image_b = ArtworkImage {
            width: 1,
            height: 1,
            pixels: vec![1, 1, 1, 1],
            format: image::ImageFormat::Png,
        };
        let image_c = ArtworkImage {
            width: 1,
            height: 1,
            pixels: vec![2, 2, 2, 2],
            format: image::ImageFormat::Png,
        };

        cache.put("a".into(), image_a);
        cache.put("b".into(), image_b);
        assert!(cache.get("a").is_some());
        cache.put("c".into(), image_c);

        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_none());
        assert!(cache.get("c").is_some());
    }

    #[test]
    fn artwork_cache_loads_image_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cover.png");
        let image = ImageBuffer::from_fn(2, 2, |x, y| {
            image::Rgba([x as u8 * 10, y as u8 * 20, 0, 255])
        });
        image.save(&path).unwrap();

        let mut cache = ArtworkCache::new(1);
        let artwork = cache.get_or_load("cover".into(), &path).unwrap();

        assert_eq!(artwork.width, 2);
        assert_eq!(artwork.height, 2);
        assert_eq!(artwork.pixels.len(), 16);
    }
}
