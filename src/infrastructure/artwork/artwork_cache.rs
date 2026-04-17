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


