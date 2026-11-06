//! Background artwork loading for list rows, the album grid and the Now Playing page
//! (cover + blurred backdrop made with `make_backdrop` on the worker).
//!
//! Before: every visible row scanned its folder (`read_dir` + stats) on every frame, the
//! egui temp-data texture cache was deep-cloned on each lookup, album covers were decoded
//! at full resolution on the UI thread (and artless albums were retried every frame), and
//! the Play tab re-probed the audio file every frame when a track had no artwork.
//!
//! Now:
//! * one worker thread resolves cover files (directory listings cached per folder), probes
//!   embedded pictures, decodes and downscales; the UI thread only uploads finished RGBA
//!   images (`ctx.load_texture`) and requests a repaint when results arrive;
//! * results are cached per track / album, **including negative results** ("no artwork"),
//!   so nothing is retried until the library changes ([`ArtworkLoader::clear`]);
//! * textures are shared per source image (many tracks of an album share `cover.jpg`) and
//!   evicted least-recently-used beyond a budget;
//! * the worker serves the newest request first and drops stale backlog when the user
//!   scrolls quickly;
//! * the Now Playing page may ask for a sharper copy of the current cover
//!   ([`ArtworkLoader::play_artwork_hires`], 1536 / 2048 px) when the cover is shown larger
//!   than 1024 physical px; only one such texture exists and it is dropped shortly after
//!   the page stops asking (page left, window shrunk, track changed).

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use image::{imageops::FilterType, DynamicImage, GenericImageView};
use lrc_player_application::infrastructure::persistence::MetaCache;

use crate::presentation::ui::views::make_backdrop;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardVisualKey};
use symphonia::core::probe::Hint;
use symphonia::default::get_probe;

/// Max side of list-row thumbnails (rows are ~50-70 px; 2x for HiDPI).
const ROW_THUMB_SIDE: u32 = 128;
/// Max side of album-grid covers (cards are ~194 px).
const ALBUM_THUMB_SIDE: u32 = 256;
/// Max side of the Play-tab artwork.
const PLAY_ART_SIDE: u32 = 1024;
/// Intermediate and max side of the on-demand sharper Now Playing artwork (16 MiB RGBA).
const PLAY_ART_HIRES_MID: u32 = 1536;
const PLAY_ART_HIRES_MAX: u32 = 2048;
/// Frames the sharper artwork survives without being asked for (then it is freed).
const HIRES_GRACE_FRAMES: u64 = 30;
/// Texture budget for row/album thumbnails before LRU eviction.
const MAX_TEXTURES: usize = 384;
/// Backlog limit; older requests are dropped (they are re-requested when visible again).
const MAX_PENDING_JOBS: usize = 256;
/// Cached directory listings on the worker.
const MAX_DIR_CACHE: usize = 1024;

/// Lookup order for a track's artwork.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtworkMode {
    /// Cover file next to the track only.
    FileOnly,
    /// Cover file, then embedded picture.
    FileFirst,
    /// Embedded picture, then cover file.
    #[allow(dead_code)]
    EmbeddedFirst,
}

type TrackKey = (String, ArtworkMode);

enum Job {
    Track { key: TrackKey },
    Album { title: String },
    Play { track: String },
    PlayHires { track: String, side: u32 },
    ClearCaches,
}

enum Done {
    /// The source is already uploaded (worker-side bookkeeping); just link it.
    TrackLinked { key: TrackKey, source: String },
    TrackDecoded { key: TrackKey, source: String, image: egui::ColorImage },
    TrackMissing { key: TrackKey },
    TrackDropped { key: TrackKey },
    Album { title: String, image: Option<egui::ColorImage> },
    AlbumDropped { title: String },
    Play {
        track: String,
        regular: Option<egui::ColorImage>,
        blur: Option<egui::ColorImage>,
    },
    /// `image` is `None` when the source is not larger than the regular Play artwork.
    PlayHires { track: String, side: u32, image: Option<egui::ColorImage> },
    PlayHiresDropped { track: String, side: u32 },
}

