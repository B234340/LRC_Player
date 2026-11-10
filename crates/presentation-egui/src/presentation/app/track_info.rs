//! Per-track auxiliary data (duration / tags probe + lyrics discovery) loaded off the UI
//! thread.
//!
//! Previously every track change decoded the whole file on the UI thread to learn its
//! duration (`refresh_play_metadata`) and parsed each candidate `.lrc` inside a sort
//! comparator. Now one worker thread runs `probe_track_info` (headers only) and
//! `Lyrics::discover_ranked` (each file parsed once) for the latest requested track; stale
//! requests are dropped ("latest wins") and the result is posted back with a repaint.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use lrc_player_application::infrastructure::decode::{probe_track_info, TrackInfo};
use lrc_player_application::infrastructure::lyrics::{Lyrics, LyricsCandidate};

use crate::presentation::app::GuiApp;

pub(crate) struct TrackInfoResult {
    pub(crate) path: String,
    pub(crate) info: Option<TrackInfo>,
    pub(crate) candidates: Vec<LyricsCandidate>,
    /// Image file for the OS media overlay (cover file, or embedded art written to a temp file).
    pub(crate) cover: Option<PathBuf>,
}

/// Cover file next to the track (cover/folder/front.*, then `<stem>.*`).
pub(crate) fn find_cover_file(track: &Path) -> Option<PathBuf> {
    let parent = track.parent()?;
    let stem = track.file_stem()?.to_string_lossy().to_lowercase();
    let mut best: Option<(u8, u64, PathBuf)> = None;
    for entry in std::fs::read_dir(parent).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let Some((base, ext)) = name.rsplit_once('.') else { continue };
        if !matches!(ext, "jpg" | "jpeg" | "png" | "webp") {
            continue;
        }
        let priority = if matches!(base, "cover" | "folder" | "front") {
            1
        } else if base == stem {
            2
        } else {
            continue;
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if best.as_ref().is_none_or(|(bp, bs, _)| priority < *bp || (priority == *bp && size > *bs)) {
            best = Some((priority, size, entry.path()));
        }
    }
    best.map(|(_, _, p)| p)
}

/// Embedded artwork written once to the temp directory (for the media overlay).
fn embedded_cover_file(track: &Path) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("lrc_player_covers");
    let name = blake3::hash(track.to_string_lossy().as_bytes()).to_hex();
    let out = dir.join(format!("{}.png", &name[..24]));
    if out.is_file() {
        return Some(out);
    }
    let image = super::artwork_loader::extract_embedded(track)?;
    let image = image.thumbnail(600, 600);
    std::fs::create_dir_all(&dir).ok()?;
    image.save_with_format(&out, image::ImageFormat::Png).ok()?;
    Some(out)
}

struct Request {
    path: String,
    lrc_targets: Vec<String>,
}

#[derive(Default)]
struct Slot {
    request: Option<Request>,
    shutdown: bool,
}

pub(crate) struct TrackInfoLoader {
    slot: Arc<(Mutex<Slot>, Condvar)>,
    rx: Receiver<TrackInfoResult>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl TrackInfoLoader {
    pub(crate) fn new(ctx: egui::Context) -> Self {
        let slot: Arc<(Mutex<Slot>, Condvar)> = Arc::default();
        let (tx, rx) = crossbeam_channel::unbounded();
        let worker_slot = Arc::clone(&slot);
        let worker = std::thread::Builder::new()
            .name("track-info".into())
            .spawn(move || worker_loop(worker_slot, tx, ctx))
            .ok();
        Self { slot, rx, worker }
    }

    /// Replaces any not-yet-started request with this one.
    pub(crate) fn request(&self, path: String, lrc_targets: Vec<String>) {
        let (lock, cvar) = &*self.slot;
        if let Ok(mut slot) = lock.lock() {
            slot.request = Some(Request { path, lrc_targets });
            cvar.notify_one();
        }
    }

    pub(crate) fn try_recv(&self) -> Option<TrackInfoResult> {
        self.rx.try_recv().ok()
    }
}

impl Drop for TrackInfoLoader {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.slot;
        if let Ok(mut slot) = lock.lock() {
            slot.shutdown = true;
            slot.request = None;
            cvar.notify_one();
        }
        // Not joined: a long probe/decode must not delay exit; the worker stops at its next
        // queue check (or when its result channel is gone).
        drop(self.worker.take());
    }
}

fn worker_loop(slot: Arc<(Mutex<Slot>, Condvar)>, tx: Sender<TrackInfoResult>, ctx: egui::Context) {
    let (lock, cvar) = &*slot;
    loop {
        let request = {
            let Ok(mut guard) = lock.lock() else {
                return;
            };
            loop {
                if guard.shutdown {
                    return;
                }
                if let Some(req) = guard.request.take() {
                    break req;
                }
                guard = match cvar.wait(guard) {
                    Ok(g) => g,
                    Err(_) => return,
                };
            }
        };
        let info = probe_track_info(&request.path).ok();
        let candidates = Lyrics::discover_ranked(&request.path, &request.lrc_targets);
        let track = Path::new(&request.path);
        let cover = find_cover_file(track).or_else(|| {
            info.as_ref()
                .is_some_and(|i| i.has_embedded_artwork)
                .then(|| embedded_cover_file(track))
                .flatten()
        });
        if tx
            .send(TrackInfoResult {
                path: request.path,
                info,
                candidates,
                cover,
            })
            .is_err()
        {
            return;
        }
        ctx.request_repaint();
    }
}

impl GuiApp {
    /// Asks the worker for duration/tags and lyrics of the current track.
    pub(crate) fn request_track_info(&mut self) {
        if self.play_path.is_empty() {
            self.lyrics_candidates.clear();
            self.selected_lyrics_index = None;
            self.loaded_timeline = None;
            self.lyrics_revision += 1;
            return;
        }
        self.track_info
            .request(self.play_path.clone(), self.scan_lrc_targets.clone());
    }

    /// Applies finished probe/lyrics results for the current track.
    pub(crate) fn poll_track_info(&mut self) {
        while let Some(result) = self.track_info.try_recv() {
            if result.path != self.play_path {
                continue;
            }
            if let Some(info) = result.info {
                if self.play_duration_ms.is_none() {
                    self.play_duration_ms = info.duration_ms;
                }
                if info.title.is_some() {
                    self.play_title = info.title;
                }
                if info.artist.is_some() {
                    self.play_artist = info.artist;
                }
                if info.album.is_some() {
                    self.play_album = info.album;
                }
                if let Some(d) = self.play_duration_ms {
                    if self.playback_state == lrc_player_domain::model::PlaybackState::Stopped {
                        self.play_position_ms = self.play_position_ms.min(d);
                    }
                }
            }
            self.play_cover_path = result.cover;
            self.set_lyrics_candidates(result.candidates);
        }
    }
}
