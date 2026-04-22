#[cfg(test)]
mod tests {
    use image::ImageBuffer;
    use lrc_player::infrastructure::artwork::artwork_cache::*;
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