enum TrackSlot {
    Pending,
    Missing,
    Source(String),
}

enum AlbumSlot {
    Pending,
    Missing,
    Ready(egui::TextureHandle, u64),
}

struct SourceTexture {
    texture: egui::TextureHandle,
    last_used: u64,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    shutdown: bool,
}

/// The sharper Now Playing artwork (see [`ArtworkLoader::play_artwork_hires`]).
struct HiresArt {
    track: String,
    /// Side requested last; `u32::MAX` once the source is known to be no larger.
    side: u32,
    pending: bool,
    texture: Option<egui::TextureHandle>,
    last_used: u64,
}

/// Side to request for a cover shown at `needed_px` physical pixels; `None` when the
/// regular 1024 px texture is sharp enough.
fn hires_side(needed_px: f32) -> Option<u32> {
    if needed_px.is_nan() || needed_px <= PLAY_ART_SIDE as f32 {
        None
    } else if needed_px <= PLAY_ART_HIRES_MID as f32 {
        Some(PLAY_ART_HIRES_MID)
    } else {
        Some(PLAY_ART_HIRES_MAX)
    }
}

/// Play-tab artwork result: (track, regular, blurred background).
pub(crate) type PlayArtwork = (String, Option<egui::TextureHandle>, Option<egui::TextureHandle>);

pub(crate) struct ArtworkLoader {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    rx: Receiver<Done>,
    /// Sources uploaded (or being decoded) — shared with the worker to skip re-decoding.
    known_sources: Arc<Mutex<HashSet<String>>>,
    tracks: HashMap<TrackKey, TrackSlot>,
    sources: HashMap<String, SourceTexture>,
    albums: HashMap<String, AlbumSlot>,
    play_result: Option<PlayArtwork>,
    hires: Option<HiresArt>,
    frame: u64,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ArtworkLoader {
    /// `db_path`: library database used to find album covers (default: the shared one).
    pub(crate) fn new(ctx: egui::Context, db_path: Option<PathBuf>) -> Self {
        let queue: Arc<(Mutex<Queue>, Condvar)> = Arc::default();
        let known_sources: Arc<Mutex<HashSet<String>>> = Arc::default();
        let (tx, rx) = crossbeam_channel::unbounded();
        let worker = {
            let queue = Arc::clone(&queue);
            let known = Arc::clone(&known_sources);
            std::thread::Builder::new()
                .name("artwork-loader".into())
                .spawn(move || Worker::new(queue, known, tx, ctx, db_path).run())
                .ok()
        };
        Self {
            queue,
            rx,
            known_sources,
            tracks: HashMap::new(),
            sources: HashMap::new(),
            albums: HashMap::new(),
            play_result: None,
            hires: None,
            frame: 0,
            worker,
        }
    }

    fn push(&self, job: Job) {
        let (lock, cvar) = &*self.queue;
        if let Ok(mut q) = lock.lock() {
            q.jobs.push_back(job);
            cvar.notify_one();
        }
    }

    /// Thumbnail for a list row; `None` while loading or when the track has no artwork.
    pub(crate) fn track_thumbnail(&mut self, track: &str, mode: ArtworkMode) -> Option<egui::TextureHandle> {
        let key = (track.to_string(), mode);
        match self.tracks.get(&key) {
            Some(TrackSlot::Pending) | Some(TrackSlot::Missing) => None,
            Some(TrackSlot::Source(source)) => match self.sources.get_mut(source) {
                Some(entry) => {
                    entry.last_used = self.frame;
                    Some(entry.texture.clone())
                }
                None => {
                    // Evicted: ask again.
                    self.tracks.insert(key.clone(), TrackSlot::Pending);
                    self.push(Job::Track { key });
                    None
                }
            },
            None => {
                self.tracks.insert(key.clone(), TrackSlot::Pending);
                self.push(Job::Track { key });
                None
            }
        }
    }

    /// Cover for an album card; `None` while loading or when the album has no artwork.
    pub(crate) fn album_cover(&mut self, title: &str) -> Option<egui::TextureHandle> {
        match self.albums.get_mut(title) {
            Some(AlbumSlot::Ready(texture, last_used)) => {
                *last_used = self.frame;
                Some(texture.clone())
            }
            Some(_) => None,
            None => {
                self.albums.insert(title.to_string(), AlbumSlot::Pending);
                self.push(Job::Album {
                    title: title.to_string(),
                });
                None
            }
        }
    }

    /// Requests the Play-tab artwork (full + blurred) for `track`.
    pub(crate) fn request_play_artwork(&mut self, track: &str) {
        self.play_result = None;
        self.hires = None;
        self.push(Job::Play {
            track: track.to_string(),
        });
    }

    /// Takes the finished Play-tab artwork if it belongs to `track`.
    pub(crate) fn take_play_artwork(&mut self, track: &str) -> Option<PlayArtwork> {
        match &self.play_result {
            Some((t, _, _)) if t == track => self.play_result.take(),
            _ => None,
        }
    }

    /// Sharper artwork for the Now Playing page when the cover is shown at `needed_px`
    /// physical pixels (> 1024). Call every frame while it is needed: it is decoded once
    /// (1536 or 2048 px, never upscaled) and freed [`HIRES_GRACE_FRAMES`] frames after the
    /// last call. `None` while loading, when not needed, or when the source is not larger
    /// than the regular texture (then use the regular one).
    pub(crate) fn play_artwork_hires(&mut self, track: &str, needed_px: f32) -> Option<egui::TextureHandle> {
        let side = hires_side(needed_px)?;
        let frame = self.frame;
        if let Some(h) = self.hires.as_mut().filter(|h| h.track == track) {
            h.last_used = frame;
            if h.side < side {
                // Needs more than requested so far: ask again, keep showing the old copy.
                h.side = side;
                h.pending = true;
                let job = Job::PlayHires { track: track.to_string(), side };
                let texture = h.texture.clone();
                self.push(job);
                return texture;
            }
            return h.texture.clone();
        }
        self.hires = Some(HiresArt { track: track.to_string(), side, pending: true, texture: None, last_used: frame });
        self.push(Job::PlayHires { track: track.to_string(), side });
        None
    }

    /// Size of the sharper Now Playing texture currently held (tests).
    #[cfg(test)]
    pub(crate) fn hires_texture_size(&self) -> Option<[usize; 2]> {
        self.hires.as_ref()?.texture.as_ref().map(|t| t.size())
    }

    /// Uploads finished images. Call once per frame on the UI thread.
    pub(crate) fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(done) = self.rx.try_recv() {
            match done {
                Done::TrackLinked { key, source } => {
                    if self.sources.contains_key(&source) {
                        self.tracks.insert(key, TrackSlot::Source(source));
                    } else {
                        // Raced with eviction; it is requested again when visible.
                        self.tracks.remove(&key);
                    }
                }
                Done::TrackDecoded { key, source, image } => {
                    let texture = ctx.load_texture(
                        format!("artwork:{source}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.sources.insert(
                        source.clone(),
                        SourceTexture {
                            texture,
                            last_used: self.frame,
                        },
                    );
                    self.tracks.insert(key, TrackSlot::Source(source));
                }
                Done::TrackMissing { key } => {
                    self.tracks.insert(key, TrackSlot::Missing);
                }
                Done::TrackDropped { key } => {
                    self.tracks.remove(&key);
                }
                Done::Album { title, image } => {
                    let slot = match image {
                        Some(image) => AlbumSlot::Ready(
                            ctx.load_texture(format!("album:{title}"), image, egui::TextureOptions::LINEAR),
                            self.frame,
                        ),
                        None => AlbumSlot::Missing,
                    };
                    self.albums.insert(title, slot);
                }
                Done::AlbumDropped { title } => {
                    self.albums.remove(&title);
                }
                Done::Play { track, regular, blur } => {
                    let regular = regular.map(|img| {
                        ctx.load_texture("play_artwork", img, egui::TextureOptions::LINEAR)
                    });
                    let blur = blur.map(|img| {
                        ctx.load_texture("play_artwork_blur", img, egui::TextureOptions::LINEAR)
                    });
                    self.play_result = Some((track, regular, blur));
                }
                Done::PlayHires { track, side, image } => {
                    let Some(h) = self.hires.as_mut().filter(|h| h.track == track && h.side == side) else {
                        continue; // stale (track changed or a larger copy was asked for)
                    };
                    h.pending = false;
                    match image {
                        Some(img) => {
                            if (img.size[0] as u32) < side {
                                // The source is smaller than asked: nothing more to gain.
                                h.side = u32::MAX;
                            }
                            h.texture = Some(ctx.load_texture("play_artwork_hires", img, egui::TextureOptions::LINEAR));
                        }
                        None => h.side = u32::MAX,
                    }
                }
                Done::PlayHiresDropped { track, side } => {
                    if self.hires.as_ref().is_some_and(|h| h.track == track && h.side == side && h.pending) {
                        self.hires = None; // asked again next frame
                    }
                }
            }
        }
    }

    /// Advances the LRU clock and evicts textures beyond the budget.
    pub(crate) fn end_frame(&mut self) {
        self.frame += 1;
        if self.hires.as_ref().is_some_and(|h| h.last_used + HIRES_GRACE_FRAMES < self.frame) {
            self.hires = None;
        }
        let ready_albums = self
            .albums
            .values()
            .filter(|s| matches!(s, AlbumSlot::Ready(..)))
            .count();
        let total = self.sources.len() + ready_albums;
        if total <= MAX_TEXTURES {
            return;
        }
        let mut excess = total - MAX_TEXTURES;
        // Oldest first; never evict what was drawn in the last two frames.
        let mut candidates: Vec<(u64, bool, String)> = self
            .sources
            .iter()
            .map(|(k, v)| (v.last_used, false, k.clone()))
            .chain(self.albums.iter().filter_map(|(k, v)| match v {
                AlbumSlot::Ready(_, used) => Some((*used, true, k.clone())),
                _ => None,
            }))
            .filter(|(used, _, _)| *used + 2 < self.frame)
            .collect();
        candidates.sort_unstable_by_key(|(used, _, _)| *used);
        let mut known = self.known_sources.lock().ok();
        for (_, is_album, key) in candidates {
            if excess == 0 {
                break;
            }
            if is_album {
                // Forget it entirely so it is reloaded when visible again.
                self.albums.remove(&key);
            } else {
                self.sources.remove(&key);
                if let Some(known) = known.as_mut() {
                    known.remove(&key);
                }
            }
            excess -= 1;
        }
    }

    /// Forgets everything (after a scan / library clear); the worker drops its folder cache.
    pub(crate) fn clear(&mut self) {
        self.hires = None;
        self.tracks.clear();
        self.sources.clear();
        self.albums.clear();
        if let Ok(mut known) = self.known_sources.lock() {
            known.clear();
        }
        let (lock, cvar) = &*self.queue;
        if let Ok(mut q) = lock.lock() {
            q.jobs.clear();
            q.jobs.push_back(Job::ClearCaches);
            cvar.notify_one();
        }
    }
}

impl Drop for ArtworkLoader {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.queue;
        if let Ok(mut q) = lock.lock() {
            q.shutdown = true;
            q.jobs.clear();
            cvar.notify_one();
        }
        // Not joined: a long probe/decode must not delay exit; the worker stops at its next
        // queue check (or when its result channel is gone).
        drop(self.worker.take());
    }
}

// ----------------------------------------------------------------------------------------
// Worker
// ----------------------------------------------------------------------------------------

struct DirEntryInfo {
    lower_name: String,
    path: PathBuf,
    size: u64,
}

struct Worker {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    known_sources: Arc<Mutex<HashSet<String>>>,
    tx: Sender<Done>,
    ctx: egui::Context,
    dirs: HashMap<PathBuf, Arc<Vec<DirEntryInfo>>>,
    dir_order: VecDeque<PathBuf>,
    db: Option<MetaCache>,
    db_path: Option<PathBuf>,
    db_failed: bool,
}

impl Worker {
    fn new(
        queue: Arc<(Mutex<Queue>, Condvar)>,
        known_sources: Arc<Mutex<HashSet<String>>>,
        tx: Sender<Done>,
        ctx: egui::Context,
        db_path: Option<PathBuf>,
    ) -> Self {
        Self {
            queue,
            known_sources,
            tx,
            ctx,
            dirs: HashMap::new(),
            dir_order: VecDeque::new(),
            db: None,
            db_path,
            db_failed: false,
        }
    }

    fn next_job(&self) -> Option<Job> {
        let (lock, cvar) = &*self.queue;
        let mut q = lock.lock().ok()?;
        loop {
            if q.shutdown {
                return None;
            }
            // Drop stale backlog (oldest first); the UI re-requests visible items.
            while q.jobs.len() > MAX_PENDING_JOBS {
                if let Some(job) = q.jobs.pop_front() {
                    let dropped = match job {
                        Job::Track { key } => Some(Done::TrackDropped { key }),
                        Job::Album { title } => Some(Done::AlbumDropped { title }),
                        Job::PlayHires { track, side } => Some(Done::PlayHiresDropped { track, side }),
                        Job::Play { .. } | Job::ClearCaches => None,
                    };
                    if let Some(d) = dropped {
                        let _ = self.tx.send(d);
                    }
                }
            }
            // Play-tab requests first, otherwise newest first (what the user looks at now).
            if let Some(pos) = q.jobs.iter().rposition(|j| matches!(j, Job::Play { .. } | Job::ClearCaches)) {
                return q.jobs.remove(pos);
            }
            if let Some(pos) = q.jobs.iter().rposition(|j| matches!(j, Job::PlayHires { .. })) {
                return q.jobs.remove(pos);
            }
            if let Some(job) = q.jobs.pop_back() {
                return Some(job);
            }
            q = cvar.wait(q).ok()?;
        }
    }

    fn run(mut self) {
        while let Some(job) = self.next_job() {
            let done = match job {
                Job::Track { key } => self.load_track(key),
                Job::Album { title } => Done::Album {
                    image: self.load_album(&title),
                    title,
                },
                Job::Play { track } => self.load_play(track),
                Job::PlayHires { track, side } => {
                    let image = self.load_play_hires(&track, side);
                    Done::PlayHires { track, side, image }
                }
                Job::ClearCaches => {
                    self.dirs.clear();
                    self.dir_order.clear();
                    continue;
                }
            };
            if self.tx.send(done).is_err() {
                return;
            }
            self.ctx.request_repaint();
        }
    }

    fn dir_listing(&mut self, dir: &Path) -> Arc<Vec<DirEntryInfo>> {
        if let Some(listing) = self.dirs.get(dir) {
            return Arc::clone(listing);
        }
        let mut entries = Vec::new();
        if let Ok(read) = fs::read_dir(dir) {
            for entry in read.flatten() {
                let Some(name) = entry.file_name().to_str().map(|s| s.to_lowercase()) else {
                    continue;
                };
                if !(name.ends_with(".jpg")
                    || name.ends_with(".jpeg")
                    || name.ends_with(".png")
                    || name.ends_with(".webp"))
                {
                    continue;
                }
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                entries.push(DirEntryInfo {
                    lower_name: name,
                    path: entry.path(),
                    size: meta.len(),
                });
            }
        }
        let listing = Arc::new(entries);
        if self.dirs.len() >= MAX_DIR_CACHE {
            if let Some(old) = self.dir_order.pop_front() {
                self.dirs.remove(&old);
            }
        }
        self.dirs.insert(dir.to_path_buf(), Arc::clone(&listing));
        self.dir_order.push_back(dir.to_path_buf());
        listing
    }

    /// Cover file next to `track`: cover/folder/front.* first, then `<track stem>.*`;
    /// the largest file wins within a priority.
    fn cover_file(&mut self, track: &Path) -> Option<PathBuf> {
        let parent = track.parent()?;
        let stem = track.file_stem()?.to_string_lossy().to_lowercase();
        let listing = self.dir_listing(parent);
        let mut best: Option<(u8, u64, &PathBuf)> = None;
        for entry in listing.iter() {
            let (base, _ext) = entry.lower_name.rsplit_once('.').unwrap_or((&entry.lower_name, ""));
            let priority = if matches!(base, "cover" | "folder" | "front") {
                1
            } else if base == stem {
                2
            } else {
                continue;
            };
            let better = match best {
                None => true,
                Some((bp, bs, _)) => priority < bp || (priority == bp && entry.size > bs),
            };
            if better {
                best = Some((priority, entry.size, &entry.path));
            }
        }
        best.map(|(_, _, p)| p.clone())
    }

    fn load_track(&mut self, key: TrackKey) -> Done {
        let track = PathBuf::from(&key.0);
        let file_source = |w: &mut Worker| {
            w.cover_file(&track)
                .map(|p| (p.to_string_lossy().into_owned(), Some(p)))
        };
        let embedded_source = || (format!("embedded:{}", key.0), None::<PathBuf>);
        let order: Vec<Option<(String, Option<PathBuf>)>> = match key.1 {
            ArtworkMode::FileOnly => vec![file_source(self)],
            ArtworkMode::FileFirst => vec![file_source(self), Some(embedded_source())],
            ArtworkMode::EmbeddedFirst => vec![Some(embedded_source()), file_source(self)],
        };
        for (source, file) in order.into_iter().flatten() {
            let already = self
                .known_sources
                .lock()
                .map(|mut known| !known.insert(source.clone()))
                .unwrap_or(false);
            if already {
                return Done::TrackLinked { key, source };
            }
            let image = match &file {
                Some(path) => decode_file(path),
                None => extract_embedded(&track),
            };
            match image.map(|img| to_color_image(square_thumbnail(img, ROW_THUMB_SIDE))) {
                Some(image) => return Done::TrackDecoded { key, source, image },
                None => {
                    if let Ok(mut known) = self.known_sources.lock() {
                        known.remove(&source);
                    }
                }
            }
        }
        Done::TrackMissing { key }
    }

    fn load_album(&mut self, title: &str) -> Option<egui::ColorImage> {
        if self.db.is_none() && !self.db_failed {
            // Own connection: never contends with the UI's shared handle.
            let opened = match &self.db_path {
                Some(path) => MetaCache::new(path),
                None => MetaCache::open_default(),
            };
            match opened {
                Ok(db) => self.db = Some(db),
                Err(err) => {
                    tracing::warn!("artwork worker cannot open the library database: {err:#}");
                    self.db_failed = true;
                }
            }
        }
        let candidates = self.db.as_ref()?.album_location_paths(title, 32).ok()?;
        for location in &candidates {
            if let Some(cover) = self.cover_file(Path::new(location)) {
                if let Some(img) = decode_file(&cover) {
                    return Some(to_color_image(square_thumbnail(img, ALBUM_THUMB_SIDE)));
                }
            }
        }
        let first = candidates.first()?;
        extract_embedded(Path::new(first)).map(|img| to_color_image(square_thumbnail(img, ALBUM_THUMB_SIDE)))
    }

    fn play_source(&mut self, track: &str) -> Option<DynamicImage> {
        let path = PathBuf::from(track);
        match self.cover_file(&path) {
            Some(cover) => decode_file(&cover).or_else(|| extract_embedded(&path)),
            None => extract_embedded(&path),
        }
    }

    /// Sharper Now Playing cover (≤ `side`), or `None` when the source is not larger than
    /// the regular [`PLAY_ART_SIDE`] copy. Re-decodes the source (worker thread only).
    fn load_play_hires(&mut self, track: &str, side: u32) -> Option<egui::ColorImage> {
        let image = self.play_source(track)?;
        let (w, h) = image.dimensions();
        if w.min(h) <= PLAY_ART_SIDE {
            return None;
        }
        Some(to_color_image(square_thumbnail(image, side)))
    }

    fn load_play(&mut self, track: String) -> Done {
        let image = self.play_source(&track);
        let Some(image) = image else {
            return Done::Play {
                track,
                regular: None,
                blur: None,
            };
        };
        let regular = to_color_image(square_thumbnail(image, PLAY_ART_SIDE));
        // Small, blurred, darkened backdrop for the Now Playing page (GPU upscales it).
        let backdrop = make_backdrop(&regular);
        Done::Play {
            track,
            regular: Some(regular),
            blur: Some(backdrop),
        }
    }
}

fn decode_file(path: &Path) -> Option<DynamicImage> {
    image::io::Reader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()
}

/// Front cover (or the largest picture) embedded in an audio file.
pub(crate) fn extract_embedded(path: &Path) -> Option<DynamicImage> {
    let src = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(src), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut probe = get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;
    let pick = |visuals: &[symphonia::core::meta::Visual]| -> Option<Vec<u8>> {
        visuals
            .iter()
            .find(|v| v.usage == Some(StandardVisualKey::FrontCover))
            .or_else(|| {
                visuals.iter().max_by_key(|v| {
                    v.dimensions
                        .map(|d| d.width.saturating_mul(d.height))
                        .unwrap_or(0)
                })
            })
            .map(|v| v.data.to_vec())
    };
    // ID3v2 (MP3) lives in the probe metadata; others in the format metadata.
    let mut data = probe
        .metadata
        .get()
        .and_then(|m| m.current().and_then(|rev| pick(rev.visuals())));
    if data.is_none() {
        let mut format = probe.format;
        let meta = format.metadata();
        data = meta.current().and_then(|rev| pick(rev.visuals()));
    }
    image::load_from_memory(&data?).ok()
}

/// Center-crops to a square and downscales to at most `max_side`.
fn square_thumbnail(image: DynamicImage, max_side: u32) -> DynamicImage {
    let (w, h) = image.dimensions();
    let side = w.min(h);
    let cropped = if w != h {
        image.crop_imm((w - side) / 2, (h - side) / 2, side, side)
    } else {
        image
    };
    if side > max_side {
        cropped.resize_exact(max_side, max_side, FilterType::Triangle)
    } else {
        cropped
    }
}

fn to_color_image(image: DynamicImage) -> egui::ColorImage {
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hires_side_policy() {
        assert_eq!(hires_side(0.0), None);
        assert_eq!(hires_side(f32::NAN), None);
        assert_eq!(hires_side(1024.0), None);
        assert_eq!(hires_side(1025.0), Some(1536));
        assert_eq!(hires_side(1536.0), Some(1536));
        assert_eq!(hires_side(1537.0), Some(2048));
        assert_eq!(hires_side(5000.0), Some(2048));
    }

    #[test]
    fn square_thumbnail_never_upscales() {
        let img = DynamicImage::new_rgba8(1400, 1200);
        assert_eq!(square_thumbnail(img.clone(), 2048).dimensions(), (1200, 1200));
        assert_eq!(square_thumbnail(img, 1024).dimensions(), (1024, 1024));
    }
}
